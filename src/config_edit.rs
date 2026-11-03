//! Configuration editing: IPv4 address/DHCP and DNS servers per adapter,
//! network category, and saved Wi-Fi profile management (forget, priority,
//! automatic/manual connection). Every change is previewed, confirmed,
//! re-validated against live state and records an undo baseline (except
//! forgetting a Wi-Fi profile, which cannot be undone and says so).
//!
//! Coexistence: DNS and IP changes are refused on adapters owned by tunnel or
//! proxy software; network category is allowed there with a warning.

use crate::{
    arrays, blocking, capture_baseline, coexistence, config, consume_confirmation, err, err_with, field_bool,
    field_i64, field_string, get_network_overview, issue_confirmation, network_insights::{notice, notice_texts},
    normalized_wifi_guid, require_admin, run_netsh, run_powershell_json, valid_index, wifi_auto_rules, wifi_windows,
    AppResult, AppState, PublicError,
};
use serde_json::{json, Value};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    thread,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State};
use windows_sys::Win32::NetworkManagement::IpHelper::SendARP;

/// IP and category changes revert automatically unless kept within this time.
pub const REVERT_SECONDS: u64 = 60;
static PENDING_REVERT: Mutex<Option<(u64, Value)>> = Mutex::new(None);
static NEXT_REVERT: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Fixed PowerShell templates (all values arrive through $InputData)
// ---------------------------------------------------------------------------

const IP_STATE_SCRIPT: &str = r#"$i=[int]$InputData.interfaceIndex
$ip=Get-NetIPInterface -InterfaceIndex $i -AddressFamily IPv4 -ErrorAction Stop
$a=@(Get-NetIPAddress -InterfaceIndex $i -AddressFamily IPv4 -ErrorAction SilentlyContinue|Where-Object{[string]$_.PrefixOrigin -ne 'WellKnown'}|ForEach-Object{[PSCustomObject]@{ipAddress=[string]$_.IPAddress;prefixLength=[int]$_.PrefixLength;origin=[string]$_.PrefixOrigin}})
$g=@(Get-NetRoute -InterfaceIndex $i -AddressFamily IPv4 -DestinationPrefix "$([IPAddress]::Any)/0" -ErrorAction SilentlyContinue|ForEach-Object{[string]$_.NextHop}|Select-Object -Unique)
$guid=[string](Get-NetAdapter -IncludeHidden -InterfaceIndex $i -ErrorAction Stop).InterfaceGuid
$ns=[string](Get-ItemProperty -LiteralPath ('HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces\'+$guid) -Name NameServer -ErrorAction SilentlyContinue).NameServer
$d=@(Get-DnsClientServerAddress -InterfaceIndex $i -AddressFamily IPv4 -ErrorAction SilentlyContinue|ForEach-Object{$_.ServerAddresses}|ForEach-Object{[string]$_})
[PSCustomObject]@{dhcp=([string]$ip.Dhcp -eq 'Enabled');addresses=$a;gateways=$g;dnsServers=$d;dnsStatic=($ns.Trim().Length -gt 0)}|ConvertTo-Json -Compress -Depth 4"#;

/// IPv4/DNS state plus network category for several adapters in one call
/// (network backups). Each adapter is read independently; a failure is
/// reported for that adapter only.
pub(crate) const IP_STATE_ALL_SCRIPT: &str = r#"$out=@()
foreach($i in @($InputData.indexes)){
  try {
    $i=[int]$i
    $ip=Get-NetIPInterface -InterfaceIndex $i -AddressFamily IPv4 -ErrorAction Stop
    $a=@(Get-NetIPAddress -InterfaceIndex $i -AddressFamily IPv4 -ErrorAction SilentlyContinue|Where-Object{[string]$_.PrefixOrigin -ne 'WellKnown'}|ForEach-Object{[PSCustomObject]@{ipAddress=[string]$_.IPAddress;prefixLength=[int]$_.PrefixLength;origin=[string]$_.PrefixOrigin}})
    $g=@(Get-NetRoute -InterfaceIndex $i -AddressFamily IPv4 -DestinationPrefix "$([IPAddress]::Any)/0" -ErrorAction SilentlyContinue|ForEach-Object{[string]$_.NextHop}|Select-Object -Unique)
    $guid=[string](Get-NetAdapter -IncludeHidden -InterfaceIndex $i -ErrorAction Stop).InterfaceGuid
    $ns=[string](Get-ItemProperty -LiteralPath ('HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces\'+$guid) -Name NameServer -ErrorAction SilentlyContinue).NameServer
    $d=@(Get-DnsClientServerAddress -InterfaceIndex $i -AddressFamily IPv4 -ErrorAction SilentlyContinue|ForEach-Object{$_.ServerAddresses}|ForEach-Object{[string]$_})
    $out+=[PSCustomObject]@{interfaceIndex=$i;state=[PSCustomObject]@{dhcp=([string]$ip.Dhcp -eq 'Enabled');addresses=$a;gateways=$g;dnsServers=$d;dnsStatic=($ns.Trim().Length -gt 0)}}
  } catch {
    $out+=[PSCustomObject]@{interfaceIndex=[int]$i;error=$_.Exception.Message}
  }
}
ConvertTo-Json -InputObject @($out) -Compress -Depth 6"#;

pub(crate) const APPLY_IP_SCRIPT: &str = r#"function Set-Target($i,$t){
  $v4=$t.ipv4
  if($v4){
    foreach($s in 'ActiveStore','PersistentStore'){Get-NetRoute -InterfaceIndex $i -AddressFamily IPv4 -DestinationPrefix "$([IPAddress]::Any)/0" -PolicyStore $s -ErrorAction SilentlyContinue|Where-Object{[string]$_.Protocol -eq 'NetMgmt'}|Remove-NetRoute -Confirm:$false -ErrorAction SilentlyContinue}
    if($v4.mode -eq 'static'){
      Set-NetIPInterface -InterfaceIndex $i -AddressFamily IPv4 -Dhcp Disabled -ErrorAction Stop
      Get-NetIPAddress -InterfaceIndex $i -AddressFamily IPv4 -ErrorAction SilentlyContinue|Where-Object{[string]$_.PrefixOrigin -ne 'WellKnown'}|Remove-NetIPAddress -Confirm:$false -ErrorAction SilentlyContinue
      $p=@{InterfaceIndex=$i;AddressFamily='IPv4';IPAddress=[string]$v4.address;PrefixLength=[int]$v4.prefixLength;ErrorAction='Stop'}
      if($v4.gateway){$p.DefaultGateway=[string]$v4.gateway}
      New-NetIPAddress @p|Out-Null
    } else {
      Get-NetIPAddress -InterfaceIndex $i -AddressFamily IPv4 -PrefixOrigin Manual -ErrorAction SilentlyContinue|Remove-NetIPAddress -Confirm:$false -ErrorAction SilentlyContinue
      Set-NetIPInterface -InterfaceIndex $i -AddressFamily IPv4 -Dhcp Enabled -ErrorAction Stop
    }
  }
  $dns=$t.dns
  if($dns){
    if($dns.mode -eq 'manual'){Set-DnsClientServerAddress -InterfaceIndex $i -ServerAddresses @($dns.servers|ForEach-Object{[string]$_}) -ErrorAction Stop}
    else{Set-DnsClientServerAddress -InterfaceIndex $i -ResetServerAddresses -ErrorAction Stop}
  }
}
$i=[int]$InputData.interfaceIndex
try { Set-Target $i $InputData.target }
catch {
  $failure=$_.Exception.Message
  if($InputData.rollback){
    try { Set-Target $i $InputData.rollback } catch { throw "IP settings failed: $failure. Restoring the previous settings also failed: $($_.Exception.Message). Use Undo last app change." }
    throw "IP settings failed: $failure. The previous settings were restored."
  }
  throw "IP settings failed: $failure"
}
[PSCustomObject]@{applied=$true}|ConvertTo-Json -Compress"#;

pub(crate) const CATEGORY_SCRIPT: &str = r#"Set-NetConnectionProfile -InterfaceIndex ([int]$InputData.interfaceIndex) -NetworkCategory ([string]$InputData.category) -ErrorAction Stop
[PSCustomObject]@{category=[string](Get-NetConnectionProfile -InterfaceIndex ([int]$InputData.interfaceIndex)).NetworkCategory}|ConvertTo-Json -Compress"#;

// ---------------------------------------------------------------------------
// Validation (pure, unit tested)
// ---------------------------------------------------------------------------

fn ipv4(value: &str) -> Option<Ipv4Addr> {
    crate::valid_ipv4(value.trim()).then(|| value.trim().parse().ok()).flatten()
}

/// A usable static IPv4 configuration: unicast host address, sane prefix and
/// a gateway on the same subnet.
pub fn validate_static_ipv4(address: &str, prefix: i64, gateway: Option<&str>) -> Result<(), String> {
    let ip = ipv4(address).ok_or("Enter a valid IPv4 address.")?;
    if !(1..=32).contains(&prefix) {
        return Err("Prefix length must be from 1 to 32.".into());
    }
    let [a, b, ..] = ip.octets();
    if a == 0 || a == 127 || a >= 224 || (a == 169 && b == 254) {
        return Err("Use a unicast address (not 0.x, loopback, link-local or multicast).".into());
    }
    let bits = u32::from(ip);
    let mask = if prefix == 32 { u32::MAX } else { !(u32::MAX >> prefix) };
    let network = bits & mask;
    let broadcast = network | !mask;
    if prefix <= 30 && (bits == network || bits == broadcast) {
        return Err("The address is the network or broadcast address of its subnet.".into());
    }
    if let Some(gateway) = gateway.map(str::trim).filter(|value| !value.is_empty()) {
        let gw = ipv4(gateway).ok_or("Enter a valid IPv4 gateway.")?;
        let gw_bits = u32::from(gw);
        if gw == ip {
            return Err("The gateway cannot be the adapter's own address.".into());
        }
        if prefix < 32 && (gw_bits & mask != network || (prefix <= 30 && (gw_bits == network || gw_bits == broadcast))) {
            return Err("The gateway must be a host on the same subnet.".into());
        }
    }
    Ok(())
}

/// Up to four distinct unicast DNS server literals (IPv4 or IPv6).
pub fn normalize_dns(servers: &Value) -> Result<Vec<String>, String> {
    let items = servers.as_array().ok_or("DNS servers must be a list.")?;
    let mut result: Vec<String> = vec![];
    for item in items {
        let text = item.as_str().map(str::trim).unwrap_or_default();
        if text.is_empty() {
            continue;
        }
        let ip: IpAddr = text.parse().map_err(|_| format!("\"{text}\" is not an IP address."))?;
        if ip.is_unspecified() || ip.is_multicast() {
            return Err(format!("{text} cannot be used as a DNS server."));
        }
        let text = ip.to_string();
        if !result.contains(&text) {
            result.push(text);
        }
    }
    if result.is_empty() {
        return Err("Enter at least one DNS server.".into());
    }
    if result.len() > 4 {
        return Err("Enter at most four DNS servers.".into());
    }
    Ok(result)
}

/// The current IPv4/DNS configuration in the same shape as a requested one.
pub fn current_target(current: &Value) -> Value {
    let ipv4 = if field_bool(current, "dhcp") {
        json!({"mode":"dhcp"})
    } else {
        let address = arrays(current.get("addresses"))
            .into_iter()
            .find(|item| field_string(item, "origin") == "Manual")
            .or_else(|| arrays(current.get("addresses")).into_iter().next());
        json!({
            "mode":"static",
            "address":address.as_ref().map(|item| field_string(item, "ipAddress")),
            "prefixLength":address.as_ref().map(|item| field_i64(item, "prefixLength")),
            "gateway":arrays(current.get("gateways")).first().and_then(Value::as_str),
        })
    };
    let dns = if field_bool(current, "dnsStatic") {
        json!({"mode":"manual","servers":arrays(current.get("dnsServers"))})
    } else {
        json!({"mode":"automatic"})
    };
    json!({"ipv4":ipv4,"dns":dns})
}

fn requested_ipv4(input: &Value) -> AppResult<Option<Value>> {
    let Some(v4) = input.get("ipv4").filter(|value| !value.is_null()) else { return Ok(None) };
    match v4.get("mode").and_then(Value::as_str) {
        Some("dhcp") => Ok(Some(json!({"mode":"dhcp"}))),
        Some("static") => {
            let address = field_string(v4, "address");
            let prefix = v4
                .get("prefixLength")
                .and_then(|value| value.as_i64().or_else(|| value.as_str().and_then(|text| text.trim().parse().ok())))
                .unwrap_or(0);
            let gateway = field_string(v4, "gateway");
            validate_static_ipv4(&address, prefix, Some(&gateway)).map_err(|message| err("VALIDATION_ERROR", message))?;
            Ok(Some(json!({
                "mode":"static",
                "address":address,
                "prefixLength":prefix,
                "gateway":if gateway.is_empty() { Value::Null } else { json!(gateway) },
            })))
        }
        _ => Err(err("VALIDATION_ERROR", "IPv4 mode must be dhcp or static.")),
    }
}

fn requested_dns(input: &Value) -> AppResult<Option<Value>> {
    let Some(dns) = input.get("dns").filter(|value| !value.is_null()) else { return Ok(None) };
    match dns.get("mode").and_then(Value::as_str) {
        Some("automatic") => Ok(Some(json!({"mode":"automatic"}))),
        Some("manual") => {
            let servers = normalize_dns(dns.get("servers").unwrap_or(&Value::Null))
                .map_err(|message| err("VALIDATION_ERROR", message))?;
            Ok(Some(json!({"mode":"manual","servers":servers})))
        }
        _ => Err(err("VALIDATION_ERROR", "DNS mode must be automatic or manual.")),
    }
}

/// Build the IP-settings payload from the request and live state. Parts equal
/// to the current configuration are dropped; `before` keeps the rollback.
pub fn ip_settings_payload(input: &Value, adapter: &Value, current: &Value) -> AppResult<Value> {
    let alias = field_string(adapter, "alias");
    if let Some(owner) = coexistence::owner_of(adapter) {
        return Err(err_with(
            "PROTECTED_ADAPTER",
            format!("{alias} is managed by {owner}. Change its addresses and DNS in {owner} instead."),
            json!({"alias":alias,"owner":owner}),
        ));
    }
    if field_bool(adapter, "systemInterface") {
        return Err(err("VALIDATION_ERROR", "System interfaces cannot be changed."));
    }
    let before = current_target(current);
    let ipv4 = requested_ipv4(input)?.filter(|value| value != &before["ipv4"]);
    let dns = requested_dns(input)?.filter(|value| value != &before["dns"]);
    let static_after = ipv4.as_ref().map(|value| value["mode"] == "static").unwrap_or(before["ipv4"]["mode"] == "static");
    let dns_after = dns.as_ref().unwrap_or(&before["dns"]);
    if static_after && dns_after["mode"] == "automatic" {
        return Err(err("VALIDATION_ERROR", "A manual IPv4 address needs manual DNS servers; DHCP cannot provide them."));
    }
    let rollback = json!({
        "ipv4":if ipv4.is_some() && before["ipv4"]["mode"] == "dhcp" || ipv4.is_some() && !before["ipv4"]["address"].is_null() { before["ipv4"].clone() } else { Value::Null },
        "dns":if dns.is_some() { before["dns"].clone() } else { Value::Null },
    });
    Ok(json!({
        "interfaceIndex":field_i64(adapter, "interfaceIndex"),
        "guid":field_string(adapter, "guid"),
        "alias":alias,
        "target":{"ipv4":ipv4,"dns":dns},
        "rollback":rollback,
        "before":before,
        "unchanged":ipv4.is_none() && dns.is_none(),
    }))
}

/// Extra warnings about a requested IPv4/DNS change (pure, unit tested).
/// `address_in_use` is the result of a local ARP probe for a new address.
pub fn ip_risk_notices(adapter: &Value, payload: &Value, address_in_use: bool) -> Vec<Value> {
    let alias = field_string(adapter, "alias");
    let target = &payload["target"];
    let before = &payload["before"]["ipv4"];
    let mut notices = vec![];
    if target["ipv4"]["mode"] == "static" {
        let address = field_string(&target["ipv4"], "address");
        if address_in_use {
            notices.push(notice(
                "ip.addressInUse",
                json!({"address":address}),
                format!("Another device on this network already answers for {address}. Using it causes an address conflict."),
            ));
        }
        let prefix = field_i64(&target["ipv4"], "prefixLength");
        let current = arrays(adapter.get("ipv4Addresses"))
            .into_iter()
            .map(|item| (field_string(&item, "ipAddress"), field_i64(&item, "prefixLength")))
            .find(|(ip, _)| !ip.starts_with("169.254."));
        if let (Some(new), Some((old, old_prefix))) = (ipv4(&address), current.as_ref().and_then(|(ip, p)| ipv4(ip).map(|ip| (ip, *p)))) {
            let shared = prefix.min(old_prefix).clamp(1, 32);
            let mask = if shared == 32 { u32::MAX } else { !(u32::MAX >> shared) };
            if u32::from(new) & mask != u32::from(old) & mask {
                notices.push(notice(
                    "ip.subnetChange",
                    json!({"alias":alias,"from":format!("{old}/{old_prefix}"),"to":format!("{address}/{prefix}")}),
                    format!("{address}/{prefix} is on a different subnet than the current {old}/{old_prefix}. Devices on the current network may become unreachable."),
                ));
            }
        }
        if target["ipv4"]["gateway"].is_null() && field_bool(adapter, "defaultRoute") {
            notices.push(notice(
                "ip.noGateway",
                json!({"alias":alias}),
                format!("No gateway: {alias} will stop carrying Internet traffic. Other adapters, if any, take over."),
            ));
        }
    }
    if target["ipv4"]["mode"] == "dhcp" && before["mode"] == "static" {
        notices.push(notice(
            "ip.toDhcp",
            json!({"alias":alias}),
            format!("{alias} will ask the network's DHCP server for an address. Without a DHCP server it gets a 169.254.x.x address and no Internet."),
        ));
    }
    if target["dns"]["mode"] == "manual" {
        let local = arrays(target["dns"].get("servers"))
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|text| text.parse::<IpAddr>().ok())
            .any(|ip| ip.is_loopback());
        if local {
            notices.push(notice(
                "dns.loopback",
                json!({}),
                "A loopback DNS server only works while the local DNS program (for example a proxy client) is running.",
            ));
        }
    }
    notices
}

/// Whether another device answers ARP for `address` (local network only).
fn address_answers_arp(address: &str) -> bool {
    let Some(ip) = ipv4(address) else { return false };
    let mut mac = [0u8; 8];
    let mut length = mac.len() as u32;
    unsafe { SendARP(u32::from_ne_bytes(ip.octets()), 0, mac.as_mut_ptr().cast(), &mut length) == 0 && length > 0 }
}

fn ip_notices(adapter: &Value, payload: &Value) -> Vec<Value> {
    let mut notices = ip_change_notices(adapter, payload);
    notices.push(revert_notice());
    notices
}

/// Warnings for one adapter's IPv4/DNS change, without the revert notice
/// (shared with network backup restores, which list several adapters).
pub(crate) fn ip_change_notices(adapter: &Value, payload: &Value) -> Vec<Value> {
    let alias = field_string(adapter, "alias");
    let mut notices = vec![];
    let target_address = field_string(&payload["target"]["ipv4"], "address");
    let own = arrays(adapter.get("ipv4Addresses")).iter().any(|item| field_string(item, "ipAddress") == target_address);
    let in_use = !target_address.is_empty() && !own && address_answers_arp(&target_address);
    notices.extend(ip_risk_notices(adapter, payload, in_use));
    if !payload["target"]["ipv4"].is_null() {
        notices.push(notice(
            "ip.interrupt",
            json!({"alias":alias}),
            format!("{alias} briefly loses its IPv4 address while the change is applied. If it fails, the app restores the previous settings."),
        ));
        if field_bool(adapter, "defaultRoute") {
            notices.push(notice(
                "ip.defaultGateway",
                json!({"alias":alias}),
                "This adapter carries the default gateway; a wrong address or gateway can cut Internet access. Undo last app change restores it.",
            ));
        }
    }
    if payload["target"]["dns"]["mode"] == "manual" {
        notices.push(notice(
            "dns.manual",
            json!({"alias":alias}),
            "Name lookups on this adapter will use only the DNS servers you entered.",
        ));
    }
    notices
}

pub(crate) fn revert_notice() -> Value {
    notice(
        "change.autoRevert",
        json!({"seconds":REVERT_SECONDS.to_string()}),
        format!("After applying, choose Keep within {REVERT_SECONDS} seconds. Otherwise the previous settings are restored automatically, even if this window stops responding."),
    )
}

// ---------------------------------------------------------------------------
// Automatic revert ("Keep these settings?")
// ---------------------------------------------------------------------------

fn take_pending(id: u64) -> Option<Value> {
    let mut pending = PENDING_REVERT.lock().ok()?;
    if matches!(pending.as_ref(), Some((current, _)) if *current == id) {
        pending.take().map(|(_, scope)| scope)
    } else {
        None
    }
}

pub(crate) fn ensure_no_pending_revert() -> AppResult<()> {
    match PENDING_REVERT.lock() {
        Ok(pending) if pending.is_none() => Ok(()),
        _ => Err(err("REVERT_PENDING", "Keep or revert the previous change before making another one.")),
    }
}

/// Register `scope` for automatic restore after `REVERT_SECONDS` unless kept.
/// The timer runs in Rust, so it also works if the window stops responding.
pub(crate) fn schedule_revert(app: &AppHandle, scope: Value) -> Value {
    let id = NEXT_REVERT.fetch_add(1, Ordering::SeqCst) + 1;
    if let Ok(mut pending) = PENDING_REVERT.lock() {
        *pending = Some((id, scope));
    }
    let handle = app.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(REVERT_SECONDS));
        if let Some(scope) = take_pending(id) {
            let results = crate::run_revert(&handle, &scope);
            let _ = handle.emit("config-reverted", json!({"revertId":id,"automatic":true,"results":results}));
        }
    });
    json!({"revertId":id,"seconds":REVERT_SECONDS})
}

#[tauri::command]
pub fn keep_config_change(revert_id: u64) -> AppResult<Value> {
    take_pending(revert_id)
        .map(|_| {
            let message = notice("change.kept", json!({}), "The new settings were kept. Undo last app change can still restore the previous ones.");
            json!({"kept":true,"message":message["text"],"messageCode":message})
        })
        .ok_or_else(|| err("NOT_FOUND", "This change was already kept or reverted."))
}

#[tauri::command]
pub async fn revert_config_change(app: AppHandle, revert_id: u64) -> AppResult<Value> {
    blocking(move || {
        let scope = take_pending(revert_id).ok_or_else(|| err("NOT_FOUND", "This change was already kept or reverted."))?;
        let results = crate::run_revert(&app, &scope);
        let message = notice("change.reverted", json!({}), "The previous settings were restored.");
        Ok(json!({"reverted":true,"results":results,"message":message["text"],"messageCode":message}))
    })
    .await
}

pub const CATEGORIES: &[&str] = &["Private", "Public"];

pub fn category_payload(input: &Value, adapter: &Value) -> AppResult<Value> {
    let category = input
        .get("category")
        .and_then(Value::as_str)
        .filter(|value| CATEGORIES.contains(value))
        .ok_or_else(|| err("VALIDATION_ERROR", "Network category must be Private or Public."))?;
    let before = field_string(adapter, "networkCategory");
    if before.is_empty() {
        return Err(err("VALIDATION_ERROR", "This adapter has no network profile to change."));
    }
    if before == "DomainAuthenticated" {
        return Err(err("VALIDATION_ERROR", "Domain networks are managed by your organization."));
    }
    Ok(json!({
        "interfaceIndex":field_i64(adapter, "interfaceIndex"),
        "guid":field_string(adapter, "guid"),
        "alias":field_string(adapter, "alias"),
        "category":category,
        "before":before,
    }))
}

/// "Connect automatically" / "自动连接" in `netsh wlan show profile` output.
pub fn parse_connection_mode(output: &str) -> Option<&'static str> {
    output.lines().find_map(|line| {
        let (key, value) = crate::split_netsh_line(line)?;
        let key = key.trim().to_lowercase();
        if !(key.contains("connection mode") || key.contains("连接模式")) {
            return None;
        }
        let value = value.trim().to_lowercase();
        if value.contains("auto") || value.contains("自动") {
            Some("auto")
        } else if value.contains("manual") || value.contains("手动") {
            Some("manual")
        } else {
            None
        }
    })
}

/// Names passed to netsh must not contain quotes or control characters.
fn netsh_safe(name: &str) -> bool {
    !name.is_empty() && name.encode_utf16().count() <= 255 && !name.chars().any(|c| c == '"' || c.is_control())
}

pub const PROFILE_ACTIONS: &[&str] = &["forget", "moveUp", "moveDown", "setMode"];

/// The new 1-based priority for a move, or None when already at the edge.
pub fn moved_priority(order: &[String], name: &str, action: &str) -> Option<usize> {
    let index = order.iter().position(|item| item == name)?;
    match action {
        "moveUp" if index > 0 => Some(index),
        "moveDown" if index + 1 < order.len() => Some(index + 2),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Live reads
// ---------------------------------------------------------------------------

fn live_adapter(index: i64) -> AppResult<Value> {
    arrays(get_network_overview()?.get("interfaces"))
        .into_iter()
        .find(|item| field_i64(item, "interfaceIndex") == index)
        .ok_or_else(|| err("NOT_FOUND", "Interface no longer exists."))
}

fn adapter_by_guid(guid: &str) -> AppResult<Value> {
    let key = crate::network_insights::normalized_guid_key(guid);
    arrays(get_network_overview()?.get("interfaces"))
        .into_iter()
        .find(|item| crate::network_insights::normalized_guid_key(&field_string(item, "guid")) == key)
        .ok_or_else(|| err("NOT_FOUND", "The adapter was removed or recreated."))
}

fn ip_state(index: i64) -> AppResult<Value> {
    run_powershell_json(IP_STATE_SCRIPT, Some(&json!({"interfaceIndex":index})))
}

fn prepare_ip(input: &Value) -> AppResult<(Value, Value)> {
    let index = valid_index(input.get("interfaceIndex").unwrap_or(&Value::Null))?;
    let adapter = live_adapter(index)?;
    let current = ip_state(index)?;
    let payload = ip_settings_payload(input, &adapter, &current)?;
    Ok((adapter, payload))
}

fn wifi_profile_details(adapter_guid: &str) -> AppResult<Value> {
    let (alias, names) = wifi_windows::adapter_profiles(adapter_guid).map_err(|message| err("WIFI_DISCOVERY_FAILED", message))?;
    let profiles = names
        .iter()
        .take(64)
        .enumerate()
        .map(|(index, name)| {
            let mode = if netsh_safe(name) {
                run_netsh(&["wlan".into(), "show".into(), "profile".into(), format!("name={name}"), format!("interface={alias}")])
                    .ok()
                    .and_then(|output| parse_connection_mode(&output))
            } else {
                None
            };
            json!({"name":name,"priority":index + 1,"connectionMode":mode,"editable":netsh_safe(name)})
        })
        .collect::<Vec<_>>();
    Ok(json!({"adapterGuid":adapter_guid,"interfaceName":alias,"profiles":profiles}))
}

fn profile_payload(state: &State<AppState>, input: &Value) -> AppResult<(Value, Vec<String>)> {
    let guid = normalized_wifi_guid(input.get("adapterGuid").and_then(Value::as_str).unwrap_or(""))?;
    let action = input
        .get("action")
        .and_then(Value::as_str)
        .filter(|action| PROFILE_ACTIONS.contains(action))
        .ok_or_else(|| err("VALIDATION_ERROR", "Unsupported Wi-Fi profile action."))?;
    let name = input.get("profileName").and_then(Value::as_str).unwrap_or("").to_string();
    let (alias, order) = wifi_windows::adapter_profiles(&guid).map_err(|message| err("WIFI_DISCOVERY_FAILED", message))?;
    if !order.contains(&name) {
        return Err(err("NOT_FOUND", "The saved Wi-Fi profile no longer exists."));
    }
    if !netsh_safe(&name) || !netsh_safe(&alias) {
        return Err(err("VALIDATION_ERROR", "This profile name cannot be changed safely here. Use Windows Settings."));
    }
    let mut payload = json!({"adapterGuid":guid,"interfaceName":alias,"profileName":name,"action":action});
    match action {
        "setMode" => {
            let mode = input
                .get("mode")
                .and_then(Value::as_str)
                .filter(|mode| *mode == "auto" || *mode == "manual")
                .ok_or_else(|| err("VALIDATION_ERROR", "Connection mode must be auto or manual."))?;
            payload["mode"] = json!(mode);
        }
        "moveUp" | "moveDown" => {
            let priority = moved_priority(&order, &name, action)
                .ok_or_else(|| err("VALIDATION_ERROR", "The profile is already at that end of the list."))?;
            payload["priority"] = json!(priority);
            payload["order"] = json!(order);
        }
        _ => {
            let rules = wifi_auto_rules(&config(state))?;
            if rules.iter().any(|rule| {
                normalized_wifi_guid(&rule.adapter_guid).ok().as_deref() == Some(guid.as_str())
                    && (rule.profile_name == name || rule.fallback_profiles.contains(&name))
            }) {
                return Err(err("IN_USE", "An auto-connect rule uses this profile. Change or remove the rule first."));
            }
        }
    }
    Ok((payload, order))
}

fn netsh_profile_mode(alias: &str, name: &str, mode: &str) -> AppResult<String> {
    run_netsh(&[
        "wlan".into(),
        "set".into(),
        "profileparameter".into(),
        format!("name={name}"),
        format!("interface={alias}"),
        format!("connectionmode={mode}"),
    ])
}

fn netsh_profile_order(alias: &str, name: &str, priority: usize) -> AppResult<String> {
    run_netsh(&[
        "wlan".into(),
        "set".into(),
        "profileorder".into(),
        format!("name={name}"),
        format!("interface={alias}"),
        format!("priority={priority}"),
    ])
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_ip_settings(interface_index: i64) -> AppResult<Value> {
    blocking(move || {
        let index = valid_index(&json!(interface_index))?;
        let current = ip_state(index)?;
        Ok(json!({"interfaceIndex":index,"current":current,"target":current_target(&current)}))
    })
    .await
}

#[tauri::command]
pub async fn preview_ip_settings(app: AppHandle, input: Value) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        let (adapter, payload) = prepare_ip(&input)?;
        let notices = ip_notices(&adapter, &payload);
        let token = if field_bool(&payload, "unchanged") {
            None
        } else {
            Some(issue_confirmation(&state, "ip-settings", &payload)?)
        };
        Ok(json!({"settings":payload,"warnings":notice_texts(&notices),"notices":notices,"confirmationToken":token,"requiresAdmin":true}))
    })
    .await
}

#[tauri::command]
pub async fn apply_ip_settings(app: AppHandle, input: Value) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        let request = input.get("settings").cloned().unwrap_or(Value::Null);
        let (_, payload) = prepare_ip(&request)?;
        if field_bool(&payload, "unchanged") {
            return Err(err("VALIDATION_ERROR", "Nothing to change."));
        }
        consume_confirmation(&state, input.get("confirmationToken").and_then(Value::as_str).unwrap_or(""), "ip-settings", &payload)?;
        require_admin()?;
        ensure_no_pending_revert()?;
        let scope = json!({"ipSettings":{
            "interfaceIndex":payload["interfaceIndex"],"guid":payload["guid"],"alias":payload["alias"],
            "after":payload["target"],"before":payload["before"]
        }});
        capture_baseline(&state, scope.clone())?;
        let result = run_powershell_json(
            APPLY_IP_SCRIPT,
            Some(&json!({"interfaceIndex":payload["interfaceIndex"],"target":payload["target"],"rollback":payload["rollback"]})),
        )?;
        let revert = schedule_revert(&app, scope);
        let alias = field_string(&payload, "alias");
        let message = notice("ip.applied", json!({"alias":alias,"seconds":REVERT_SECONDS.to_string()}), format!("IP settings updated on {alias}. Keep them within {REVERT_SECONDS} seconds or they are reverted."));
        Ok(json!({"result":result,"revert":revert,"message":message["text"],"messageCode":message}))
    })
    .await
}

fn prepare_category(input: &Value) -> AppResult<(Value, Vec<Value>)> {
    let index = valid_index(input.get("interfaceIndex").unwrap_or(&Value::Null))?;
    let adapter = live_adapter(index)?;
    let payload = category_payload(input, &adapter)?;
    let mut notices = category_notices(&adapter, &field_string(&payload, "category"));
    notices.push(revert_notice());
    Ok((payload, notices))
}

/// Warnings for a network category change, without the revert notice.
pub(crate) fn category_notices(adapter: &Value, category: &str) -> Vec<Value> {
    let alias = field_string(adapter, "alias");
    let mut notices = vec![];
    if category == "Private" {
        notices.push(notice(
            "category.private",
            json!({"alias":alias}),
            "Private lets other devices on this network discover this PC, depending on your sharing settings. Use it only on trusted networks.",
        ));
    } else {
        notices.push(notice(
            "category.public",
            json!({"alias":alias}),
            "Public hides this PC from other devices; file and printer sharing on this network may stop working.",
        ));
    }
    if let Some(owner) = coexistence::owner_of(&adapter) {
        notices.push(notice(
            "category.protected",
            json!({"alias":alias,"owner":owner}),
            format!("{alias} is managed by {owner}. {owner} may reset the category when it recreates the adapter."),
        ));
    }
    notices
}

#[tauri::command]
pub async fn preview_network_category(app: AppHandle, input: Value) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        let (payload, notices) = prepare_category(&input)?;
        let unchanged = payload["category"] == payload["before"];
        let token = if unchanged { None } else { Some(issue_confirmation(&state, "network-category", &payload)?) };
        Ok(json!({"category":payload,"unchanged":unchanged,"warnings":notice_texts(&notices),"notices":notices,"confirmationToken":token}))
    })
    .await
}

#[tauri::command]
pub async fn apply_network_category(app: AppHandle, input: Value) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        let (payload, _) = prepare_category(input.get("category").unwrap_or(&Value::Null))?;
        consume_confirmation(&state, input.get("confirmationToken").and_then(Value::as_str).unwrap_or(""), "network-category", &payload)?;
        require_admin()?;
        ensure_no_pending_revert()?;
        let scope = json!({"networkCategory":payload});
        capture_baseline(&state, scope.clone())?;
        let result = run_powershell_json(CATEGORY_SCRIPT, Some(&payload))?;
        let revert = schedule_revert(&app, scope);
        let alias = field_string(&payload, "alias");
        let message = notice(
            "category.applied",
            json!({"alias":alias,"category":payload["category"]}),
            format!("{alias} is now a {} network.", field_string(&payload, "category")),
        );
        Ok(json!({"result":result,"revert":revert,"message":message["text"],"messageCode":message}))
    })
    .await
}

#[tauri::command]
pub async fn get_wifi_profile_details(adapter_guid: String) -> AppResult<Value> {
    blocking(move || {
        let guid = normalized_wifi_guid(&adapter_guid)?;
        wifi_profile_details(&guid)
    })
    .await
}

fn profile_notices(payload: &Value) -> Vec<Value> {
    let name = field_string(payload, "profileName");
    match field_string(payload, "action").as_str() {
        "forget" => vec![notice(
            "wifiProfile.forget",
            json!({"profileName":name}),
            format!("Windows deletes \"{name}\" and its saved password. This cannot be undone; you will need the password to connect again."),
        )],
        "setMode" if field_string(payload, "mode") == "manual" => vec![notice(
            "wifiProfile.manual",
            json!({"profileName":name}),
            format!("Windows will no longer join \"{name}\" automatically."),
        )],
        "setMode" => vec![notice(
            "wifiProfile.auto",
            json!({"profileName":name}),
            format!("Windows may join \"{name}\" automatically when it is in range."),
        )],
        _ => vec![notice(
            "wifiProfile.order",
            json!({"profileName":name,"priority":payload["priority"].to_string()}),
            format!("\"{name}\" moves to priority {}. Windows prefers networks higher in the list.", payload["priority"]),
        )],
    }
}

#[tauri::command]
pub async fn preview_wifi_profile_change(app: AppHandle, input: Value) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        let (payload, _) = profile_payload(&state, &input)?;
        let notices = profile_notices(&payload);
        Ok(json!({"change":payload,"warnings":notice_texts(&notices),"notices":notices,"confirmationToken":issue_confirmation(&state, "wifi-profile", &payload)?}))
    })
    .await
}

#[tauri::command]
pub async fn apply_wifi_profile_change(app: AppHandle, input: Value) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        let (payload, order) = profile_payload(&state, input.get("change").unwrap_or(&Value::Null))?;
        consume_confirmation(&state, input.get("confirmationToken").and_then(Value::as_str).unwrap_or(""), "wifi-profile", &payload)?;
        let alias = field_string(&payload, "interfaceName");
        let name = field_string(&payload, "profileName");
        let action = field_string(&payload, "action");
        match action.as_str() {
            "forget" => {
                run_netsh(&["wlan".into(), "delete".into(), "profile".into(), format!("name={name}"), format!("interface={alias}")])?;
            }
            "setMode" => {
                let before = run_netsh(&["wlan".into(), "show".into(), "profile".into(), format!("name={name}"), format!("interface={alias}")])
                    .ok()
                    .and_then(|output| parse_connection_mode(&output));
                if let Some(before) = before {
                    capture_baseline(&state, json!({"wifiProfileMode":{"adapterGuid":payload["adapterGuid"],"interfaceName":alias,"profileName":name,"before":before}}))?;
                }
                netsh_profile_mode(&alias, &name, &field_string(&payload, "mode"))?;
            }
            _ => {
                capture_baseline(&state, json!({"wifiProfileOrder":{"adapterGuid":payload["adapterGuid"],"interfaceName":alias,"before":order}}))?;
                netsh_profile_order(&alias, &name, field_i64(&payload, "priority") as usize)?;
            }
        }
        let message = notice(
            "wifiProfile.applied",
            json!({"profileName":name,"action":action}),
            format!("Saved Wi-Fi profile \"{name}\" updated."),
        );
        Ok(json!({"change":payload,"message":message["text"],"messageCode":message}))
    })
    .await
}

// ---------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------

fn result_of(kind: &str, result: Result<Value, PublicError>) -> Value {
    match result {
        Ok(value) => json!({"status":"success","kind":kind,"result":value}),
        Err(error) => json!({"status":"failed","kind":kind,"error":error}),
    }
}

/// Restore the configuration parts recorded by an undo baseline.
pub fn restore_scope(scope: &Value) -> Vec<Value> {
    let mut results = vec![];
    if let Some(item) = scope.get("ipSettings") {
        results.push(match adapter_by_guid(&field_string(item, "guid")) {
            Ok(adapter) => {
                let after = &item["after"];
                let target = json!({
                    "ipv4":if after["ipv4"].is_null() { Value::Null } else { item["before"]["ipv4"].clone() },
                    "dns":if after["dns"].is_null() { Value::Null } else { item["before"]["dns"].clone() },
                });
                let input = json!({"interfaceIndex":field_i64(&adapter, "interfaceIndex"),"target":target,"rollback":Value::Null});
                result_of("ipSettings", run_powershell_json(APPLY_IP_SCRIPT, Some(&input)))
            }
            Err(_) => json!({"status":"skipped","kind":"ipSettings","message":"The adapter was removed or recreated."}),
        });
    }
    if let Some(item) = scope.get("networkCategory") {
        results.push(match adapter_by_guid(&field_string(item, "guid")) {
            Ok(adapter) => result_of(
                "networkCategory",
                run_powershell_json(
                    CATEGORY_SCRIPT,
                    Some(&json!({"interfaceIndex":field_i64(&adapter, "interfaceIndex"),"category":item["before"]})),
                ),
            ),
            Err(_) => json!({"status":"skipped","kind":"networkCategory","message":"The adapter was removed or recreated."}),
        });
    }
    if let Some(item) = scope.get("wifiProfileMode") {
        let (alias, name, mode) = (field_string(item, "interfaceName"), field_string(item, "profileName"), field_string(item, "before"));
        results.push(if netsh_safe(&alias) && netsh_safe(&name) && (mode == "auto" || mode == "manual") {
            result_of("wifiProfileMode", netsh_profile_mode(&alias, &name, &mode).map(Value::String))
        } else {
            json!({"status":"skipped","kind":"wifiProfileMode","message":"The recorded profile cannot be restored."})
        });
    }
    if let Some(item) = scope.get("wifiProfileOrder") {
        match wifi_windows::adapter_profiles(&field_string(item, "adapterGuid")) {
            Ok((alias, current)) => {
                for (index, name) in arrays(item.get("before")).iter().filter_map(Value::as_str).enumerate() {
                    if current.iter().any(|item| item == name) && netsh_safe(name) {
                        results.push(result_of("wifiProfileOrder", netsh_profile_order(&alias, name, index + 1).map(Value::String)));
                    }
                }
            }
            Err(message) => results.push(json!({"status":"skipped","kind":"wifiProfileOrder","message":message})),
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dotted IPv4 text for test data.
    fn ip(a: u8, b: u8, c: u8, d: u8) -> String {
        Ipv4Addr::new(a, b, c, d).to_string()
    }

    #[test]
    fn static_ipv4_validation() {
        assert!(validate_static_ipv4(&ip(192, 168, 1, 20), 24, Some(&ip(192, 168, 1, 1))).is_ok());
        assert!(validate_static_ipv4(&ip(10, 1, 2, 3), 8, None).is_ok());
        assert!(validate_static_ipv4(&ip(10, 1, 2, 3), 32, Some("")).is_ok());
        let cases: Vec<(String, i64, Option<String>)> = vec![
            ("192.168.1".into(), 24, None),
            (ip(192, 168, 1, 20), 33, None),
            (ip(192, 168, 1, 20), 0, None),
            (ip(127, 0, 0, 5), 24, None),
            (ip(169, 254, 3, 4), 24, None),
            (ip(0, 1, 2, 3), 16, None),
            (ip(224, 0, 0, 9), 4, None),
            (ip(192, 168, 1, 0), 24, None),
            (ip(192, 168, 1, 255), 24, None),
            (ip(192, 168, 1, 20), 24, Some(ip(192, 168, 2, 1))),
            (ip(192, 168, 1, 20), 24, Some(ip(192, 168, 1, 20))),
            (ip(192, 168, 1, 20), 24, Some(ip(192, 168, 1, 255))),
            (ip(192, 168, 1, 20), 24, Some("gateway".into())),
        ];
        for (address, prefix, gateway) in cases {
            assert!(validate_static_ipv4(&address, prefix, gateway.as_deref()).is_err(), "{address}/{prefix} via {gateway:?}");
        }
    }

    #[test]
    fn dns_servers_are_normalized() {
        let (one, two) = (ip(1, 1, 1, 1), ip(8, 8, 8, 8));
        assert_eq!(normalize_dns(&json!([one, format!(" {two} "), "", one])).unwrap(), vec![one.clone(), two.clone()]);
        assert_eq!(normalize_dns(&json!(["2606:4700:4700:0:0:0:0:1111"])).unwrap(), vec!["2606:4700:4700::1111"]);
        assert!(normalize_dns(&json!([])).is_err());
        assert!(normalize_dns(&json!(["dns.google"])).is_err());
        assert!(normalize_dns(&json!([ip(0, 0, 0, 0)])).is_err());
        assert!(normalize_dns(&json!([ip(224, 0, 0, 251)])).is_err());
        let five = (1..=5).map(|d| ip(9, 9, 9, d)).collect::<Vec<_>>();
        assert!(normalize_dns(&json!(five)).is_err());
        assert!(normalize_dns(&json!([format!("{one}; Remove-Item C:\\")])).is_err());
    }

    fn adapter(owner: Value) -> Value {
        json!({"interfaceIndex":23,"alias":"以太网","guid":"{AAAA}","managedBy":owner,"systemInterface":false,"defaultRoute":true,"networkCategory":"Public"})
    }

    fn dhcp_state() -> Value {
        json!({"dhcp":true,"addresses":[{"ipAddress":ip(192, 168, 1, 20),"prefixLength":24,"origin":"Dhcp"}],"gateways":[ip(192, 168, 1, 1)],"dnsServers":[ip(192, 168, 1, 1)],"dnsStatic":false})
    }

    fn static_state() -> Value {
        json!({"dhcp":false,"addresses":[{"ipAddress":ip(192, 168, 1, 50),"prefixLength":24,"origin":"Manual"}],"gateways":[ip(192, 168, 1, 1)],"dnsServers":[ip(1, 1, 1, 1)],"dnsStatic":true})
    }

    #[test]
    fn ip_payloads_drop_unchanged_parts_and_keep_rollback() {
        let static_request = json!({
            "interfaceIndex":23,
            "ipv4":{"mode":"static","address":ip(192, 168, 1, 50),"prefixLength":"24","gateway":ip(192, 168, 1, 1)},
            "dns":{"mode":"manual","servers":[ip(1, 1, 1, 1), ip(8, 8, 8, 8)]}
        });
        let payload = ip_settings_payload(&static_request, &adapter(Value::Null), &dhcp_state()).unwrap();
        assert_eq!(payload["target"]["ipv4"]["prefixLength"], 24);
        assert_eq!(payload["rollback"]["ipv4"], json!({"mode":"dhcp"}));
        assert_eq!(payload["rollback"]["dns"], json!({"mode":"automatic"}));
        assert_eq!(payload["unchanged"], false);
        let same = json!({"interfaceIndex":23,"ipv4":{"mode":"dhcp"},"dns":{"mode":"automatic"}});
        assert_eq!(ip_settings_payload(&same, &adapter(Value::Null), &dhcp_state()).unwrap()["unchanged"], true);
        let dns_only = json!({"interfaceIndex":23,"dns":{"mode":"manual","servers":[ip(1, 1, 1, 1)]}});
        let payload = ip_settings_payload(&dns_only, &adapter(Value::Null), &dhcp_state()).unwrap();
        assert!(payload["target"]["ipv4"].is_null());
        assert!(payload["rollback"]["ipv4"].is_null());
        let no_dns = json!({"interfaceIndex":23,"ipv4":{"mode":"static","address":ip(192, 168, 1, 50),"prefixLength":24}});
        assert!(ip_settings_payload(&no_dns, &adapter(Value::Null), &dhcp_state()).is_err(), "static needs manual DNS");
    }

    #[test]
    fn protected_adapters_refuse_ip_and_dns_but_allow_category() {
        let request = json!({"interfaceIndex":33,"dns":{"mode":"manual","servers":[ip(1, 1, 1, 1)]}});
        assert_eq!(
            ip_settings_payload(&request, &adapter(json!("EasyTier")), &dhcp_state()).unwrap_err().code,
            "PROTECTED_ADAPTER"
        );
        let payload = category_payload(&json!({"category":"Private"}), &adapter(json!("EasyTier"))).unwrap();
        assert_eq!(payload["before"], "Public");
        assert!(category_payload(&json!({"category":"DomainAuthenticated"}), &adapter(Value::Null)).is_err());
        let mut domain = adapter(Value::Null);
        domain["networkCategory"] = json!("DomainAuthenticated");
        assert!(category_payload(&json!({"category":"Private"}), &domain).is_err());
    }

    #[test]
    fn risky_ip_changes_get_extra_warnings() {
        let mut eth = adapter(Value::Null);
        eth["ipv4Addresses"] = json!([{"ipAddress":ip(192, 168, 1, 20),"prefixLength":24}]);
        let request = json!({"interfaceIndex":23,"ipv4":{"mode":"static","address":ip(10, 0, 0, 50),"prefixLength":24},"dns":{"mode":"manual","servers":[ip(127, 0, 0, 1)]}});
        let payload = ip_settings_payload(&request, &eth, &dhcp_state()).unwrap();
        let codes = ip_risk_notices(&eth, &payload, true).iter().map(|item| item["code"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(codes, vec!["ip.addressInUse", "ip.subnetChange", "ip.noGateway", "dns.loopback"]);
        let same_subnet = json!({"interfaceIndex":23,"ipv4":{"mode":"static","address":ip(192, 168, 1, 50),"prefixLength":24,"gateway":ip(192, 168, 1, 1)},"dns":{"mode":"manual","servers":[ip(1, 1, 1, 1)]}});
        let payload = ip_settings_payload(&same_subnet, &eth, &dhcp_state()).unwrap();
        assert!(ip_risk_notices(&eth, &payload, false).is_empty());
        let to_dhcp = json!({"interfaceIndex":23,"ipv4":{"mode":"dhcp"}});
        let payload = ip_settings_payload(&to_dhcp, &eth, &static_state()).unwrap();
        assert_eq!(ip_risk_notices(&eth, &payload, false)[0]["code"], "ip.toDhcp");
    }

    #[test]
    fn pending_reverts_are_taken_once() {
        *PENDING_REVERT.lock().unwrap() = Some((41, json!({"networkCategory":{}})));
        assert!(ensure_no_pending_revert().is_err(), "a second change waits for keep/revert");
        assert!(take_pending(40).is_none());
        assert!(take_pending(41).is_some());
        assert!(take_pending(41).is_none());
        assert!(ensure_no_pending_revert().is_ok());
    }

    #[test]
    fn current_static_configuration_is_described() {
        assert_eq!(
            current_target(&static_state()),
            json!({"ipv4":{"mode":"static","address":ip(192, 168, 1, 50),"prefixLength":24,"gateway":ip(192, 168, 1, 1)},"dns":{"mode":"manual","servers":[ip(1, 1, 1, 1)]}})
        );
    }

    #[test]
    fn wifi_profile_helpers() {
        assert_eq!(parse_connection_mode("    Connection mode        : Connect automatically\n"), Some("auto"));
        assert_eq!(parse_connection_mode("    Connection mode        : Connect manually\n"), Some("manual"));
        assert_eq!(parse_connection_mode("    连接模式               : 自动连接\n"), Some("auto"));
        assert_eq!(parse_connection_mode("    连接模式               : 手动连接\n"), Some("manual"));
        assert_eq!(parse_connection_mode("Name : x"), None);
        let order = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        assert_eq!(moved_priority(&order, "B", "moveUp"), Some(1));
        assert_eq!(moved_priority(&order, "B", "moveDown"), Some(3));
        assert_eq!(moved_priority(&order, "A", "moveUp"), None);
        assert_eq!(moved_priority(&order, "C", "moveDown"), None);
        assert_eq!(moved_priority(&order, "Z", "moveUp"), None);
        assert!(netsh_safe("转化医学研究中心813"));
        assert!(!netsh_safe("bad\"name"));
        assert!(!netsh_safe("line\nbreak"));
    }

    #[test]
    #[ignore = "manual, read-only check of the IP state script"]
    fn read_only_ip_state() {
        let overview = get_network_overview().unwrap();
        for item in arrays(overview.get("interfaces")).iter().filter(|item| field_bool(item, "defaultRoute")) {
            let current = ip_state(field_i64(item, "interfaceIndex")).unwrap();
            println!("{} {} -> {}", field_string(item, "alias"), current, current_target(&current));
        }
    }
}
