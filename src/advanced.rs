//! Advanced features: network triggers ("on Wi-Fi X apply saved
//! configuration Y"), connecting to a new Wi-Fi network, built-in Windows VPN
//! connections, domain-based routes and the hosts file editor.
//!
//! Each write is previewed and confirmed with a token. Domain routes refuse
//! proxy fake-IP answers (the 198.18/15 range) and warn when a TUN captures default
//! traffic. The hosts editor touches only the fixed Windows hosts path and
//! keeps backups in the app-data folder. Passwords for new Wi-Fi networks stay
//! in memory, go only to Windows, and are wiped after use.

use crate::{
    add_route_record, arrays, blocking, capture_baseline, coexistence, config, consume_confirmation, err, err_with,
    field_bool, field_i64, field_string, fingerprint, get_routes, hidden_command, is_admin, issue_confirmation,
    net_tools, network_insights::{normalized_guid_key, notice, notice_texts}, normalized_wifi_guid, persist,
    remove_route_record, require_admin, restore_snapshot, route_key, route_payload, route_remove_payload,
    run_powershell_json, snapshot_current, store_recent, wide_null, wifi_windows, AppConfig, AppResult, AppState,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    ffi::OsStr,
    fs,
    net::IpAddr,
    path::{Path, PathBuf},
    ptr,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, State};
use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};

// ===========================================================================
// Network triggers
// ===========================================================================

pub const MAX_TRIGGERS: usize = 20;
const TRIGGER_COOLDOWN: Duration = Duration::from_secs(60);
static LAST_TRIGGER_RUN: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NetworkTrigger {
    pub id: String,
    pub adapter_guid: String,
    pub profile_name: String,
    pub saved_config_id: String,
    #[serde(default)]
    pub enabled: bool,
}

pub fn validate_trigger(trigger: &NetworkTrigger, cfg: &AppConfig) -> Result<(), String> {
    if !crate::wifi_auto_connect::valid_guid(&trigger.adapter_guid) {
        return Err("Invalid Wi-Fi adapter.".into());
    }
    let name = &trigger.profile_name;
    if name.trim().is_empty() || name.encode_utf16().count() > 255 || name.chars().any(char::is_control) {
        return Err("Invalid Wi-Fi profile name.".into());
    }
    if trigger.id.is_empty() || trigger.id.len() > 64 || trigger.id.chars().any(|c| !c.is_ascii_alphanumeric() && c != '-') {
        return Err("Invalid trigger id.".into());
    }
    if !cfg.profiles.iter().any(|item| field_string(item, "id") == trigger.saved_config_id && item.get("state").is_some()) {
        return Err("The saved configuration no longer exists or has no network state.".into());
    }
    Ok(())
}

pub fn triggers(cfg: &AppConfig) -> Vec<NetworkTrigger> {
    cfg.network_triggers.iter().filter_map(|item| serde_json::from_value(item.clone()).ok()).collect()
}

/// Enabled triggers for a Wi-Fi connection on `adapter_guid` to `profile`.
pub fn matching_triggers(list: &[NetworkTrigger], adapter_guid: &str, profile: &str) -> Vec<NetworkTrigger> {
    let key = normalized_guid_key(adapter_guid);
    list.iter()
        .filter(|trigger| trigger.enabled && normalized_guid_key(&trigger.adapter_guid) == key && trigger.profile_name == profile)
        .cloned()
        .collect()
}

fn cooled_down(id: &str) -> bool {
    let Ok(mut guard) = LAST_TRIGGER_RUN.lock() else { return false };
    let map = guard.get_or_insert_with(HashMap::new);
    if map.get(id).map(|last| last.elapsed() < TRIGGER_COOLDOWN).unwrap_or(false) {
        return false;
    }
    map.insert(id.to_string(), Instant::now());
    true
}

fn trigger_event(code: &str, params: Value, text: String) -> Value {
    let mut item = notice(code, params, text);
    item["kind"] = json!("trigger");
    item["tunnel"] = json!(false);
    item
}

fn record_trigger_event(app: &AppHandle, event: Value) {
    let stamped = app.state::<AppState>().monitor.record(vec![event]);
    crate::on_network_events(app, &stamped);
}

/// Run matching triggers for Wi-Fi connection events from the monitor.
pub fn handle_events(app: &AppHandle, events: &[Value]) {
    let wifi = events.iter().filter(|event| {
        matches!(event.get("code").and_then(Value::as_str), Some("event.wifiConnected" | "event.wifiSwitched"))
    });
    let state = app.state::<AppState>();
    let list = triggers(&config(&state));
    for event in wifi {
        let guid = event.pointer("/params/adapterGuid").and_then(Value::as_str).unwrap_or_default();
        let profile = event.pointer("/params/profile").and_then(Value::as_str).unwrap_or_default();
        for trigger in matching_triggers(&list, guid, profile) {
            if !cooled_down(&trigger.id) {
                continue;
            }
            let handle = app.clone();
            std::thread::spawn(move || run_trigger(&handle, &trigger));
        }
    }
}

/// Apply a saved configuration's Metric and app-managed routes. Wi-Fi and
/// adapter power are never changed by a trigger.
fn run_trigger(app: &AppHandle, trigger: &NetworkTrigger) {
    let state = app.state::<AppState>();
    let cfg = config(&state);
    let name = cfg
        .profiles
        .iter()
        .find(|item| field_string(item, "id") == trigger.saved_config_id)
        .map(|item| field_string(item, "name"))
        .unwrap_or_default();
    let params = json!({"profileName":trigger.profile_name,"name":name});
    if cfg!(debug_assertions) {
        return record_trigger_event(app, trigger_event("event.triggerDevelopment", params, format!("Connected to {}: \"{name}\" was not applied because triggers are inactive in development builds.", trigger.profile_name)));
    }
    if !is_admin() {
        return record_trigger_event(app, trigger_event("event.triggerNeedsAdmin", params, format!("Connected to {}: \"{name}\" was not applied because the app is not running as administrator.", trigger.profile_name)));
    }
    let Some(snapshot) = cfg
        .profiles
        .iter()
        .find(|item| field_string(item, "id") == trigger.saved_config_id)
        .and_then(|item| item.get("state").cloned())
    else {
        return record_trigger_event(app, trigger_event("event.triggerMissing", params, format!("Connected to {}: the saved configuration no longer exists.", trigger.profile_name)));
    };
    let scope = json!({"restoreProfile":trigger.saved_config_id,"trigger":true});
    let outcome = capture_baseline(&state, scope.clone())
        .and_then(|_| snapshot_current(&state))
        .and_then(|current| store_recent(&state, "Before automatic configuration".into(), current))
        .and_then(|_| restore_snapshot(&state, &snapshot, &scope));
    let event = match outcome {
        Ok(result) if !crate::has_failed_result(&result) => trigger_event(
            "event.triggerApplied",
            params,
            format!("Connected to {}: applied \"{name}\" (Metric and app-managed routes).", trigger.profile_name),
        ),
        Ok(_) => trigger_event("event.triggerPartial", params, format!("Connected to {}: \"{name}\" was only partly applied. Undo last app change can restore the previous state.", trigger.profile_name)),
        Err(error) => trigger_event("event.triggerFailed", json!({"profileName":trigger.profile_name,"name":name,"reason":error.message}), format!("Connected to {}: \"{name}\" could not be applied: {}", trigger.profile_name, error.message)),
    };
    record_trigger_event(app, event);
}

fn trigger_from(input: &Value) -> AppResult<NetworkTrigger> {
    let mut trigger: NetworkTrigger = serde_json::from_value(json!({
        "id":input.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()).map(str::to_owned).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        "adapterGuid":input.get("adapterGuid"),
        "profileName":input.get("profileName"),
        "savedConfigId":input.get("savedConfigId"),
        "enabled":input.get("enabled").and_then(Value::as_bool).unwrap_or(false),
    }))
    .map_err(|_| err("VALIDATION_ERROR", "Invalid automatic configuration."))?;
    trigger.adapter_guid = normalized_wifi_guid(&trigger.adapter_guid)?;
    Ok(trigger)
}

#[tauri::command]
pub fn preview_network_trigger(state: State<AppState>, input: Value) -> AppResult<Value> {
    let trigger = trigger_from(&input)?;
    let cfg = config(&state);
    validate_trigger(&trigger, &cfg).map_err(|message| err("VALIDATION_ERROR", message))?;
    let existing = triggers(&cfg);
    if !existing.iter().any(|item| item.id == trigger.id) && existing.len() >= MAX_TRIGGERS {
        return Err(err("VALIDATION_ERROR", "Too many automatic configurations."));
    }
    let payload = serde_json::to_value(&trigger).map_err(|_| err("SERIALIZE_ERROR", "Cannot prepare the automatic configuration."))?;
    let notices = vec![
        notice("trigger.scope", json!({}), "When the adapter connects to this network, the app applies the saved configuration's Metric and app-managed routes without asking again. Wi-Fi and adapter power are not changed."),
        notice("trigger.admin", json!({}), "Triggers run only while the app runs as administrator (it can stay in the tray), and never in development builds. Each run records an undo point and an event."),
    ];
    Ok(json!({"trigger":payload,"warnings":notice_texts(&notices),"notices":notices,"confirmationToken":issue_confirmation(&state, "network-trigger", &payload)?}))
}

// Configuration-only write: stores the trigger; nothing runs until a connection event.
#[tauri::command]
pub fn save_network_trigger(state: State<AppState>, input: Value) -> AppResult<Value> {
    let trigger = trigger_from(input.get("trigger").unwrap_or(&Value::Null))?;
    let mut cfg = config(&state);
    validate_trigger(&trigger, &cfg).map_err(|message| err("VALIDATION_ERROR", message))?;
    let payload = serde_json::to_value(&trigger).map_err(|_| err("SERIALIZE_ERROR", "Cannot save the automatic configuration."))?;
    consume_confirmation(&state, input.get("confirmationToken").and_then(Value::as_str).unwrap_or(""), "network-trigger", &payload)?;
    cfg.network_triggers.retain(|item| field_string(item, "id") != trigger.id);
    cfg.network_triggers.push(payload.clone());
    persist(&state, &cfg)?;
    let message = notice("trigger.saved", json!({"profileName":trigger.profile_name}), "Automatic configuration saved.");
    Ok(json!({"trigger":payload,"message":message["text"],"messageCode":message}))
}

// Configuration-only write: removes a stored trigger.
#[tauri::command]
pub fn remove_network_trigger(state: State<AppState>, id: String) -> AppResult<Value> {
    let mut cfg = config(&state);
    let before = cfg.network_triggers.len();
    cfg.network_triggers.retain(|item| field_string(item, "id") != id);
    if cfg.network_triggers.len() == before {
        return Err(err("NOT_FOUND", "The automatic configuration no longer exists."));
    }
    persist(&state, &cfg)?;
    let message = notice("trigger.removed", json!({}), "Automatic configuration removed.");
    Ok(json!({"removed":true,"message":message["text"],"messageCode":message}))
}

// ===========================================================================
// Connect to a new Wi-Fi network
// ===========================================================================

fn new_network_payload(input: &Value) -> AppResult<Value> {
    let guid = normalized_wifi_guid(input.get("adapterGuid").and_then(Value::as_str).unwrap_or(""))?;
    let ssid = input.get("ssid").and_then(Value::as_str).unwrap_or("").to_string();
    let security = input.get("security").and_then(Value::as_str).unwrap_or("").to_string();
    // Validate everything except the password, which the preview never sees.
    let probe = if security == "Open" { String::new() } else { "placeholder-pass".into() };
    wifi_windows::validate_new_network(&ssid, &security, &probe).map_err(|message| err("VALIDATION_ERROR", message))?;
    Ok(json!({"adapterGuid":guid,"ssid":ssid,"security":security}))
}

fn check_new_network(payload: &Value) -> AppResult<String> {
    let (alias, visible, saved) = wifi_windows::new_network_status(&field_string(payload, "adapterGuid"), &field_string(payload, "ssid"))
        .map_err(|message| err("WIFI_DISCOVERY_FAILED", message))?;
    if saved {
        return Err(err("CONFLICT", "This network is already saved. Connect to it from the adapter's saved profiles."));
    }
    if !visible {
        return Err(err("NOT_FOUND", "The network is no longer in range. Scan again."));
    }
    Ok(alias)
}

#[tauri::command]
pub async fn preview_connect_new_wifi(app: AppHandle, input: Value) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        let payload = new_network_payload(&input)?;
        let alias = check_new_network(&payload)?;
        let mut notices = vec![notice(
            "wifiNew.password",
            json!({}),
            "The password goes only to Windows, which stores it in the new Wi-Fi profile. This app never saves, logs or exports it.",
        )];
        if field_string(&payload, "security") == "Open" {
            notices.push(notice("wifiNew.open", json!({}), "This network is open: other people nearby can see unencrypted traffic."));
        }
        notices.push(notice(
            "wifiNew.switch",
            json!({"interfaceName":alias}),
            format!("{alias} leaves its current network to join this one."),
        ));
        Ok(json!({"network":payload,"interfaceName":alias,"warnings":notice_texts(&notices),"notices":notices,"confirmationToken":issue_confirmation(&state, "wifi-new", &payload)?}))
    })
    .await
}

#[tauri::command]
pub async fn connect_new_wifi(app: AppHandle, input: Value, mut password: String) -> AppResult<Value> {
    let result = blocking({
        let secret = password.clone();
        move || {
            let mut secret = secret;
            let outcome = (|| {
                let state = app.state::<AppState>();
                let payload = new_network_payload(&input)?;
                consume_confirmation(&state, input.get("confirmationToken").and_then(Value::as_str).unwrap_or(""), "wifi-new", &payload)?;
                let alias = check_new_network(&payload)?;
                capture_baseline(&state, json!({"wifi":[{"interfaceName":alias}]}))?;
                wifi_windows::connect_new_network(
                    &field_string(&payload, "adapterGuid"),
                    &field_string(&payload, "ssid"),
                    &field_string(&payload, "security"),
                    &secret,
                    is_admin(),
                )
                .map_err(|message| err("WIFI_CONNECT_FAILED", message))?;
                let ssid = field_string(&payload, "ssid");
                let message = notice("wifiNew.requested", json!({"interfaceName":alias,"ssid":ssid}), format!("Connection request sent to {alias} for {ssid}."));
                Ok(json!({"message":message["text"],"messageCode":message}))
            })();
            wifi_windows::wipe(&mut secret);
            outcome
        }
    })
    .await;
    wifi_windows::wipe(&mut password);
    result
}

// ===========================================================================
// Built-in Windows VPN connections
// ===========================================================================

const VPN_LIST_SCRIPT: &str = r#"function Convert-Vpn($v,$all){[PSCustomObject]@{name=[string]$v.Name;serverAddress=[string]$v.ServerAddress;tunnelType=[string]$v.TunnelType;status=[string]$v.ConnectionStatus;splitTunneling=[bool]$v.SplitTunneling;allUsers=$all}}
$items=@(Get-VpnConnection -ErrorAction SilentlyContinue|ForEach-Object{Convert-Vpn $_ $false})
$items+=@(Get-VpnConnection -AllUserConnection -ErrorAction SilentlyContinue|ForEach-Object{Convert-Vpn $_ $true})
[PSCustomObject]@{connections=$items}|ConvertTo-Json -Compress -Depth 4"#;

fn vpn_connections() -> AppResult<Vec<Value>> {
    Ok(arrays(run_powershell_json(VPN_LIST_SCRIPT, None)?.get("connections")))
}

pub fn valid_vpn_name(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= 256 && !name.chars().any(|c| c == '"' || c.is_control())
}

/// Text for common rasdial failures (the output itself is localized).
pub fn rasdial_hint(code: Option<i32>) -> &'static str {
    match code {
        Some(691) | Some(703) | Some(718) => "Windows needs your sign-in for this VPN. Connect once from Windows Settings > VPN and save the sign-in, then try again.",
        Some(623) => "Windows cannot find this VPN connection.",
        Some(800) | Some(809) => "The VPN server could not be reached.",
        _ => "Windows could not change the VPN connection.",
    }
}

fn vpn_payload(input: &Value) -> AppResult<(Value, Value)> {
    let name = input.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    let action = input
        .get("action")
        .and_then(Value::as_str)
        .filter(|action| *action == "connect" || *action == "disconnect")
        .ok_or_else(|| err("VALIDATION_ERROR", "VPN action must be connect or disconnect."))?;
    if !valid_vpn_name(&name) {
        return Err(err("VALIDATION_ERROR", "This VPN name cannot be used here. Use Windows Settings."));
    }
    let vpn = vpn_connections()?
        .into_iter()
        .find(|item| field_string(item, "name") == name)
        .ok_or_else(|| err("NOT_FOUND", "The VPN connection no longer exists."))?;
    Ok((json!({"name":name,"action":action}), vpn))
}

#[tauri::command]
pub async fn get_vpn_connections() -> AppResult<Value> {
    blocking(|| Ok(json!({"connections":vpn_connections()?}))).await
}

#[tauri::command]
pub async fn preview_vpn_action(app: AppHandle, input: Value) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        let (payload, vpn) = vpn_payload(&input)?;
        let name = field_string(&payload, "name");
        let mut notices = vec![];
        if field_string(&payload, "action") == "connect" {
            if !field_bool(&vpn, "splitTunneling") {
                notices.push(notice("vpn.allTraffic", json!({"name":name}), "This VPN may send all traffic through the tunnel and change the default route. Proxy/TUN software such as v2rayN or EasyTier can be affected."));
            }
            notices.push(notice("vpn.credentials", json!({}), "Windows uses the sign-in saved for this VPN. The app never asks for or stores VPN passwords."));
        } else {
            notices.push(notice("vpn.disconnect", json!({"name":name}), "Connections that go through this VPN will drop."));
        }
        Ok(json!({"vpn":payload,"connection":vpn,"warnings":notice_texts(&notices),"notices":notices,"confirmationToken":issue_confirmation(&state, "vpn", &payload)?}))
    })
    .await
}

#[tauri::command]
pub async fn apply_vpn_action(app: AppHandle, input: Value) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        let (payload, _) = vpn_payload(input.get("vpn").unwrap_or(&Value::Null))?;
        consume_confirmation(&state, input.get("confirmationToken").and_then(Value::as_str).unwrap_or(""), "vpn", &payload)?;
        let name = field_string(&payload, "name");
        let mut command = hidden_command("rasdial.exe");
        command.arg(&name);
        if field_string(&payload, "action") == "disconnect" {
            command.arg("/disconnect");
        }
        let output = command
            .output()
            .map_err(|e| err_with("COMMAND_FAILED", "rasdial could not start.", json!({"reason":e.to_string()})))?;
        if !output.status.success() {
            let code = output.status.code();
            return Err(err_with(
                "VPN_FAILED",
                rasdial_hint(code),
                json!({"exitCode":code,"output":crate::sanitize(&crate::decode_windows(&output.stdout))}),
            ));
        }
        let message = notice("vpn.done", json!({"name":name,"action":payload["action"]}), format!("VPN \"{name}\" updated."));
        Ok(json!({"message":message["text"],"messageCode":message}))
    })
    .await
}

/// Opens the fixed Windows Settings page for VPN connections.
#[tauri::command]
pub fn open_vpn_settings() -> AppResult<Value> {
    let verb = wide_null(OsStr::new("open"));
    let target = wide_null(OsStr::new("ms-settings:network-vpn"));
    let result = unsafe { ShellExecuteW(ptr::null_mut(), verb.as_ptr(), target.as_ptr(), ptr::null(), ptr::null(), SW_SHOWNORMAL) };
    if result as isize <= 32 {
        return Err(err("COMMAND_FAILED", "Windows Settings could not be opened."));
    }
    Ok(json!({"opened":true}))
}

// ===========================================================================
// Domain-based routes
// ===========================================================================

pub const MAX_DOMAIN_ADDRESSES: usize = 16;
pub const MAX_DOMAIN_ROUTES: usize = 32;

/// A domain name (not an IP literal) that the resolver may look up.
pub fn valid_domain(value: &str) -> Result<String, String> {
    let domain = net_tools::valid_target(value)?.to_lowercase();
    if domain.parse::<IpAddr>().is_ok() || !domain.chars().any(|c| c.is_ascii_alphabetic()) || !domain.contains('.') {
        return Err("Enter a domain name such as example.com.".into());
    }
    Ok(domain)
}

/// Usable IPv4 answers. Any proxy fake-IP answer refuses the whole lookup,
/// because routes to fake addresses would never reach the real site.
pub fn route_addresses(answers: &[IpAddr]) -> Result<Vec<String>, String> {
    if answers.iter().any(|ip| coexistence::is_fake_ip(&ip.to_string())) {
        return Err("Your DNS returns proxy fake-IP answers (v2rayN, mihomo or sing-box fake-IP mode). Domain routes would point at fake addresses, so none were created.".into());
    }
    let mut result = answers
        .iter()
        .filter_map(|ip| match ip {
            IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_unspecified() && !v4.is_multicast() && !v4.is_broadcast() => Some(v4.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    result.sort();
    result.dedup();
    if result.is_empty() {
        return Err("The domain has no usable IPv4 addresses.".into());
    }
    if result.len() > MAX_DOMAIN_ADDRESSES {
        return Err(format!("The domain resolves to more than {MAX_DOMAIN_ADDRESSES} addresses; use a network route instead."));
    }
    Ok(result)
}

pub fn domain_id(domain: &str, interface_index: i64) -> String {
    format!("{domain}#{interface_index}")
}

/// Routes to add and remove for a domain-route request. `addresses` come from
/// the preview's lookup and are not resolved again on apply.
pub fn build_domain_plan(request: &Value, addresses: &[String], cfg: &AppConfig, routes: &[Value]) -> AppResult<Value> {
    let domain = valid_domain(&field_string(request, "domain")).map_err(|message| err("VALIDATION_ERROR", message))?;
    let remove_mode = field_string(request, "mode") == "remove";
    let index = field_i64(request, "interfaceIndex");
    let id = domain_id(&domain, index);
    let existing = cfg.domain_routes.iter().find(|item| field_string(item, "id") == id).cloned();
    if remove_mode && existing.is_none() {
        return Err(err("NOT_FOUND", "The domain route no longer exists."));
    }
    if existing.is_none() && cfg.domain_routes.len() >= MAX_DOMAIN_ROUTES {
        return Err(err("VALIDATION_ERROR", "Too many domain routes."));
    }
    let template = |address: &str, base: &Value| {
        route_payload(&json!({
            "addressFamily":"IPv4",
            "destinationPrefix":format!("{address}/32"),
            "nextHop":field_string(base, "nextHop"),
            "interfaceIndex":field_i64(base, "interfaceIndex"),
            "routeMetric":field_i64(base, "routeMetric"),
            "persistent":false
        }))
    };
    let live = |route: &Value| routes.iter().find(|item| route_key(item) == route_key(route)).cloned();
    let old_routes = match &existing {
        Some(entry) => arrays(entry.get("addresses"))
            .iter()
            .filter_map(Value::as_str)
            .map(|address| template(address, entry))
            .collect::<AppResult<Vec<_>>>()?,
        None => vec![],
    };
    let new_routes = if remove_mode {
        vec![]
    } else {
        let checked = route_addresses(&addresses.iter().filter_map(|text| text.parse().ok()).collect::<Vec<_>>())
            .map_err(|message| err("FAKE_IP_OR_INVALID", message))?;
        if checked.len() != addresses.len() {
            return Err(err("VALIDATION_ERROR", "The address list is invalid. Preview again."));
        }
        checked.iter().map(|address| template(address, request)).collect::<AppResult<Vec<_>>>()?
    };
    let new_keys = new_routes.iter().map(route_key).collect::<Vec<_>>();
    let add = new_routes.iter().filter(|route| live(route).is_none()).cloned().collect::<Vec<_>>();
    let remove = old_routes
        .iter()
        .filter(|route| !new_keys.contains(&route_key(route)))
        .filter_map(|route| live(route).map(|actual| route_remove_payload(&actual)))
        .collect::<Vec<_>>();
    Ok(json!({
        "id":id,
        "mode":if remove_mode { "remove" } else { "save" },
        "domain":domain,
        "nextHop":field_string(request, "nextHop"),
        "interfaceIndex":index,
        "routeMetric":field_i64(request, "routeMetric"),
        "addresses":if remove_mode { json!([]) } else { json!(addresses) },
        "add":add,
        "remove":remove,
    }))
}

fn domain_notices(plan: &Value, overview: &Value, routes: &Value) -> Vec<Value> {
    let mut notices = vec![];
    if field_string(plan, "mode") == "save" {
        notices.push(notice(
            "domain.snapshot",
            json!({"count":arrays(plan.get("addresses")).len().to_string()}),
            "Only the addresses resolved now are routed. Sites behind CDNs change addresses; use Refresh later. These routes are temporary and disappear after a restart.",
        ));
        if let Some(via) = coexistence::default_via(overview, routes) {
            notices.push(notice(
                "domain.tun",
                json!({"alias":field_string(&via, "alias"),"owner":field_string(&via, "owner")}),
                format!(
                    "{} ({}) currently captures default traffic. These more specific routes bypass it for this domain.",
                    field_string(&via, "alias"),
                    field_string(&via, "owner")
                ),
            ));
        }
    }
    notices
}

fn domain_request(input: &Value) -> Value {
    json!({
        "domain":input.get("domain"),
        "mode":input.get("mode").and_then(Value::as_str).unwrap_or("save"),
        "nextHop":input.get("nextHop"),
        "interfaceIndex":input.get("interfaceIndex").and_then(|value| value.as_i64().or_else(|| value.as_str().and_then(|text| text.parse().ok()))),
        "routeMetric":input.get("routeMetric").and_then(|value| value.as_i64().or_else(|| value.as_str().and_then(|text| text.parse().ok()))),
    })
}

#[tauri::command]
pub async fn preview_domain_route(app: AppHandle, input: Value) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        let request = domain_request(&input);
        let cfg = config(&state);
        let addresses = if field_string(&request, "mode") == "remove" {
            vec![]
        } else {
            let domain = valid_domain(&field_string(&request, "domain")).map_err(|message| err("VALIDATION_ERROR", message))?;
            let answers = net_tools::resolve(&domain).map_err(|message| err("DNS_FAILED", message))?;
            route_addresses(&answers).map_err(|message| err("FAKE_IP_OR_INVALID", message))?
        };
        // Use the existing entry's gateway when removing.
        let request = if field_string(&request, "mode") == "remove" {
            let id = domain_id(&valid_domain(&field_string(&request, "domain")).map_err(|m| err("VALIDATION_ERROR", m))?, field_i64(&request, "interfaceIndex"));
            let entry = cfg.domain_routes.iter().find(|item| field_string(item, "id") == id).cloned().unwrap_or(Value::Null);
            let mut merged = request.clone();
            merged["nextHop"] = entry.get("nextHop").cloned().unwrap_or(Value::Null);
            merged["routeMetric"] = entry.get("routeMetric").cloned().unwrap_or(json!(1));
            merged
        } else {
            request
        };
        let snapshot = snapshot_current(&state)?;
        let routes = get_routes()?;
        let plan = build_domain_plan(&request, &addresses, &cfg, &arrays(routes.get("routes")))?;
        let notices = domain_notices(&plan, snapshot.get("overview").unwrap_or(&Value::Null), &routes);
        let unchanged = arrays(plan.get("add")).is_empty() && arrays(plan.get("remove")).is_empty() && field_string(&plan, "mode") == "save"
            && cfg.domain_routes.iter().any(|item| field_string(item, "id") == field_string(&plan, "id"));
        Ok(json!({"plan":plan,"unchanged":unchanged,"warnings":notice_texts(&notices),"notices":notices,"confirmationToken":issue_confirmation(&state, "domain-route", &plan)?,"requiresAdmin":true}))
    })
    .await
}

#[tauri::command]
pub async fn apply_domain_route(app: AppHandle, input: Value) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        let submitted = input.get("plan").cloned().unwrap_or(Value::Null);
        let addresses = arrays(submitted.get("addresses")).iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>();
        let mut cfg = config(&state);
        let routes = get_routes()?;
        let plan = build_domain_plan(&domain_request(&submitted), &addresses, &cfg, &arrays(routes.get("routes")))?;
        consume_confirmation(&state, input.get("confirmationToken").and_then(Value::as_str).unwrap_or(""), "domain-route", &plan)?;
        require_admin()?;
        capture_baseline(&state, json!({"domainRoute":{"domain":plan["domain"],"added":plan["add"],"removed":plan["remove"]}}))?;
        let mut results = vec![];
        for route in arrays(plan.get("remove")) {
            results.extend(arrays(Some(&remove_route_record(&route)?)));
        }
        for route in arrays(plan.get("add")) {
            match add_route_record(&route) {
                Ok(value) => results.push(json!({"status":"success","kind":"route","result":value})),
                Err(error) => results.push(json!({"status":"failed","kind":"route","route":route,"error":error})),
            }
        }
        let removed_keys = arrays(plan.get("remove")).iter().map(route_key).collect::<Vec<_>>();
        cfg.managed_routes.retain(|item| !removed_keys.contains(&route_key(item)));
        for route in arrays(plan.get("add")) {
            if !cfg.managed_routes.iter().any(|item| route_key(item) == route_key(&route)) {
                cfg.managed_routes.push(json!({"addressFamily":"IPv4","destinationPrefix":route["destinationPrefix"],"nextHop":route["nextHop"],"interfaceIndex":route["interfaceIndex"]}));
            }
        }
        let id = field_string(&plan, "id");
        cfg.domain_routes.retain(|item| field_string(item, "id") != id);
        if field_string(&plan, "mode") == "save" {
            cfg.domain_routes.push(json!({
                "id":id,"domain":plan["domain"],"nextHop":plan["nextHop"],"interfaceIndex":plan["interfaceIndex"],
                "routeMetric":plan["routeMetric"],"addresses":plan["addresses"],"updatedAt":Utc::now().to_rfc3339()
            }));
        }
        persist(&state, &cfg)?;
        let domain = field_string(&plan, "domain");
        let message = notice("domain.applied", json!({"domain":domain}), format!("Domain routes for {domain} updated."));
        Ok(json!({"results":results,"message":message["text"],"messageCode":message}))
    })
    .await
}

/// Undo for domain routes: remove what was added, re-add what was removed.
pub fn restore_domain_scope(scope: &Value) -> AppResult<Vec<Value>> {
    let Some(item) = scope.get("domainRoute") else { return Ok(vec![]) };
    let current = arrays(get_routes()?.get("routes"));
    let mut results = vec![];
    for added in arrays(item.get("added")) {
        if let Some(actual) = current.iter().find(|route| route_key(route) == route_key(&added)) {
            results.extend(arrays(Some(&remove_route_record(&route_remove_payload(actual))?)));
        }
    }
    for removed in arrays(item.get("removed")) {
        if !current.iter().any(|route| route_key(route) == route_key(&removed)) {
            results.push(match add_route_record(&removed) {
                Ok(value) => json!({"status":"success","kind":"restore-route","result":value}),
                Err(error) => json!({"status":"failed","kind":"restore-route","error":error}),
            });
        }
    }
    Ok(results)
}

// ===========================================================================
// Hosts file editor
// ===========================================================================

const MAX_HOSTS_BYTES: usize = 1024 * 1024;
const MAX_HOSTS_LINES: usize = 5000;
const HOSTS_BACKUPS: usize = 10;

/// The fixed Windows hosts path.
pub fn hosts_path() -> PathBuf {
    let root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join(r"System32\drivers\etc\hosts")
}

pub fn valid_host_name(name: &str) -> bool {
    name.len() <= 253
        && !name.is_empty()
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
}

fn parse_entry(text: &str) -> Option<(String, Vec<String>, String)> {
    let (body, comment) = match text.split_once('#') {
        Some((body, comment)) => (body, comment.trim().to_string()),
        None => (text, String::new()),
    };
    let mut parts = body.split_whitespace();
    let ip = parts.next()?;
    ip.parse::<IpAddr>().ok()?;
    let names = parts.map(str::to_owned).collect::<Vec<_>>();
    if names.is_empty() || !names.iter().all(|name| valid_host_name(name)) {
        return None;
    }
    Some((ip.to_string(), names, comment))
}

/// Split hosts text into entries (enabled, or disabled with a leading `#`)
/// and raw lines (comments, blanks) kept exactly as written.
pub fn parse_hosts(text: &str) -> Vec<Value> {
    text.lines()
        .map(|line| {
            let trimmed = line.trim();
            if let Some((ip, names, comment)) = parse_entry(trimmed) {
                return json!({"kind":"entry","ip":ip,"names":names,"comment":comment,"enabled":true,"original":line});
            }
            if let Some(rest) = trimmed.strip_prefix('#') {
                if let Some((ip, names, comment)) = parse_entry(rest.trim()) {
                    return json!({"kind":"entry","ip":ip,"names":names,"comment":comment,"enabled":false,"original":line});
                }
            }
            json!({"kind":"raw","text":line})
        })
        .collect()
}

fn entry_line(ip: &str, names: &[String], comment: &str, enabled: bool) -> String {
    let mut line = format!("{}{ip}\t{}", if enabled { "" } else { "# " }, names.join(" "));
    if !comment.is_empty() {
        line.push_str(&format!("\t# {comment}"));
    }
    line
}

/// Validate edited lines and produce the file text. Unchanged entries keep
/// their original formatting.
pub fn serialize_hosts(lines: &[Value]) -> Result<String, String> {
    if lines.len() > MAX_HOSTS_LINES {
        return Err("The hosts file has too many lines.".into());
    }
    let mut output = vec![];
    for line in lines {
        match line.get("kind").and_then(Value::as_str) {
            Some("raw") => {
                let text = field_string_raw(line, "text");
                let trimmed = text.trim();
                if !(trimmed.is_empty() || trimmed.starts_with('#')) || text.len() > 1000 || text.chars().any(|c| c.is_control() && c != '\t') {
                    return Err("Only comments and blank lines can be kept as raw text.".into());
                }
                output.push(text);
            }
            Some("entry") => {
                let ip = field_string(line, "ip");
                let parsed: IpAddr = ip.parse().map_err(|_| format!("\"{ip}\" is not an IP address."))?;
                let names = arrays(line.get("names")).iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>();
                if names.is_empty() || names.len() > 20 || !names.iter().all(|name| valid_host_name(name)) {
                    return Err(format!("The host names for {ip} are invalid (letters, digits, '-', '_' and dots; no wildcards)."));
                }
                let comment = field_string(line, "comment");
                if comment.chars().count() > 200 || comment.chars().any(char::is_control) {
                    return Err("A comment is too long or contains control characters.".into());
                }
                let enabled = line.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                let original = field_string_raw(line, "original");
                let unchanged = parse_hosts(&original).first().map(|item| {
                    item["kind"] == "entry"
                        && item["ip"] == json!(parsed.to_string())
                        && item["names"] == json!(names)
                        && item["comment"] == json!(comment)
                        && item["enabled"] == json!(enabled)
                });
                output.push(if unchanged == Some(true) { original } else { entry_line(&parsed.to_string(), &names, &comment, enabled) });
            }
            _ => return Err("Unknown hosts line.".into()),
        }
    }
    let text = output.join("\r\n") + "\r\n";
    if text.len() > MAX_HOSTS_BYTES {
        return Err("The hosts file is too large.".into());
    }
    Ok(text)
}

fn field_string_raw(value: &Value, name: &str) -> String {
    value.get(name).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn entry_keys(lines: &[Value]) -> Vec<String> {
    lines
        .iter()
        .filter(|line| line["kind"] == "entry")
        .map(|line| {
            let names = arrays(line.get("names")).iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ");
            format!("{}{} {}", if line["enabled"] == json!(false) { "# " } else { "" }, field_string(line, "ip"), names)
        })
        .collect()
}

/// Entries added and removed between two parsed files (for the preview).
pub fn hosts_diff(before: &[Value], after: &[Value]) -> Value {
    let (old, new) = (entry_keys(before), entry_keys(after));
    json!({
        "added":new.iter().filter(|key| !old.contains(key)).collect::<Vec<_>>(),
        "removed":old.iter().filter(|key| !new.contains(key)).collect::<Vec<_>>(),
    })
}

fn read_hosts() -> AppResult<String> {
    let bytes = fs::read(hosts_path()).map_err(|e| err_with("HOSTS_READ_FAILED", "The hosts file cannot be read.", json!({"reason":e.to_string()})))?;
    if bytes.len() > MAX_HOSTS_BYTES {
        return Err(err("HOSTS_READ_FAILED", "The hosts file is too large to edit here."));
    }
    Ok(crate::decode_windows(&bytes).trim_start_matches('\u{feff}').to_string())
}

fn backup_dir(state: &State<AppState>) -> PathBuf {
    state.config_path.parent().map(Path::to_path_buf).unwrap_or_default().join("hosts-backups")
}

#[tauri::command]
pub fn get_hosts_file() -> AppResult<Value> {
    let text = read_hosts()?;
    Ok(json!({"path":hosts_path().display().to_string(),"lines":parse_hosts(&text),"hash":fingerprint(&json!(text)),"isAdmin":is_admin()}))
}

fn hosts_payload(input: &Value) -> AppResult<(Value, String, Vec<Value>)> {
    let lines = arrays(input.get("lines"));
    let text = serialize_hosts(&lines).map_err(|message| err("VALIDATION_ERROR", message))?;
    let base = field_string(input, "baseHash");
    let current = read_hosts()?;
    if fingerprint(&json!(current)) != base {
        return Err(err("CONFLICT", "The hosts file changed since it was loaded. Reload it and make your changes again."));
    }
    let payload = json!({"baseHash":base,"newHash":fingerprint(&json!(text))});
    Ok((payload, text, parse_hosts(&current)))
}

#[tauri::command]
pub fn preview_hosts_change(state: State<AppState>, input: Value) -> AppResult<Value> {
    let (payload, text, before) = hosts_payload(&input)?;
    let diff = hosts_diff(&before, &parse_hosts(&text));
    let notices = vec![
        notice("hosts.effect", json!({}), "Hosts entries override DNS for every app on this PC. A wrong entry can make a site unreachable or send it to another address."),
        notice("hosts.backup", json!({}), "A backup of the current hosts file is saved in the app-data folder first; Undo last app change restores it."),
    ];
    Ok(json!({"change":payload,"diff":diff,"warnings":notice_texts(&notices),"notices":notices,"confirmationToken":issue_confirmation(&state, "hosts", &payload)?,"requiresAdmin":true}))
}

#[tauri::command]
pub fn apply_hosts_change(state: State<AppState>, input: Value) -> AppResult<Value> {
    let (payload, text, _) = hosts_payload(&input)?;
    consume_confirmation(&state, input.get("confirmationToken").and_then(Value::as_str).unwrap_or(""), "hosts", &payload)?;
    require_admin()?;
    let current = read_hosts()?;
    let dir = backup_dir(&state);
    fs::create_dir_all(&dir).map_err(|e| err_with("HOSTS_BACKUP_FAILED", "Cannot create the hosts backup folder.", json!({"reason":e.to_string()})))?;
    let backup_name = format!("hosts-{}.bak", Utc::now().format("%Y%m%d%H%M%S%3f"));
    fs::write(dir.join(&backup_name), &current).map_err(|e| err_with("HOSTS_BACKUP_FAILED", "Cannot back up the hosts file.", json!({"reason":e.to_string()})))?;
    let mut backups = fs::read_dir(&dir)
        .map(|entries| entries.filter_map(Result::ok).map(|entry| entry.file_name().to_string_lossy().to_string()).filter(|name| name.starts_with("hosts-") && name.ends_with(".bak")).collect::<Vec<_>>())
        .unwrap_or_default();
    backups.sort();
    for old in backups.iter().take(backups.len().saturating_sub(HOSTS_BACKUPS)) {
        let _ = fs::remove_file(dir.join(old));
    }
    capture_baseline(&state, json!({"hosts":{"backup":backup_name,"hashAfter":payload["newHash"]}}))?;
    fs::write(hosts_path(), &text).map_err(|e| err_with("HOSTS_WRITE_FAILED", "Windows did not allow writing the hosts file. It may be read-only or locked by security software.", json!({"reason":e.to_string()})))?;
    let message = notice("hosts.applied", json!({}), "The hosts file was updated.");
    Ok(json!({"backup":backup_name,"message":message["text"],"messageCode":message}))
}

/// Undo for the hosts file: restore the backup if the file is unchanged since.
pub fn restore_hosts_scope(state: &State<AppState>, scope: &Value) -> Vec<Value> {
    let Some(item) = scope.get("hosts") else { return vec![] };
    let name = field_string(item, "backup");
    if !name.starts_with("hosts-") || !name.ends_with(".bak") || name.contains(['/', '\\']) {
        return vec![json!({"status":"skipped","kind":"hosts","message":"The recorded hosts backup is invalid."})];
    }
    let current = match read_hosts() {
        Ok(text) => text,
        Err(error) => return vec![json!({"status":"failed","kind":"hosts","error":error})],
    };
    if fingerprint(&json!(current)) != field_string(item, "hashAfter") {
        return vec![json!({"status":"skipped","kind":"hosts","message":"The hosts file changed after the app edited it, so it was not restored."})];
    }
    match fs::read_to_string(backup_dir(state).join(&name)).and_then(|text| fs::write(hosts_path(), text)) {
        Ok(()) => vec![json!({"status":"success","kind":"hosts"})],
        Err(e) => vec![json!({"status":"failed","kind":"hosts","error":e.to_string()})],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    #[test]
    fn triggers_match_enabled_rules_for_the_adapter_and_profile() {
        let trigger = NetworkTrigger { id: "t1".into(), adapter_guid: "255BC7A7-A80C-47E6-902B-386AA6EE90DB".into(), profile_name: "Office".into(), saved_config_id: "cfg".into(), enabled: true };
        let list = vec![trigger.clone(), NetworkTrigger { enabled: false, id: "t2".into(), ..trigger.clone() }];
        assert_eq!(matching_triggers(&list, "{255bc7a7-a80c-47e6-902b-386aa6ee90db}", "Office"), vec![trigger.clone()]);
        assert!(matching_triggers(&list, "{255bc7a7-a80c-47e6-902b-386aa6ee90db}", "Home").is_empty());
        let mut cfg = AppConfig::default();
        assert!(validate_trigger(&trigger, &cfg).is_err(), "the saved configuration must exist");
        cfg.profiles.push(json!({"id":"cfg","name":"Office routes","state":{"overview":{"interfaces":[]}}}));
        assert!(validate_trigger(&trigger, &cfg).is_ok());
        assert!(validate_trigger(&NetworkTrigger { id: "../x".into(), ..trigger.clone() }, &cfg).is_err());
        assert!(cooled_down("unit-test-trigger"));
        assert!(!cooled_down("unit-test-trigger"), "a trigger runs at most once a minute");
    }

    #[test]
    fn vpn_names_and_hints() {
        assert!(valid_vpn_name("Office VPN"));
        assert!(!valid_vpn_name("bad\"name"));
        assert!(!valid_vpn_name(" "));
        assert!(rasdial_hint(Some(691)).contains("sign-in"));
        assert!(rasdial_hint(Some(1)).contains("could not"));
    }

    #[test]
    fn domain_routes_refuse_fake_ip_and_plan_changes() {
        assert_eq!(valid_domain("Example.COM").unwrap(), "example.com");
        assert!(valid_domain(&ip(1, 1, 1, 1).to_string()).is_err());
        assert!(valid_domain("localhost").is_err());
        assert!(valid_domain("a;b.com").is_err());
        assert!(route_addresses(&[ip(93, 184, 216, 34), ip(198, 18, 0, 7)]).unwrap_err().contains("fake-IP"));
        assert_eq!(route_addresses(&[ip(93, 184, 216, 34), ip(93, 184, 216, 34), ip(127, 0, 0, 1)]).unwrap(), vec![ip(93, 184, 216, 34).to_string()]);
        assert!(route_addresses(&[ip(127, 0, 0, 1)]).is_err());
        let gateway = ip(192, 168, 1, 1).to_string();
        let request = json!({"domain":"example.com","mode":"save","nextHop":gateway,"interfaceIndex":23,"routeMetric":5});
        let first = ip(93, 184, 216, 34).to_string();
        let second = ip(93, 184, 216, 35).to_string();
        let plan = build_domain_plan(&request, &[first.clone()], &AppConfig::default(), &[]).unwrap();
        assert_eq!(plan["id"], "example.com#23");
        assert_eq!(plan["add"][0]["destinationPrefix"], format!("{first}/32"));
        assert!(arrays(plan.get("remove")).is_empty());
        let mut cfg = AppConfig::default();
        cfg.domain_routes.push(json!({"id":"example.com#23","domain":"example.com","nextHop":gateway,"interfaceIndex":23,"routeMetric":5,"addresses":[first]}));
        let live = vec![json!({"addressFamily":"IPv4","destinationPrefix":format!("{first}/32"),"nextHop":gateway,"interfaceIndex":23,"routeMetric":5,"persistent":false,"policyStores":["ActiveStore"]})];
        let refresh = build_domain_plan(&request, &[second.clone()], &cfg, &live).unwrap();
        assert_eq!(refresh["add"][0]["destinationPrefix"], format!("{second}/32"));
        assert_eq!(refresh["remove"][0]["destinationPrefix"], format!("{first}/32"));
        let mut remove = request.clone();
        remove["mode"] = json!("remove");
        let removal = build_domain_plan(&remove, &[], &cfg, &live).unwrap();
        assert!(arrays(removal.get("add")).is_empty());
        assert_eq!(arrays(removal.get("remove")).len(), 1);
        assert!(build_domain_plan(&request, &[ip(198, 19, 1, 1).to_string()], &cfg, &live).is_err(), "apply re-checks fake-IP answers");
    }

    #[test]
    fn hosts_files_round_trip_and_validate() {
        let local = ip(127, 0, 0, 1).to_string();
        let lan = ip(192, 168, 1, 10).to_string();
        let text = format!("# Copyright\r\n\r\n{local}\tlocalhost\r\n#  {lan}  nas.lan nas  # storage\r\n::1 localhost\r\n# just a comment\r\n");
        let lines = parse_hosts(&text);
        assert_eq!(lines.iter().filter(|line| line["kind"] == "entry").count(), 3);
        assert_eq!(lines[3]["enabled"], false);
        assert_eq!(lines[3]["names"], json!(["nas.lan", "nas"]));
        assert_eq!(lines[3]["comment"], "storage");
        assert_eq!(serialize_hosts(&lines).unwrap(), text, "unchanged files are written back unchanged");
        let mut edited = lines.clone();
        edited[3]["enabled"] = json!(true);
        edited.push(json!({"kind":"entry","ip":"fe80::1","names":["router.lan"],"comment":"","enabled":true}));
        let output = serialize_hosts(&edited).unwrap();
        assert!(output.contains(&format!("{lan}\tnas.lan nas\t# storage")));
        assert!(output.ends_with("fe80::1\trouter.lan\r\n"));
        let diff = hosts_diff(&lines, &parse_hosts(&output));
        assert_eq!(arrays(diff.get("added")).len(), 2);
        assert_eq!(arrays(diff.get("removed")).len(), 1);
        for bad in [
            json!({"kind":"entry","ip":"not-ip","names":["a"]}),
            json!({"kind":"entry","ip":local,"names":["*.ads.com"]}),
            json!({"kind":"entry","ip":local,"names":[]}),
            json!({"kind":"entry","ip":local,"names":["a"],"comment":"x\ny"}),
            json!({"kind":"raw","text":"rm -rf /"}),
            json!({"kind":"raw","text":"# ok\r\nmalicious"}),
            json!({"kind":"other"}),
        ] {
            assert!(serialize_hosts(&[bad.clone()]).is_err(), "{bad}");
        }
        assert!(hosts_path().ends_with(r"System32\drivers\etc\hosts"));
    }

    #[test]
    #[ignore = "manual, read-only check of VPN connections and the hosts file"]
    fn read_only_vpn_and_hosts() {
        println!("vpn: {:?}", vpn_connections().map(|items| items.len()));
        let text = read_hosts().unwrap();
        let lines = parse_hosts(&text);
        println!("hosts lines={} entries={}", lines.len(), lines.iter().filter(|line| line["kind"] == "entry").count());
        let normalize = |text: &str| text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim_end().to_string();
        assert_eq!(normalize(&serialize_hosts(&lines).unwrap()), normalize(&text), "round trip");
    }
}
