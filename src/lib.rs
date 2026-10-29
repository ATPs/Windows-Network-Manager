use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::Utc;
use encoding_rs::GBK;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsStr,
    fs,
    mem::{size_of, MaybeUninit},
    net::Ipv6Addr,
    os::windows::{ffi::OsStrExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
    ptr,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;
use windows_sys::Win32::UI::{
    Shell::{IsUserAnAdmin, ShellExecuteW},
    WindowsAndMessaging::SW_SHOWNORMAL,
};
use windows_sys::Win32::{
    NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GetIfEntry2, GetIpInterfaceEntry, InitializeIpInterfaceEntry,
        GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_INCLUDE_PREFIX, IP_ADAPTER_ADDRESSES_LH, MIB_IF_ROW2,
        MIB_IPINTERFACE_ROW,
    },
    NetworkManagement::Ndis::{NET_IF_ADMIN_STATUS_DOWN, NET_IF_ADMIN_STATUS_UP},
    Networking::WinSock::{
        AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6, SOCKET_ADDRESS,
    },
};

const CONFIG_FILE: &str = "network-profiles.json";
const CACHE_FILE: &str = "network-state-cache.json";
const DISCOVERY_CACHE_TTL: Duration = Duration::from_secs(5);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PublicError {
    code: String,
    message: String,
    details: Option<Value>,
    recoverable: bool,
}
type AppResult<T> = Result<T, PublicError>;

fn err(code: &str, message: impl Into<String>) -> PublicError {
    PublicError {
        code: code.into(),
        message: message.into(),
        details: None,
        recoverable: true,
    }
}
fn err_with(code: &str, message: impl Into<String>, details: Value) -> PublicError {
    PublicError {
        code: code.into(),
        message: message.into(),
        details: Some(details),
        recoverable: true,
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Settings {
    #[serde(default = "default_language")]
    language: String,
    #[serde(default = "default_theme")]
    theme: String,
    #[serde(default)]
    show_system_interfaces: bool,
    #[serde(default)]
    show_all_routes: bool,
}
fn default_language() -> String {
    "en".into()
}
fn default_theme() -> String {
    "system".into()
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            language: default_language(),
            theme: default_theme(),
            show_system_interfaces: false,
            show_all_routes: false,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    id: String,
    hash: String,
    name: String,
    created_at: String,
    last_used_at: String,
    #[serde(default)]
    use_count: u32,
    state: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Baseline {
    snapshot: Snapshot,
    scope: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AppConfig {
    #[serde(default = "schema_version")]
    schema_version: u32,
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    profiles: Vec<Value>,
    #[serde(default)]
    recent_snapshots: Vec<Snapshot>,
    #[serde(default)]
    baseline: Option<Baseline>,
    #[serde(default)]
    managed_routes: Vec<Value>,
    #[serde(default)]
    route_notes: BTreeMap<String, String>,
}
fn schema_version() -> u32 {
    3
}
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: 3,
            settings: Settings::default(),
            profiles: vec![],
            recent_snapshots: vec![],
            baseline: None,
            managed_routes: vec![],
            route_notes: BTreeMap::new(),
        }
    }
}

struct Confirmation {
    kind: String,
    hash: String,
    expires_at: Instant,
}
struct DiscoveryCache {
    snapshot: Value,
    refreshed_at: Instant,
}
struct AppState {
    config_path: PathBuf,
    cache_path: PathBuf,
    confirmations: Mutex<HashMap<String, Confirmation>>,
    discovery: Mutex<Option<DiscoveryCache>>,
}

const OVERVIEW_SCRIPT: &str = r#"
$Adapters = @(Get-NetAdapter -IncludeHidden | ForEach-Object { [PSCustomObject]@{ interfaceIndex=[int]$_.InterfaceIndex; alias=[string]$_.Name; description=[string]$_.InterfaceDescription; guid=[string]$_.InterfaceGuid; macAddress=[string]$_.MacAddress; interfaceType=[int]$_.InterfaceType; hardwareInterface=[bool]$_.HardwareInterface; virtual=[bool]$_.Virtual; adminStatus=[string]$_.AdminStatus; operationalStatus=[string]$_.Status; mediaConnectionState=[string]$_.MediaConnectionState; linkSpeed=[string]$_.LinkSpeed } })
$IpInterfaces = @(Get-NetIPInterface | ForEach-Object { [PSCustomObject]@{ interfaceIndex=[int]$_.InterfaceIndex; alias=[string]$_.InterfaceAlias; addressFamily=[string]$_.AddressFamily; automaticMetric=([string]$_.AutomaticMetric -eq 'Enabled'); interfaceMetric=[long]$_.InterfaceMetric; connectionState=[string]$_.ConnectionState; dhcp=([string]$_.Dhcp -eq 'Enabled') } })
  $IpConfigurations = @(Get-NetRoute -DestinationPrefix 0.0.0.0/0,::/0 -ErrorAction SilentlyContinue | Group-Object InterfaceIndex | ForEach-Object { [PSCustomObject]@{ interfaceIndex=[int]$_.Name; alias=[string]$_.Group[0].InterfaceAlias; ipv4Gateways=@($_.Group | Where-Object { [string]$_.AddressFamily -eq 'IPv4' } | ForEach-Object { [string]$_.NextHop }); ipv6Gateways=@($_.Group | Where-Object { [string]$_.AddressFamily -eq 'IPv6' } | ForEach-Object { [string]$_.NextHop }) } })
$Addresses = @(Get-NetIPAddress | ForEach-Object { [PSCustomObject]@{ interfaceIndex=[int]$_.InterfaceIndex; addressFamily=[string]$_.AddressFamily; ipAddress=[string]$_.IPAddress; prefixLength=[int]$_.PrefixLength; addressState=[string]$_.AddressState; type=[string]$_.Type } })
$Dns = @(Get-DnsClientServerAddress | ForEach-Object { [PSCustomObject]@{ interfaceIndex=[int]$_.InterfaceIndex; addressFamily=[string]$_.AddressFamily; servers=@($_.ServerAddresses | ForEach-Object { [string]$_ }) } })
$Profiles = @(Get-NetConnectionProfile -ErrorAction SilentlyContinue | ForEach-Object { [PSCustomObject]@{ interfaceIndex=[int]$_.InterfaceIndex; name=[string]$_.Name; category=[string]$_.NetworkCategory } })
[PSCustomObject]@{ adapters=$Adapters; ipInterfaces=$IpInterfaces; ipConfigurations=$IpConfigurations; addresses=$Addresses; dns=$Dns; connectionProfiles=$Profiles } | ConvertTo-Json -Compress -Depth 6
"#;

const ROUTES_SCRIPT: &str = r#"
function Convert-Route($Route, $Store) { [PSCustomObject]@{ addressFamily=[string]$Route.AddressFamily; destinationPrefix=[string]$Route.DestinationPrefix; nextHop=[string]$Route.NextHop; interfaceAlias=[string]$Route.InterfaceAlias; interfaceIndex=[int]$Route.InterfaceIndex; routeMetric=[long]$Route.RouteMetric; protocol=[string]$Route.Protocol; policyStore=$Store; state=[string]$Route.State; publish=[string]$Route.Publish; validLifetime=[string]$Route.ValidLifetime; preferredLifetime=[string]$Route.PreferredLifetime } }
$Active=@(Get-NetRoute -PolicyStore ActiveStore | ForEach-Object { Convert-Route $_ 'ActiveStore' })
$Persistent=@(Get-NetRoute -PolicyStore PersistentStore | ForEach-Object { Convert-Route $_ 'PersistentStore' })
$Metrics=@(Get-NetIPInterface | ForEach-Object { [PSCustomObject]@{ addressFamily=[string]$_.AddressFamily; interfaceIndex=[int]$_.InterfaceIndex; interfaceMetric=[long]$_.InterfaceMetric } })
[PSCustomObject]@{active=$Active;persistent=$Persistent;metrics=$Metrics}|ConvertTo-Json -Compress -Depth 5
"#;

// New-NetRoute defaults to both stores. Explicit PersistentStore is unsupported.
const ADD_ROUTE_SCRIPT: &str = r#"
$p=@{
    AddressFamily='IPv4'
    DestinationPrefix=[string]$InputData.destinationPrefix
    NextHop=[string]$InputData.nextHop
    InterfaceIndex=[int]$InputData.interfaceIndex
    RouteMetric=[int]$InputData.routeMetric
    Confirm=$false
    ErrorAction='Stop'
}
$s='PersistentStore'
if(-not $InputData.persistent){$p.PolicyStore='ActiveStore';$s='ActiveStore'}
New-NetRoute @p | Out-Null
[PSCustomObject]@{created=$true;policyStore=$s}|ConvertTo-Json -Compress
"#;

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
            fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
            let config_path = config_dir.join(CONFIG_FILE);
            let cache_path = config_dir.join(CACHE_FILE);
            app.manage(AppState {
                config_path,
                cache_path,
                confirmations: Mutex::new(HashMap::new()),
                discovery: Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_wifi_profiles,
            get_app_config,
            get_bootstrap_state,
            refresh_network_state,
            update_settings,
            save_profile,
            delete_profile,
            restore_profile_preview,
            restore_profile_apply,
            preview_metric_change,
            apply_metric_change,
            preview_add_route,
            add_route,
            preview_edit_route,
            edit_route,
            preview_persist_route,
            persist_route,
            preview_remove_route,
            remove_route,
            preview_adapter_control,
            apply_adapter_control,
            preview_wifi_change,
            connect_wifi,
            disconnect_wifi,
            capture_recent_snapshot,
            restore_recent_preview,
            restore_recent_apply,
            reset_preview,
            reset_apply,
            windows_defaults_preview,
            windows_defaults_apply,
            get_diagnostics,
            restart_as_administrator
        ])
        .run(tauri::generate_context!())
        .expect("error while running Windows Network Manager");
}

fn hidden_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

fn wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn run_powershell_json(script: &str, input: Option<&Value>) -> AppResult<Value> {
    let prefix = "$ErrorActionPreference='Stop';[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false);$OutputEncoding=[Console]::OutputEncoding;$InputData=$null;if($env:WNM_INPUT_BASE64){$InputData=([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($env:WNM_INPUT_BASE64))|ConvertFrom-Json);}";
    let encoded = BASE64.encode(
        format!("{}\ntry {{\n{}\n}} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}", prefix, script)
            .encode_utf16()
            .flat_map(|x| x.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    let mut command = hidden_command("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-EncodedCommand",
        &encoded,
    ]);
    if let Some(payload) = input {
        command.env(
            "WNM_INPUT_BASE64",
            BASE64.encode(
                serde_json::to_vec(payload)
                    .map_err(|_| err("SERIALIZE_ERROR", "Cannot serialize command input."))?,
            ),
        );
    }
    let output = command.output().map_err(|e| {
        err_with(
            "COMMAND_FAILED",
            "PowerShell could not start.",
            json!({"reason":e.to_string()}),
        )
    })?;
    if !output.status.success() {
        let reason = sanitize(&String::from_utf8_lossy(&output.stderr));
        return Err(err_with(
            "COMMAND_FAILED",
            if reason.is_empty() {
                "Windows command failed.".to_string()
            } else {
                format!("Windows command failed: {reason}")
            },
            json!({"stderr":reason,"exitCode":output.status.code()}),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout)
        .trim_start_matches('\u{feff}')
        .trim()
        .to_string();
    if text.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|_| {
        err_with(
            "PARSE_ERROR",
            "Windows returned invalid JSON.",
            json!({"output":sanitize(&text)}),
        )
    })
}

fn run_netsh(args: &[String]) -> AppResult<String> {
    let output = hidden_command("netsh.exe")
        .args(args)
        .output()
        .map_err(|e| {
            err_with(
                "COMMAND_FAILED",
                "netsh could not start.",
                json!({"reason":e.to_string()}),
            )
        })?;
    let stdout = decode_windows(&output.stdout);
    if !output.status.success() {
        return Err(err_with(
            "COMMAND_FAILED",
            "netsh command failed.",
            json!({"stderr":sanitize(&decode_windows(&output.stderr)),"stdout":sanitize(&stdout)}),
        ));
    }
    Ok(stdout)
}

fn decode_windows(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap_or_else(|_| {
        let (decoded, _, _) = GBK.decode(bytes);
        decoded.into_owned()
    })
}
fn sanitize(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(2000)
        .collect::<String>()
        .trim()
        .into()
}
fn arrays(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::Null) | None => vec![],
        Some(item) => vec![item.clone()],
    }
}
fn field_string(value: &Value, name: &str) -> String {
    value
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}
fn field_i64(value: &Value, name: &str) -> i64 {
    value
        .get(name)
        .and_then(Value::as_i64)
        .or_else(|| value.get(name).and_then(Value::as_u64).map(|x| x as i64))
        .unwrap_or_default()
}
fn field_bool(value: &Value, name: &str) -> bool {
    value.get(name).and_then(Value::as_bool).unwrap_or(false)
}

fn normalize_overview(raw: Value, wifi: Vec<Value>) -> Value {
    let mut entries: BTreeMap<i64, Map<String, Value>> = BTreeMap::new();
    for adapter in arrays(raw.get("adapters")) {
        let index = field_i64(&adapter, "interfaceIndex");
        if index > 0 {
            entries
                .entry(index)
                .or_default()
                .insert("adapter".into(), adapter);
        }
    }
    for ip in arrays(raw.get("ipInterfaces")) {
        let index = field_i64(&ip, "interfaceIndex");
        if index > 0 {
            let entry = entries.entry(index).or_default();
            let family = field_string(&ip, "addressFamily");
            entry
                .entry("ipInterfaces")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .unwrap()
                .insert(family, ip);
        }
    }
    for item in arrays(raw.get("ipConfigurations")) {
        let index = field_i64(&item, "interfaceIndex");
        if index > 0 {
            entries
                .entry(index)
                .or_default()
                .insert("ipConfiguration".into(), item);
        }
    }
    for item in arrays(raw.get("addresses")) {
        let index = field_i64(&item, "interfaceIndex");
        if index > 0 {
            let entry = entries.entry(index).or_default();
            entry
                .entry("addresses")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .unwrap()
                .push(item);
        }
    }
    for item in arrays(raw.get("dns")) {
        let index = field_i64(&item, "interfaceIndex");
        if index > 0 {
            let entry = entries.entry(index).or_default();
            entry
                .entry("dns")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .unwrap()
                .push(item);
        }
    }
    for item in arrays(raw.get("connectionProfiles")) {
        let index = field_i64(&item, "interfaceIndex");
        if index > 0 {
            entries
                .entry(index)
                .or_default()
                .insert("connectionProfile".into(), item);
        }
    }
    let wifi_by_alias: HashMap<String, Value> = wifi
        .into_iter()
        .filter_map(|item| Some((field_string(&item, "alias"), item)))
        .collect();
    let interfaces: Vec<Value> = entries.into_iter().map(|(index, entry)| {
    let adapter=entry.get("adapter").cloned().unwrap_or_else(||json!({})); let ip=entry.get("ipInterfaces").and_then(Value::as_object).cloned().unwrap_or_default(); let config=entry.get("ipConfiguration").cloned().unwrap_or_else(||json!({}));
    let alias={let a=field_string(&adapter,"alias"); if a.is_empty(){ip.values().next().map(|v|field_string(v,"alias")).unwrap_or_else(||format!("Interface {index}"))}else{a}};
    let wifi_item=wifi_by_alias.get(&alias).cloned(); let interface_type=field_i64(&adapter,"interfaceType"); let description=field_string(&adapter,"description"); let text=format!("{} {}",alias,description).to_lowercase();
    let kind=if interface_type==24||text.contains("loopback"){"Loopback"}else if wifi_item.is_some()||interface_type==71{"Wi-Fi"}else if text.contains("tailscale")||text.contains("wireguard")||text.contains("zerotier")||text.contains("vpn"){"VPN"}else if [53,131].contains(&interface_type)||text.contains("tunnel")||text.contains("sing-tun"){"Tunnel"}else if field_bool(&adapter,"virtual")||text.contains("hyper-v")||text.contains("vmware")||text.contains("virtualbox")||text.contains("easytier"){"Virtual"}else if interface_type==6||field_bool(&adapter,"hardwareInterface"){"Ethernet"}else{"Other"};
    let system=kind=="Loopback"||text.contains("wan miniport")||text.contains("wi-fi direct virtual")||text.contains("ip-https platform")||text.contains("isatap")||text.contains("teredo")||alias.starts_with("本地连接*");
    let addresses=arrays(entry.get("addresses")); let ipv4=addresses.iter().filter(|x|field_string(x,"addressFamily")=="IPv4").cloned().collect::<Vec<_>>(); let ipv6=addresses.iter().filter(|x|field_string(x,"addressFamily")=="IPv6").cloned().collect::<Vec<_>>();
    let ipv4_metric=ip.get("IPv4").cloned(); let ipv6_metric=ip.get("IPv6").cloned(); let dns=arrays(entry.get("dns")).into_iter().flat_map(|x|arrays(x.get("servers"))).filter_map(|x|x.as_str().map(str::to_owned)).collect::<Vec<_>>();
    let ipv4_gw=arrays(config.get("ipv4Gateways")); let ipv6_gw=arrays(config.get("ipv6Gateways"));
    json!({"interfaceIndex":index,"alias":alias,"description":description,"guid":field_string(&adapter,"guid").trim_matches(|c|c=='{'||c=='}'),"macAddress":field_string(&adapter,"macAddress"),"type":kind,"hardwareInterface":field_bool(&adapter,"hardwareInterface"),"virtual":field_bool(&adapter,"virtual")||["Virtual","VPN","Tunnel"].contains(&kind),"systemInterface":system,"adminStatus":field_string(&adapter,"adminStatus"),"operationalStatus":field_string(&adapter,"operationalStatus"),"mediaConnectionState":field_string(&adapter,"mediaConnectionState"),"linkSpeed":field_string(&adapter,"linkSpeed"),"ipv4Addresses":ipv4,"ipv6Addresses":ipv6,"ipv4Gateways":ipv4_gw,"ipv6Gateways":ipv6_gw,"dnsServers":dns,"dhcpEnabled":ipv4_metric.as_ref().map(|x|field_bool(x,"dhcp")),"metrics":{"ipv4":ipv4_metric,"ipv6":ipv6_metric},"networkCategory":entry.get("connectionProfile").map(|x|field_string(x,"category")),"defaultRoute":!arrays(config.get("ipv4Gateways")).is_empty()||!arrays(config.get("ipv6Gateways")).is_empty(),"wifi":wifi_item,"deviations":{"manualIpv4Metric":ipv4_metric.as_ref().map(|x|!field_bool(x,"automaticMetric")).unwrap_or(false),"manualIpv6Metric":ipv6_metric.as_ref().map(|x|!field_bool(x,"automaticMetric")).unwrap_or(false),"adapterDisabled":field_string(&adapter,"adminStatus").eq_ignore_ascii_case("down")}})
  }).collect();
    json!({"interfaces":interfaces,"refreshedAt":Utc::now().to_rfc3339()})
}

fn wide_text(value: *const u16) -> String {
    if value.is_null() {
        return String::new();
    }
    unsafe {
        let mut length = 0usize;
        while length < 1024 && *value.add(length) != 0 {
            length += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(value, length))
    }
}
fn native_socket_text(address: SOCKET_ADDRESS) -> Option<(String, String)> {
    if address.lpSockaddr.is_null() {
        return None;
    }
    unsafe {
        match (*address.lpSockaddr).sa_family {
            AF_INET => {
                let item = &*address.lpSockaddr.cast::<SOCKADDR_IN>();
                let bytes = item.sin_addr.S_un.S_un_b;
                Some((
                    "IPv4".into(),
                    format!(
                        "{}.{}.{}.{}",
                        bytes.s_b1, bytes.s_b2, bytes.s_b3, bytes.s_b4
                    ),
                ))
            }
            AF_INET6 => {
                let item = &*address.lpSockaddr.cast::<SOCKADDR_IN6>();
                let bytes = item.sin6_addr.u.Byte;
                Some(("IPv6".into(), Ipv6Addr::from(bytes).to_string()))
            }
            _ => None,
        }
    }
}
fn native_link_speed(bits: u64) -> String {
    if bits == 0 {
        "—".into()
    } else {
        format!("{} Mbps", bits / 1_000_000)
    }
}
fn native_metric(index: u32, family: u16) -> Value {
    let mut row = MIB_IPINTERFACE_ROW::default();
    unsafe {
        InitializeIpInterfaceEntry(&mut row);
        row.InterfaceIndex = index;
        row.Family = family;
        if GetIpInterfaceEntry(&mut row) == 0 {
            return json!({"interfaceIndex":index,"addressFamily":if family==AF_INET{"IPv4"}else{"IPv6"},"automaticMetric":row.UseAutomaticMetric,"interfaceMetric":row.Metric,"dhcp":Value::Null});
        }
    }
    Value::Null
}
fn native_admin_status(index: u32) -> String {
    let mut row = MIB_IF_ROW2::default();
    row.InterfaceIndex = index;
    unsafe {
        if GetIfEntry2(&mut row) == 0 {
            return if row.AdminStatus == NET_IF_ADMIN_STATUS_DOWN {
                "Down".into()
            } else if row.AdminStatus == NET_IF_ADMIN_STATUS_UP {
                "Up".into()
            } else {
                "Unknown".into()
            };
        }
    }
    "Unknown".into()
}
fn native_network_overview() -> AppResult<Value> {
    let mut bytes = 15_000u32;
    for _ in 0..3 {
        let count = (bytes as usize / size_of::<IP_ADAPTER_ADDRESSES_LH>()).max(1) + 1;
        let mut buffer: Vec<MaybeUninit<IP_ADAPTER_ADDRESSES_LH>> = Vec::with_capacity(count);
        unsafe {
            buffer.set_len(count);
        }
        let result = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC as u32,
                GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_INCLUDE_PREFIX,
                ptr::null(),
                buffer.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if result == 111 {
            continue;
        }
        if result != 0 {
            return Err(err_with(
                "NATIVE_DISCOVERY_FAILED",
                "Windows networking APIs could not read adapters.",
                json!({"code":result}),
            ));
        }
        let wifi = wifi_overview().unwrap_or_default();
        let wifi_by_alias: HashMap<String, Value> = wifi
            .into_iter()
            .map(|item| (field_string(&item, "alias"), item))
            .collect();
        let mut interfaces = vec![];
        let mut current = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        unsafe {
            while !current.is_null() {
                let item = &*current;
                let alias = wide_text(item.FriendlyName);
                let description = wide_text(item.Description);
                let interface_type = item.IfType;
                let text = format!("{} {}", alias, description).to_lowercase();
                let wifi_item = wifi_by_alias.get(&alias).cloned();
                let kind = if interface_type == 24 || text.contains("loopback") {
                    "Loopback"
                } else if wifi_item.is_some() || interface_type == 71 {
                    "Wi-Fi"
                } else if text.contains("tailscale")
                    || text.contains("wireguard")
                    || text.contains("zerotier")
                    || text.contains("vpn")
                {
                    "VPN"
                } else if [53, 131].contains(&interface_type) || text.contains("tunnel") {
                    "Tunnel"
                } else if interface_type == 6 {
                    "Ethernet"
                } else {
                    "Other"
                };
                let system = kind == "Loopback"
                    || text.contains("wan miniport")
                    || text.contains("wi-fi direct virtual")
                    || text.contains("isatap")
                    || text.contains("teredo")
                    || alias.starts_with("本地连接*");
                let mut ipv4 = vec![];
                let mut ipv6 = vec![];
                let mut address = item.FirstUnicastAddress;
                while !address.is_null() {
                    let node = &*address;
                    if let Some((family, ip)) = native_socket_text(node.Address) {
                        let entry = json!({"ipAddress":ip,"prefixLength":node.OnLinkPrefixLength,"addressState":"Preferred","type":"Unicast"});
                        if family == "IPv4" {
                            ipv4.push(entry)
                        } else {
                            ipv6.push(entry)
                        }
                    }
                    address = node.Next;
                }
                let mut gateways4 = vec![];
                let mut gateways6 = vec![];
                let mut gateway = item.FirstGatewayAddress;
                while !gateway.is_null() {
                    let node = &*gateway;
                    if let Some((family, ip)) = native_socket_text(node.Address) {
                        if family == "IPv4" {
                            gateways4.push(Value::String(ip))
                        } else {
                            gateways6.push(Value::String(ip))
                        }
                    }
                    gateway = node.Next;
                }
                let mut dns = vec![];
                let mut dns_node = item.FirstDnsServerAddress;
                while !dns_node.is_null() {
                    let node = &*dns_node;
                    if let Some((_, ip)) = native_socket_text(node.Address) {
                        dns.push(Value::String(ip))
                    }
                    dns_node = node.Next;
                }
                let mac = item.PhysicalAddress[..(item
                    .PhysicalAddressLength
                    .min(item.PhysicalAddress.len() as u32)
                    as usize)]
                    .iter()
                    .map(|byte| format!("{byte:02X}"))
                    .collect::<Vec<_>>()
                    .join("-");
                let index = item.Anonymous1.Anonymous.IfIndex.max(item.Ipv6IfIndex);
                let ipv4_metric = native_metric(index, AF_INET);
                let ipv6_metric = native_metric(index, AF_INET6);
                let admin_status = native_admin_status(index);
                let adapter_disabled = admin_status.eq_ignore_ascii_case("down");
                let manual_ipv4 = ipv4_metric
                    .get("automaticMetric")
                    .and_then(Value::as_bool)
                    .map(|automatic| !automatic)
                    .unwrap_or(false);
                let manual_ipv6 = ipv6_metric
                    .get("automaticMetric")
                    .and_then(Value::as_bool)
                    .map(|automatic| !automatic)
                    .unwrap_or(false);
                interfaces.push(json!({"interfaceIndex":index,"alias":alias,"description":description,"guid":"","macAddress":mac,"type":kind,"hardwareInterface":interface_type==6,"virtual":(["VPN","Tunnel"].contains(&kind)),"systemInterface":system,"adminStatus":admin_status,"operationalStatus":item.OperStatus,"mediaConnectionState":"unknown","linkSpeed":native_link_speed(item.TransmitLinkSpeed.max(item.ReceiveLinkSpeed)),"ipv4Addresses":ipv4,"ipv6Addresses":ipv6,"ipv4Gateways":gateways4,"ipv6Gateways":gateways6,"dnsServers":dns,"dhcpEnabled":Value::Null,"metrics":{"ipv4":ipv4_metric,"ipv6":ipv6_metric},"networkCategory":Value::Null,"defaultRoute":!gateways4.is_empty()||!gateways6.is_empty(),"wifi":wifi_item,"deviations":{"manualIpv4Metric":manual_ipv4,"manualIpv6Metric":manual_ipv6,"adapterDisabled":adapter_disabled}}));
                current = item.Next;
            }
        }
        return Ok(json!({"interfaces":interfaces,"refreshedAt":Utc::now().to_rfc3339()}));
    }
    Err(err(
        "NATIVE_DISCOVERY_FAILED",
        "Windows networking APIs requested an unexpectedly large adapter buffer.",
    ))
}

fn parse_wifi_interfaces(output: &str) -> Vec<Value> {
    let mut result = vec![];
    let mut current: Map<String, Value> = Map::new();
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':').or_else(|| line.split_once('：')) else {
            continue;
        };
        let key = key.trim().to_lowercase();
        let value = value.trim();
        let name = match key.as_str() {
            "name" | "名称" => Some("alias"),
            "description" | "描述" | "说明" => Some("description"),
            "guid" => Some("guid"),
            "state" | "状态" => Some("state"),
            "ssid" => Some("ssid"),
            "signal" | "信号" => Some("signal"),
            "profile" | "配置文件" => Some("currentProfile"),
            "radio type" | "无线电类型" => Some("radioType"),
            _ => None,
        };
        if let Some(name) = name {
            if name == "alias" && !current.is_empty() {
                let connected = field_string(&Value::Object(current.clone()), "state")
                    .to_lowercase()
                    == "connected"
                    || field_string(&Value::Object(current.clone()), "state") == "已连接";
                current.insert("connected".into(), json!(connected));
                result.push(Value::Object(current));
                current = Map::new();
            }
            current.insert(name.into(), json!(value));
        }
    }
    if !current.is_empty() {
        let connected = field_string(&Value::Object(current.clone()), "state").to_lowercase()
            == "connected"
            || field_string(&Value::Object(current.clone()), "state") == "已连接";
        current.insert("connected".into(), json!(connected));
        result.push(Value::Object(current));
    }
    result
}

fn parse_wifi_profiles(output: &str) -> Vec<String> {
    let mut profiles = vec![];
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':').or_else(|| line.split_once('：')) else {
            continue;
        };
        let lower = key.to_lowercase();
        if lower.contains("profile") || key.contains("配置文件") {
            let value = value.trim().to_string();
            if !value.is_empty() && !profiles.contains(&value) {
                profiles.push(value)
            }
        }
    }
    profiles
}

fn wifi_overview() -> AppResult<Vec<Value>> {
    let output = run_netsh(&["wlan".into(), "show".into(), "interfaces".into()])?;
    let mut interfaces = parse_wifi_interfaces(&output);
    for item in &mut interfaces {
        item.as_object_mut()
            .unwrap()
            .insert("profiles".into(), json!([]));
        item.as_object_mut()
            .unwrap()
            .insert("profilesLoaded".into(), json!(false));
    }
    Ok(interfaces)
}
fn valid_interface_name(value: &str) -> AppResult<String> {
    let name = value.trim();
    if name.is_empty()
        || name.len() > 256
        || name
            .chars()
            .any(|c| c == '\0' || c == '\r' || c == '\n' || c.is_control())
    {
        Err(err("VALIDATION_ERROR", "Invalid Wi-Fi interface name."))
    } else {
        Ok(name.to_string())
    }
}
fn valid_theme(value: &str) -> AppResult<&str> {
    if ["system", "light", "dark"].contains(&value) {
        Ok(value)
    } else {
        Err(err("VALIDATION_ERROR", "Unsupported theme."))
    }
}

fn config_from_path(path: &Path) -> AppConfig {
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<AppConfig>(&text) {
            Ok(config) => config,
            Err(_) => {
                let backup = path.with_extension(format!(
                    "corrupt-{}.json",
                    Utc::now().format("%Y%m%d%H%M%S")
                ));
                let _ = fs::rename(path, backup);
                AppConfig::default()
            }
        },
        Err(_) => AppConfig::default(),
    }
}
fn save_config(path: &Path, config: &AppConfig) -> AppResult<()> {
    let text = serde_json::to_string_pretty(config)
        .map_err(|_| err("SERIALIZE_ERROR", "Cannot save configuration."))?;
    let temp = path.with_extension("tmp");
    fs::write(&temp, format!("{text}\n")).map_err(|e| {
        err_with(
            "CONFIG_WRITE_FAILED",
            "Cannot write configuration.",
            json!({"reason":e.to_string()}),
        )
    })?;
    fs::rename(temp, path).map_err(|e| {
        err_with(
            "CONFIG_WRITE_FAILED",
            "Cannot replace configuration.",
            json!({"reason":e.to_string()}),
        )
    })
}
fn config(state: &State<AppState>) -> AppConfig {
    config_from_path(&state.config_path)
}
fn persist(state: &State<AppState>, config: &AppConfig) -> AppResult<()> {
    save_config(&state.config_path, config)
}
fn disk_cache(path: &Path) -> Option<Value> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(|value| value.get("overview").is_some() && value.get("routes").is_some())
}
fn persist_cache(path: &Path, snapshot: &Value) {
    let temp = path.with_extension("tmp");
    if let Ok(text) = serde_json::to_string(snapshot) {
        if fs::write(&temp, text).is_ok() {
            let _ = fs::rename(&temp, path);
        }
    }
}
fn live_snapshot() -> AppResult<Value> {
    let routes = get_routes()?;
    let (overview, source) = match native_network_overview() {
        Ok(overview) => (overview, "native"),
        Err(_) => (get_network_overview()?, "powershell-fallback"),
    };
    Ok(
        json!({"overview":overview,"routes":routes,"refreshedAt":Utc::now().to_rfc3339(),"source":source,"stale":false}),
    )
}
fn current_snapshot(state: &State<AppState>) -> AppResult<Value> {
    let snapshot = live_snapshot()?;
    if let Ok(mut cache) = state.discovery.lock() {
        *cache = Some(DiscoveryCache {
            snapshot: snapshot.clone(),
            refreshed_at: Instant::now(),
        });
    }
    persist_cache(&state.cache_path, &snapshot);
    Ok(snapshot)
}
fn recent_snapshot(state: &State<AppState>) -> Option<Value> {
    state.discovery.lock().ok().and_then(|cache| {
        cache
            .as_ref()
            .filter(|item| item.refreshed_at.elapsed() <= DISCOVERY_CACHE_TTL)
            .map(|item| item.snapshot.clone())
    })
}

fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|k| format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap(),
                        canonical(&map[k])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        _ => value.to_string(),
    }
}
fn fingerprint(value: &Value) -> String {
    format!("{:x}", Sha256::digest(canonical(value).as_bytes()))
}
fn issue_confirmation(state: &State<AppState>, kind: &str, payload: &Value) -> AppResult<String> {
    let token = Uuid::new_v4().to_string();
    state
        .confirmations
        .lock()
        .map_err(|_| err("STATE_ERROR", "Cannot create confirmation."))?
        .insert(
            token.clone(),
            Confirmation {
                kind: kind.into(),
                hash: fingerprint(payload),
                expires_at: Instant::now() + Duration::from_secs(120),
            },
        );
    Ok(token)
}
fn consume_confirmation(
    state: &State<AppState>,
    token: &str,
    kind: &str,
    payload: &Value,
) -> AppResult<()> {
    let record = state
        .confirmations
        .lock()
        .map_err(|_| err("STATE_ERROR", "Cannot validate confirmation."))?
        .remove(token);
    match record {
        Some(item)
            if item.kind == kind
                && item.hash == fingerprint(payload)
                && item.expires_at > Instant::now() =>
        {
            Ok(())
        }
        _ => Err(err(
            "CONFIRMATION_EXPIRED",
            "Confirmation expired. Preview the operation again.",
        )),
    }
}
fn is_admin() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}
fn require_admin() -> AppResult<()> {
    if is_admin() {
        Ok(())
    } else {
        Err(err(
            "PERMISSION_REQUIRED",
            "This operation requires administrator privileges.",
        ))
    }
}
fn valid_index(value: &Value) -> AppResult<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|x| x.parse().ok()))
        .filter(|x| *x > 0)
        .ok_or_else(|| {
            err(
                "VALIDATION_ERROR",
                "InterfaceIndex must be a positive integer.",
            )
        })
}
fn valid_metric(value: &Value) -> AppResult<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|x| x.parse().ok()))
        .filter(|x| *x >= 1 && *x <= 9999)
        .ok_or_else(|| {
            err(
                "VALIDATION_ERROR",
                "Metric must be an integer from 1 to 9999.",
            )
        })
}
fn valid_ipv4(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 4
        && parts
            .iter()
            .all(|x| !x.is_empty() && x.parse::<u8>().is_ok())
}
fn normalize_cidr(value: &str) -> AppResult<String> {
    let (mut ip, prefix) = value
        .split_once('/')
        .ok_or_else(|| err("VALIDATION_ERROR", "DestinationPrefix must be IPv4 CIDR."))?;
    ip = ip.trim();
    let prefix: u32 = prefix
        .parse()
        .map_err(|_| err("VALIDATION_ERROR", "Invalid CIDR prefix."))?;
    if prefix > 32 || !valid_ipv4(ip) {
        return Err(err("VALIDATION_ERROR", "Invalid IPv4 CIDR."));
    }
    let nums = ip
        .split('.')
        .map(|x| x.parse::<u32>().unwrap())
        .collect::<Vec<_>>();
    let number = (nums[0] << 24) | (nums[1] << 16) | (nums[2] << 8) | nums[3];
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    let n = number & mask;
    Ok(format!(
        "{}.{}.{}.{}/{}",
        n >> 24,
        (n >> 16) & 255,
        (n >> 8) & 255,
        n & 255,
        prefix
    ))
}

fn get_network_overview() -> AppResult<Value> {
    let raw = run_powershell_json(OVERVIEW_SCRIPT, None)?;
    let wifi = wifi_overview().unwrap_or_default();
    Ok(normalize_overview(raw, wifi))
}
#[tauri::command]
fn get_wifi_profiles(interface_name: String) -> AppResult<Value> {
    let alias = valid_interface_name(&interface_name)?;
    let output = run_netsh(&[
        "wlan".into(),
        "show".into(),
        "profiles".into(),
        format!("interface={alias}"),
    ])?;
    Ok(json!({"interfaceName":alias,"profiles":parse_wifi_profiles(&output),"profilesLoaded":true}))
}
fn get_routes() -> AppResult<Value> {
    let raw = run_powershell_json(ROUTES_SCRIPT, None)?;
    let mut metrics = HashMap::new();
    for item in arrays(raw.get("metrics")) {
        let family = field_string(&item, "addressFamily");
        let index = field_i64(&item, "interfaceIndex");
        metrics.insert(
            format!("{family}:{index}"),
            field_i64(&item, "interfaceMetric"),
        );
    }
    let mut routes: BTreeMap<String, Value> = BTreeMap::new();
    let persistent = arrays(raw.get("persistent"));
    let persistent_keys = persistent.iter().map(route_key).collect::<Vec<_>>();
    for route in arrays(raw.get("active")).into_iter().chain(persistent) {
        let key = route_key(&route);
        let old = routes.remove(&key);
        let mut stores = arrays(old.as_ref().and_then(|x| x.get("policyStores")));
        let store = field_string(&route, "policyStore");
        if !stores.iter().any(|x| x.as_str() == Some(&store)) {
            stores.push(json!(store));
        }
        let family = field_string(&route, "addressFamily");
        let index = field_i64(&route, "interfaceIndex");
        let metric = metrics.get(&format!("{family}:{index}")).copied();
        let next = field_string(&route, "nextHop");
        let dest = field_string(&route, "destinationPrefix");
        let is_persistent = persistent_keys.contains(&key);
        let protocol = field_string(&route, "protocol");
        let default_route = dest == "0.0.0.0/0" || dest == "::/0";
        let on_link = next == "0.0.0.0" || next == "::";
        let multicast = dest == "224.0.0.0/4" || dest.to_lowercase() == "ff00::/8";
        let managed = is_persistent || protocol == "NetMgmt";
        let identity_editable = family == "IPv4" && managed && !on_link && !multicast;
        let removable = identity_editable && !default_route;
        routes.insert(key,json!({"addressFamily":family,"destinationPrefix":dest,"nextHop":next,"interfaceAlias":field_string(&route,"interfaceAlias"),"interfaceIndex":index,"routeMetric":field_i64(&route,"routeMetric"),"interfaceMetric":metric,"effectiveMetric":metric.map(|m|m+field_i64(&route,"routeMetric")),"protocol":protocol,"policyStores":stores,"persistent":is_persistent,"temporary":!is_persistent,"state":field_string(&route,"state"),"publish":field_string(&route,"publish"),"validLifetime":field_string(&route,"validLifetime"),"preferredLifetime":field_string(&route,"preferredLifetime"),"defaultRoute":default_route,"onLink":on_link,"systemManaged":!managed,"editable":true,"identityEditable":identity_editable,"removable":removable,"hiddenByDefault":multicast||(on_link&&!default_route&&!managed)}));
    }
    let route_items = routes.into_values().collect::<Vec<_>>();
    let default_candidates = route_items
        .iter()
        .filter(|route| {
            route
                .get("defaultRoute")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .cloned()
        .collect::<Vec<_>>();
    Ok(
        json!({"routes":route_items,"defaultCandidates":default_candidates,"refreshedAt":Utc::now().to_rfc3339()}),
    )
}
fn route_key(route: &Value) -> String {
    format!(
        "{}|{}|{}|{}",
        field_string(route, "addressFamily"),
        field_string(route, "destinationPrefix"),
        field_string(route, "nextHop"),
        field_i64(route, "interfaceIndex")
    )
}
fn is_removable_route(route: &Value) -> bool {
    route
        .get("removable")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}
fn persistable_route_payload(route: &Value) -> AppResult<Value> {
    if !is_removable_route(route) || field_bool(route, "persistent") {
        return Err(err(
            "VALIDATION_ERROR",
            "Only temporary static IPv4 routes can be made persistent.",
        ));
    }
    let mut payload = route_payload(route)?;
    payload["persistent"] = json!(true);
    Ok(payload)
}
fn route_remove_payload(route: &Value) -> Value {
    json!({"addressFamily":field_string(route,"addressFamily"),"destinationPrefix":field_string(route,"destinationPrefix"),"nextHop":field_string(route,"nextHop"),"interfaceIndex":field_i64(route,"interfaceIndex"),"routeMetric":field_i64(route,"routeMetric"),"persistent":route.get("persistent").and_then(Value::as_bool).unwrap_or(false),"policyStores":route.get("policyStores").cloned().unwrap_or(json!(["ActiveStore"]))})
}
fn metric_payload(input: &Value) -> AppResult<Value> {
    let index = valid_index(input.get("interfaceIndex").unwrap_or(&Value::Null))?;
    let family = input
        .get("addressFamily")
        .and_then(Value::as_str)
        .filter(|x| *x == "IPv4" || *x == "IPv6")
        .ok_or_else(|| err("VALIDATION_ERROR", "Address family must be IPv4 or IPv6."))?;
    let automatic = input
        .get("automatic")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let metric = if automatic {
        None
    } else {
        Some(valid_metric(input.get("metric").unwrap_or(&Value::Null))?)
    };
    Ok(json!({"interfaceIndex":index,"addressFamily":family,"automatic":automatic,"metric":metric}))
}
fn route_payload(input: &Value) -> AppResult<Value> {
    let destination = normalize_cidr(
        input
            .get("destinationPrefix")
            .and_then(Value::as_str)
            .ok_or_else(|| err("VALIDATION_ERROR", "DestinationPrefix is required."))?,
    )?;
    let next = input
        .get("nextHop")
        .and_then(Value::as_str)
        .filter(|x| valid_ipv4(x))
        .ok_or_else(|| err("VALIDATION_ERROR", "NextHop must be an IPv4 address."))?;
    let index = valid_index(input.get("interfaceIndex").unwrap_or(&Value::Null))?;
    let metric = valid_metric(input.get("routeMetric").unwrap_or(&Value::Null))?;
    Ok(
        json!({"addressFamily":"IPv4","destinationPrefix":destination,"nextHop":next,"interfaceIndex":index,"routeMetric":metric,"persistent":input.get("persistent").and_then(Value::as_bool).unwrap_or(false),"note":input.get("note").and_then(Value::as_str).unwrap_or("")}),
    )
}
fn capture_baseline(state: &State<AppState>, scope: Value) -> AppResult<()> {
    let snapshot_state = current_snapshot(state)?;
    let now = Utc::now().to_rfc3339();
    let snapshot = Snapshot {
        id: Uuid::new_v4().to_string(),
        hash: fingerprint(&snapshot_state),
        name: "Before latest change".into(),
        created_at: now.clone(),
        last_used_at: now,
        use_count: 0,
        state: snapshot_state,
    };
    let mut cfg = config(state);
    cfg.baseline = Some(Baseline { snapshot, scope });
    persist(state, &cfg)
}
fn ps_set_metric(payload: &Value) -> AppResult<Value> {
    let script = r#"$p=@{InterfaceIndex=[int]$InputData.interfaceIndex;AddressFamily=[string]$InputData.addressFamily;AutomaticMetric=$(if($InputData.automatic){'Enabled'}else{'Disabled'});Confirm=$false};if(-not $InputData.automatic){$p.InterfaceMetric=[int]$InputData.metric};Set-NetIPInterface @p -ErrorAction Stop;$r=Get-NetIPInterface -InterfaceIndex ([int]$InputData.interfaceIndex) -AddressFamily ([string]$InputData.addressFamily);[PSCustomObject]@{interfaceIndex=[int]$r.InterfaceIndex;addressFamily=[string]$r.AddressFamily;automaticMetric=([string]$r.AutomaticMetric -eq 'Enabled');interfaceMetric=[long]$r.InterfaceMetric}|ConvertTo-Json -Compress"#;
    run_powershell_json(script, Some(payload))
}
#[tauri::command]
fn preview_metric_change(state: State<AppState>, input: Value) -> AppResult<Value> {
    let payload = metric_payload(&input)?;
    let overview = get_network_overview()?;
    let index = field_i64(&payload, "interfaceIndex");
    let family = field_string(&payload, "addressFamily");
    let adapter = arrays(overview.get("interfaces"))
        .into_iter()
        .find(|x| field_i64(x, "interfaceIndex") == index)
        .ok_or_else(|| err("NOT_FOUND", "Interface does not exist."))?;
    let key = if family == "IPv4" { "ipv4" } else { "ipv6" };
    let current = adapter
        .get("metrics")
        .and_then(Value::as_object)
        .and_then(|metrics| metrics.get(key))
        .cloned()
        .unwrap_or(Value::Null);
    if current.is_null() {
        return Err(err(
            "NOT_FOUND",
            "The interface does not support this address family.",
        ));
    }
    let unchanged = current.get("automaticMetric").and_then(Value::as_bool)
        == payload.get("automatic").and_then(Value::as_bool)
        && (payload
            .get("automatic")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || current.get("interfaceMetric") == payload.get("metric"));
    let mut warnings = vec![];
    if adapter
        .get("defaultRoute")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        warnings.push("This interface carries a default gateway; changing Metric can alter the default network path.");
    }
    let token = if unchanged {
        None
    } else {
        Some(issue_confirmation(&state, "metric", &payload)?)
    };
    Ok(
        json!({"input":payload,"current":current,"unchanged":unchanged,"warnings":warnings,"confirmationToken":token,"requiresAdmin":true}),
    )
}
#[tauri::command]
fn apply_metric_change(state: State<AppState>, input: Value) -> AppResult<Value> {
    let payload = metric_payload(input.get("metric").unwrap_or(&input))?;
    let token = input
        .get("confirmationToken")
        .and_then(Value::as_str)
        .ok_or_else(|| err("CONFIRMATION_EXPIRED", "Preview is required."))?;
    consume_confirmation(&state, token, "metric", &payload)?;
    require_admin()?;
    capture_baseline(&state, json!({"metrics":[payload.clone()]}))?;
    ps_set_metric(&payload)
}

#[tauri::command]
fn preview_add_route(state: State<AppState>, input: Value) -> AppResult<Value> {
    let payload = route_payload(&input)?;
    let routes = get_routes()?;
    let exact = arrays(routes.get("routes")).into_iter().find(|r| {
        field_string(r, "destinationPrefix") == field_string(&payload, "destinationPrefix")
            && field_string(r, "nextHop") == field_string(&payload, "nextHop")
            && field_i64(r, "interfaceIndex") == field_i64(&payload, "interfaceIndex")
    });
    if exact.is_some() {
        return Err(err("CONFLICT", "The same route already exists."));
    }
    let conflicts = arrays(routes.get("routes"))
        .into_iter()
        .filter(|r| {
            field_string(r, "destinationPrefix") == field_string(&payload, "destinationPrefix")
        })
        .collect::<Vec<_>>();
    let mut warnings = vec![];
    if field_string(&payload, "destinationPrefix") == "0.0.0.0/0" {
        warnings.push("This is a default route and can change network connectivity.");
    }
    if !conflicts.is_empty() {
        warnings.push("Routes with the same destination prefix already exist.");
    }
    Ok(
        json!({"route":payload,"conflicts":conflicts,"warnings":warnings,"confirmationToken":issue_confirmation(&state,"add-route",&payload)?}),
    )
}
#[tauri::command]
fn add_route(state: State<AppState>, input: Value) -> AppResult<Value> {
    let payload = route_payload(input.get("route").unwrap_or(&input))?;
    let token = input
        .get("confirmationToken")
        .and_then(Value::as_str)
        .ok_or_else(|| err("CONFIRMATION_EXPIRED", "Preview is required."))?;
    consume_confirmation(&state, token, "add-route", &payload)?;
    require_admin()?;
    capture_baseline(&state, json!({"addRoute":payload.clone()}))?;
    let result = add_route_record(&payload)?;
    let mut cfg = config(&state);
    if payload
        .get("persistent")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || true
    {
        cfg.managed_routes.push(json!({"addressFamily":"IPv4","destinationPrefix":field_string(&payload,"destinationPrefix"),"nextHop":field_string(&payload,"nextHop"),"interfaceIndex":field_i64(&payload,"interfaceIndex")}));
    }
    persist(&state, &cfg)?;
    Ok(result)
}
fn prepare_route_edit(routes: &[Value], original: &Value, input: &Value) -> AppResult<Value> {
    let target = routes
        .iter()
        .find(|r| route_key(r) == route_key(original))
        .ok_or_else(|| {
            err(
                "NOT_FOUND",
                "Route no longer exists. Refresh the route table.",
            )
        })?;
    if !is_removable_route(target) {
        return Err(err(
            "VALIDATION_ERROR",
            "Only manual non-default IPv4 routes can be edited.",
        ));
    }
    let replacement = route_payload(input)?;
    let destination = field_string(&replacement, "destinationPrefix");
    let first_octet = destination
        .split('.')
        .next()
        .and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(255);
    if destination == "0.0.0.0/0"
        || first_octet >= 224
        || field_string(&replacement, "nextHop") == "0.0.0.0"
    {
        return Err(err(
            "VALIDATION_ERROR",
            "Editing into a default, on-link, or multicast route is not supported.",
        ));
    }
    if route_key(target) != route_key(&replacement)
        && routes
            .iter()
            .any(|r| route_key(r) == route_key(&replacement))
    {
        return Err(err(
            "CONFLICT",
            "Another route already uses this destination, gateway, and interface.",
        ));
    }
    Ok(json!({"oldRoute":route_remove_payload(target),"route":replacement,"mode":"replace"}))
}
fn route_edit_unchanged(payload: &Value) -> bool {
    let old = &payload["oldRoute"];
    let new = &payload["route"];
    route_key(old) == route_key(new)
        && field_i64(old, "routeMetric") == field_i64(new, "routeMetric")
        && field_bool(old, "persistent") == field_bool(new, "persistent")
}
fn edit_route_script() -> String {
    format!(
        r#"
function Add-EditedRoute($record) {{
    $InputData=$record
    & {{ {ADD_ROUTE_SCRIPT} }} | Out-Null
}}
function Remove-EditedRoute($record, $stores) {{
    foreach($store in $stores){{
        $existing=@(Get-NetRoute -AddressFamily IPv4 -DestinationPrefix ([string]$record.destinationPrefix) -NextHop ([string]$record.nextHop) -InterfaceIndex ([int]$record.interfaceIndex) -PolicyStore $store -ErrorAction SilentlyContinue)
        if($existing.Count -gt 0){{
            Remove-NetRoute -AddressFamily IPv4 -DestinationPrefix ([string]$record.destinationPrefix) -NextHop ([string]$record.nextHop) -InterfaceIndex ([int]$record.interfaceIndex) -PolicyStore $store -Confirm:$false -ErrorAction Stop
        }}
    }}
}}
$old=$InputData.oldRoute
$new=$InputData.route
try {{
    Remove-EditedRoute $old @($old.policyStores)
    Add-EditedRoute $new
}} catch {{
    $failure=$_.Exception.Message
    try {{
        Remove-EditedRoute $new @('PersistentStore','ActiveStore')
        Remove-EditedRoute $old @('PersistentStore','ActiveStore')
        Add-EditedRoute $old
    }} catch {{ throw "Route edit failed: $failure. Restoring the original route also failed: $($_.Exception.Message). Use Undo last app change to recover." }}
    throw "Route edit failed: $failure. The original route was restored."
}}
[PSCustomObject]@{{edited=$true}}|ConvertTo-Json -Compress
"#
    )
}
#[tauri::command]
fn preview_edit_route(state: State<AppState>, input: Value) -> AppResult<Value> {
    let routes = get_routes()?;
    let payload = prepare_route_edit(
        &arrays(routes.get("routes")),
        &input["originalRoute"],
        &input["route"],
    )?;
    let unchanged = route_edit_unchanged(&payload);
    let token = if unchanged {
        None
    } else {
        Some(issue_confirmation(&state, "edit-route", &payload)?)
    };
    Ok(
        json!({"oldRoute":payload["oldRoute"],"route":payload["route"],"unchanged":unchanged,"warnings":["The old route will be briefly replaced. Connectivity may be interrupted. The app will try to restore the original route if the edit fails."],"confirmationToken":token}),
    )
}
#[tauri::command]
fn edit_route(state: State<AppState>, input: Value) -> AppResult<Value> {
    let routes = get_routes()?;
    let payload = prepare_route_edit(
        &arrays(routes.get("routes")),
        &input["originalRoute"],
        &input["route"],
    )?;
    consume_confirmation(
        &state,
        input
            .get("confirmationToken")
            .and_then(Value::as_str)
            .unwrap_or(""),
        "edit-route",
        &payload,
    )?;
    require_admin()?;
    capture_baseline(&state, json!({"editRoute":payload}))?;
    let result = run_powershell_json(&edit_route_script(), Some(&payload))?;
    let mut cfg = config(&state);
    cfg.managed_routes.retain(|r| {
        route_key(r) != route_key(&payload["oldRoute"])
            && route_key(r) != route_key(&payload["route"])
    });
    cfg.managed_routes.push(payload["route"].clone());
    persist(&state, &cfg)?;
    Ok(result)
}
fn temporary_route(requested: &Value) -> AppResult<Value> {
    let target = arrays(get_routes()?.get("routes"))
        .into_iter()
        .find(|r| route_key(r) == route_key(requested))
        .ok_or_else(|| err("NOT_FOUND", "Route no longer exists."))?;
    persistable_route_payload(&target)?;
    Ok(target)
}
#[tauri::command]
fn preview_persist_route(state: State<AppState>, route: Value) -> AppResult<Value> {
    let target = temporary_route(&route)?;
    let payload = persistable_route_payload(&target)?;
    Ok(
        json!({"route":payload,"warnings":["The route will be briefly replaced and saved across restarts. If creation fails, the app will try to restore the temporary route."],"confirmationToken":issue_confirmation(&state,"persist-route",&payload)?}),
    )
}
fn persist_route_script() -> String {
    // Recreate with the default stores; restore the active route on failure.
    format!(
        r#"
Remove-NetRoute -AddressFamily IPv4 -DestinationPrefix ([string]$InputData.destinationPrefix) -NextHop ([string]$InputData.nextHop) -InterfaceIndex ([int]$InputData.interfaceIndex) -PolicyStore ActiveStore -Confirm:$false -ErrorAction Stop
try {{
{ADD_ROUTE_SCRIPT}
}} catch {{
    $failure=$_.Exception.Message
    try {{
        $active=@(Get-NetRoute -AddressFamily IPv4 -DestinationPrefix ([string]$InputData.destinationPrefix) -NextHop ([string]$InputData.nextHop) -InterfaceIndex ([int]$InputData.interfaceIndex) -PolicyStore ActiveStore -ErrorAction SilentlyContinue)
        if($active.Count -eq 0){{
            $InputData.persistent=$false
            & {{ {ADD_ROUTE_SCRIPT} }} | Out-Null
        }}
    }} catch {{ throw "Could not make the route persistent: $failure. Restoring the temporary route also failed: $($_.Exception.Message). Use Reset to pre-change baseline." }}
    throw "Could not make the route persistent: $failure. The temporary route remains active."
}}
"#
    )
}
#[tauri::command]
fn persist_route(state: State<AppState>, input: Value) -> AppResult<Value> {
    let target = temporary_route(input.get("route").unwrap_or(&input))?;
    let payload = persistable_route_payload(&target)?;
    consume_confirmation(
        &state,
        input
            .get("confirmationToken")
            .and_then(Value::as_str)
            .unwrap_or(""),
        "persist-route",
        &payload,
    )?;
    require_admin()?;
    capture_baseline(
        &state,
        json!({"editRoute":{"oldRoute":target,"route":payload,"mode":"persistence"}}),
    )?;
    let result = run_powershell_json(&persist_route_script(), Some(&payload))?;
    let mut cfg = config(&state);
    if !cfg
        .managed_routes
        .iter()
        .any(|r| route_key(r) == route_key(&target))
    {
        cfg.managed_routes.push(target);
    }
    persist(&state, &cfg)?;
    Ok(result)
}
#[tauri::command]
fn preview_remove_route(state: State<AppState>, route: Value) -> AppResult<Value> {
    let routes = get_routes()?;
    let target = arrays(routes.get("routes"))
        .into_iter()
        .find(|r| route_key(r) == route_key(&route))
        .ok_or_else(|| err("NOT_FOUND", "Route no longer exists."))?;
    if !is_removable_route(&target) {
        return Err(err("VALIDATION_ERROR","This route is Windows-managed, on-link, multicast, IPv6, or a default route and cannot be safely removed."));
    }
    let payload = route_remove_payload(&target);
    Ok(
        json!({"route":target,"warnings":["If connectivity is affected, reopen the app as administrator and use Reset to pre-change baseline."],"confirmationToken":issue_confirmation(&state,"remove-route",&payload)?}),
    )
}
#[tauri::command]
fn remove_route(state: State<AppState>, input: Value) -> AppResult<Value> {
    let requested = input.get("route").cloned().unwrap_or(input.clone());
    let routes = get_routes()?;
    let target = arrays(routes.get("routes"))
        .into_iter()
        .find(|route| route_key(route) == route_key(&requested))
        .ok_or_else(|| err("NOT_FOUND", "Route no longer exists."))?;
    if !is_removable_route(&target) {
        return Err(err(
            "VALIDATION_ERROR",
            "This route can no longer be safely removed.",
        ));
    }
    let payload = route_remove_payload(&target);
    let token = input
        .get("confirmationToken")
        .and_then(Value::as_str)
        .ok_or_else(|| err("CONFIRMATION_EXPIRED", "Preview is required."))?;
    consume_confirmation(&state, token, "remove-route", &payload)?;
    require_admin()?;
    capture_baseline(&state, json!({"removeRoute":target}))?;
    let result = remove_route_record(&payload)?;
    let completed = arrays(Some(&result))
        .iter()
        .all(|item| field_string(item, "status") == "success");
    if completed {
        let mut cfg = config(&state);
        let key = route_key(&payload);
        cfg.managed_routes.retain(|item| route_key(item) != key);
        persist(&state, &cfg)?;
    }
    Ok(json!({"results":result,"completed":completed}))
}

fn wifi_payload(input: &Value, action: &str) -> AppResult<Value> {
    let alias = input
        .get("interfaceName")
        .and_then(Value::as_str)
        .filter(|x| !x.contains(['\r', '\n', '\0']))
        .ok_or_else(|| err("VALIDATION_ERROR", "Invalid Wi-Fi interface name."))?
        .to_string();
    let profile = input
        .get("profileName")
        .and_then(Value::as_str)
        .filter(|x| !x.contains(['\r', '\n', '\0']))
        .map(str::to_string);
    if action == "connect" && profile.as_deref().unwrap_or("").is_empty() {
        return Err(err("VALIDATION_ERROR", "Invalid Wi-Fi profile name."));
    }
    Ok(json!({"interfaceName":alias,"profileName":profile,"action":action}))
}
#[tauri::command]
fn preview_wifi_change(state: State<AppState>, input: Value) -> AppResult<Value> {
    let action = input.get("action").and_then(Value::as_str).unwrap_or("");
    if action != "connect" && action != "disconnect" {
        return Err(err(
            "VALIDATION_ERROR",
            "Wi-Fi action must be connect or disconnect.",
        ));
    }
    let payload = wifi_payload(&input, action)?;
    let overview = get_network_overview()?;
    let adapter = arrays(overview.get("interfaces"))
        .into_iter()
        .find(|item| {
            field_string(item, "alias") == field_string(&payload, "interfaceName")
                && item.get("wifi").is_some()
        })
        .ok_or_else(|| err("NOT_FOUND", "Wireless interface no longer exists."))?;
    let mut warnings = vec![];
    if adapter
        .get("defaultRoute")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        warnings.push(
            "This adapter currently has a default route; the change can interrupt connectivity.",
        );
    }
    Ok(
        json!({"wifi":payload,"warnings":warnings,"confirmationToken":issue_confirmation(&state,"wifi",&payload)?}),
    )
}
#[tauri::command]
fn connect_wifi(state: State<AppState>, input: Value) -> AppResult<Value> {
    let payload = wifi_payload(input.get("wifi").unwrap_or(&input), "connect")?;
    consume_confirmation(
        &state,
        input
            .get("confirmationToken")
            .and_then(Value::as_str)
            .unwrap_or(""),
        "wifi",
        &payload,
    )?;
    let alias = field_string(&payload, "interfaceName");
    let profile = field_string(&payload, "profileName");
    capture_baseline(
        &state,
        json!({"wifi":[{"interfaceName":alias,"profileName":profile}]}),
    )?;
    run_netsh(&[
        "wlan".into(),
        "connect".into(),
        format!("name={profile}"),
        format!("interface={alias}"),
    ])?;
    Ok(json!({"message":format!("Connection request sent to {alias}.")}))
}
#[tauri::command]
fn disconnect_wifi(state: State<AppState>, input: Value) -> AppResult<Value> {
    let payload = wifi_payload(input.get("wifi").unwrap_or(&input), "disconnect")?;
    consume_confirmation(
        &state,
        input
            .get("confirmationToken")
            .and_then(Value::as_str)
            .unwrap_or(""),
        "wifi",
        &payload,
    )?;
    let alias = field_string(&payload, "interfaceName");
    capture_baseline(&state, json!({"wifi":[{"interfaceName":alias}]}))?;
    run_netsh(&[
        "wlan".into(),
        "disconnect".into(),
        format!("interface={alias}"),
    ])?;
    Ok(json!({"message":format!("Disconnect request sent to {alias}.")}))
}

fn adapter_control_payload(input: &Value) -> AppResult<Value> {
    let index = valid_index(input.get("interfaceIndex").unwrap_or(&Value::Null))?;
    let action = input
        .get("action")
        .and_then(Value::as_str)
        .filter(|value| *value == "enable" || *value == "disable")
        .ok_or_else(|| {
            err(
                "VALIDATION_ERROR",
                "Adapter action must be enable or disable.",
            )
        })?;
    Ok(json!({"interfaceIndex":index,"action":action}))
}
#[tauri::command]
fn preview_adapter_control(state: State<AppState>, input: Value) -> AppResult<Value> {
    let payload = adapter_control_payload(&input)?;
    let overview = get_network_overview()?;
    let adapter = arrays(overview.get("interfaces"))
        .into_iter()
        .find(|item| field_i64(item, "interfaceIndex") == field_i64(&payload, "interfaceIndex"))
        .ok_or_else(|| err("NOT_FOUND", "Interface no longer exists."))?;
    if adapter
        .get("systemInterface")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(err(
            "VALIDATION_ERROR",
            "System interfaces cannot be enabled or disabled.",
        ));
    }
    let mut warnings = vec![];
    if payload.get("action").and_then(Value::as_str) == Some("disable") {
        warnings.push("Disabling this adapter can interrupt network connectivity.".to_string());
        if adapter
            .get("defaultRoute")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            warnings.push(
                "This adapter currently has a default gateway and may be your active network path."
                    .to_string(),
            );
        }
    }
    Ok(
        json!({"adapter":adapter,"action":payload["action"],"warnings":warnings,"requiresAdmin":true,"confirmationToken":issue_confirmation(&state,"adapter-control",&payload)?}),
    )
}
#[tauri::command]
fn apply_adapter_control(state: State<AppState>, input: Value) -> AppResult<Value> {
    let payload = adapter_control_payload(input.get("adapter").unwrap_or(&input))?;
    consume_confirmation(
        &state,
        input
            .get("confirmationToken")
            .and_then(Value::as_str)
            .unwrap_or(""),
        "adapter-control",
        &payload,
    )?;
    require_admin()?;
    capture_baseline(&state, json!({"adapterControl":payload.clone()}))?;
    let script = r#"$item=Get-NetAdapter -IncludeHidden -InterfaceIndex ([int]$InputData.interfaceIndex) -ErrorAction Stop;if($InputData.action -eq 'enable'){$item|Enable-NetAdapter -Confirm:$false -ErrorAction Stop}else{$item|Disable-NetAdapter -Confirm:$false -ErrorAction Stop};$result=Get-NetAdapter -IncludeHidden -InterfaceIndex ([int]$InputData.interfaceIndex);[PSCustomObject]@{interfaceIndex=[int]$result.InterfaceIndex;adminStatus=[string]$result.AdminStatus;status=[string]$result.Status}|ConvertTo-Json -Compress"#;
    run_powershell_json(script, Some(&payload))
}
#[tauri::command]
fn get_app_config(state: State<AppState>) -> Value {
    let cfg = config(&state);
    json!({"settings":cfg.settings,"profiles":cfg.profiles,"recentSnapshots":cfg.recent_snapshots,"managedRoutes":cfg.managed_routes,"hasBaseline":cfg.baseline.is_some()})
}
fn profile_summary(item: &Value) -> Value {
    json!({"id":field_string(item,"id"),"name":field_string(item,"name"),"createdAt":field_string(item,"createdAt"),"updatedAt":field_string(item,"updatedAt")})
}
#[tauri::command]
fn get_bootstrap_state(state: State<AppState>) -> Value {
    let cfg = config(&state);
    let cached=disk_cache(&state.cache_path).map(|snapshot|json!({"overview":snapshot.get("overview"),"routes":snapshot.get("routes"),"refreshedAt":snapshot.get("refreshedAt"),"source":"disk-cache","stale":true}));
    json!({"settings":cfg.settings,"config":{"profiles":cfg.profiles.iter().map(profile_summary).collect::<Vec<_>>(),"recentSnapshots":cfg.recent_snapshots.iter().map(|item|json!({"id":item.id,"name":item.name,"createdAt":item.created_at,"lastUsedAt":item.last_used_at,"useCount":item.use_count})).collect::<Vec<_>>(),"hasBaseline":cfg.baseline.is_some()},"privilege":{"isAdmin":is_admin()},"cachedNetworkState":cached})
}
#[tauri::command]
fn refresh_network_state(
    state: State<AppState>,
    scopes: Option<Vec<String>>,
    force: Option<bool>,
) -> AppResult<Value> {
    let requested = scopes.unwrap_or_else(|| vec!["interfaces".into(), "routes".into()]);
    let snapshot = if force.unwrap_or(false) {
        current_snapshot(&state)?
    } else {
        match recent_snapshot(&state) {
            Some(snapshot) => snapshot,
            None => current_snapshot(&state)?,
        }
    };
    let interfaces = requested.iter().any(|scope| scope == "interfaces");
    let routes = requested.iter().any(|scope| scope == "routes");
    Ok(
        json!({"overview":if interfaces{snapshot.get("overview").cloned()}else{None},"routes":if routes{snapshot.get("routes").cloned()}else{None},"refreshedAt":snapshot.get("refreshedAt"),"source":snapshot.get("source").and_then(Value::as_str).unwrap_or("memory"),"stale":false}),
    )
}
#[tauri::command]
fn update_settings(state: State<AppState>, settings: Settings) -> AppResult<Settings> {
    if settings.language != "en" && settings.language != "zh-CN" {
        return Err(err("VALIDATION_ERROR", "Unsupported language."));
    }
    let theme = valid_theme(&settings.theme)?.to_string();
    let mut cfg = config(&state);
    cfg.settings = Settings {
        language: settings.language,
        theme,
        show_system_interfaces: settings.show_system_interfaces,
        show_all_routes: settings.show_all_routes,
    };
    persist(&state, &cfg)?;
    Ok(cfg.settings)
}
#[tauri::command]
fn save_profile(state: State<AppState>, profile: Value) -> AppResult<Value> {
    let name = profile
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|x| !x.is_empty() && x.len() <= 100)
        .ok_or_else(|| err("VALIDATION_ERROR", "Configuration name is required."))?;
    let mut saved = profile.clone();
    let now = Utc::now().to_rfc3339();
    let id = saved
        .get("id")
        .and_then(Value::as_str)
        .filter(|x| !x.is_empty())
        .unwrap_or("")
        .to_string();
    if id.is_empty() {
        saved
            .as_object_mut()
            .unwrap()
            .insert("id".into(), json!(Uuid::new_v4().to_string()));
        saved
            .as_object_mut()
            .unwrap()
            .insert("createdAt".into(), json!(now));
    }
    saved
        .as_object_mut()
        .unwrap()
        .insert("name".into(), json!(name));
    saved
        .as_object_mut()
        .unwrap()
        .insert("updatedAt".into(), json!(now));
    let id = field_string(&saved, "id");
    let mut cfg = config(&state);
    if let Some(item) = cfg
        .profiles
        .iter_mut()
        .find(|item| field_string(item, "id") == id)
    {
        *item = saved.clone()
    } else {
        cfg.profiles.push(saved.clone())
    }
    persist(&state, &cfg)?;
    Ok(saved)
}
#[tauri::command]
fn delete_profile(state: State<AppState>, id: String) -> AppResult<Value> {
    let mut cfg = config(&state);
    let before = cfg.profiles.len();
    cfg.profiles.retain(|x| field_string(x, "id") != id);
    if cfg.profiles.len() == before {
        return Err(err("NOT_FOUND", "Configuration does not exist."));
    }
    persist(&state, &cfg)?;
    Ok(json!({"deleted":true}))
}
#[tauri::command]
fn restore_profile_preview(state: State<AppState>, id: String) -> AppResult<Value> {
    let cfg = config(&state);
    let profile = cfg
        .profiles
        .iter()
        .find(|item| field_string(item, "id") == id)
        .ok_or_else(|| err("NOT_FOUND", "Configuration does not exist."))?;
    let snapshot = profile.get("state").ok_or_else(|| {
        err(
            "VALIDATION_ERROR",
            "This configuration has no restorable network state.",
        )
    })?;
    let payload = json!({"id":id,"hash":fingerprint(snapshot)});
    Ok(
        json!({"profile":profile,"summary":"The saved configuration will restore its Metric, Wi-Fi, and app-managed-route state.","confirmationToken":issue_confirmation(&state,"restore-profile",&payload)?}),
    )
}
#[tauri::command]
fn restore_profile_apply(state: State<AppState>, input: Value) -> AppResult<Value> {
    let id = input
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| err("VALIDATION_ERROR", "Configuration id is required."))?;
    let cfg = config(&state);
    let profile = cfg
        .profiles
        .iter()
        .find(|item| field_string(item, "id") == id)
        .ok_or_else(|| err("NOT_FOUND", "Configuration does not exist."))?;
    let snapshot = profile
        .get("state")
        .ok_or_else(|| {
            err(
                "VALIDATION_ERROR",
                "This configuration has no restorable network state.",
            )
        })?
        .clone();
    let payload = json!({"id":id,"hash":fingerprint(&snapshot)});
    consume_confirmation(
        &state,
        input
            .get("confirmationToken")
            .and_then(Value::as_str)
            .unwrap_or(""),
        "restore-profile",
        &payload,
    )?;
    require_admin()?;
    capture_baseline(&state, json!({"restoreProfile":id}))?;
    store_recent(
        &state,
        "Before saved configuration restore".into(),
        snapshot_current(&state)?,
    )?;
    restore_snapshot(&state, &snapshot, &json!({"restoreProfile":id}))
}

fn snapshot_current(state: &State<AppState>) -> AppResult<Value> {
    recent_snapshot(state)
        .map(Ok)
        .unwrap_or_else(|| current_snapshot(state))
}
fn remove_route_record(route: &Value) -> AppResult<Value> {
    let stores = arrays(route.get("policyStores"));
    let mut results = vec![];
    for store in stores {
        let mut input = route.clone();
        input
            .as_object_mut()
            .ok_or_else(|| err("VALIDATION_ERROR", "Invalid route record."))?
            .insert("policyStore".into(), store);
        let script = r#"Remove-NetRoute -AddressFamily ([string]$InputData.addressFamily) -DestinationPrefix ([string]$InputData.destinationPrefix) -NextHop ([string]$InputData.nextHop) -InterfaceIndex ([int]$InputData.interfaceIndex) -PolicyStore ([string]$InputData.policyStore) -Confirm:$false -ErrorAction Stop;[PSCustomObject]@{removed=$true;policyStore=$InputData.policyStore}|ConvertTo-Json -Compress"#;
        match run_powershell_json(script, Some(&input)) {
            Ok(value) => results.push(json!({"status":"success","route":route,"result":value})),
            Err(error) => results.push(json!({"status":"failed","route":route,"error":error})),
        }
    }
    Ok(json!(results))
}
fn update_route_metric_record(route: &Value) -> AppResult<Value> {
    let script = r#"$results=@();foreach($store in @($InputData.policyStores)){try{Set-NetRoute -AddressFamily ([string]$InputData.addressFamily) -DestinationPrefix ([string]$InputData.destinationPrefix) -NextHop ([string]$InputData.nextHop) -InterfaceIndex ([int]$InputData.interfaceIndex) -PolicyStore ([string]$store) -RouteMetric ([int]$InputData.routeMetric) -Confirm:$false -ErrorAction Stop;$results+=[PSCustomObject]@{status='success';policyStore=[string]$store}}catch{$results+=[PSCustomObject]@{status='failed';policyStore=[string]$store;message=$_.Exception.Message}}};$results|ConvertTo-Json -Compress"#;
    run_powershell_json(script, Some(route))
}
fn add_route_record(route: &Value) -> AppResult<Value> {
    run_powershell_json(ADD_ROUTE_SCRIPT, Some(route))
}
fn restore_route_scope(scope: &Value) -> AppResult<Value> {
    let current = get_routes()?;
    if let Some(added) = scope.get("addRoute") {
        if let Some(actual) = arrays(current.get("routes"))
            .into_iter()
            .find(|route| route_key(route) == route_key(added))
        {
            return remove_route_record(&route_remove_payload(&actual));
        }
        return Ok(
            json!([{"status":"skipped","kind":"route","message":"The added route no longer exists."}]),
        );
    }
    if let Some(edit) = scope.get("editRoute") {
        let old = edit
            .get("oldRoute")
            .ok_or_else(|| err("VALIDATION_ERROR", "Baseline route data is incomplete."))?;
        let new_route = edit
            .get("route")
            .ok_or_else(|| err("VALIDATION_ERROR", "Baseline route data is incomplete."))?;
        if field_string(edit, "mode") == "metric" {
            let actual = arrays(current.get("routes"))
                .into_iter()
                .find(|route| route_key(route) == route_key(old))
                .ok_or_else(|| err("NOT_FOUND", "The edited route no longer exists."))?;
            let mut update = route_remove_payload(&actual);
            update.as_object_mut().unwrap().insert(
                "routeMetric".into(),
                old.get("routeMetric").cloned().unwrap_or(json!(1)),
            );
            return update_route_metric_record(&update);
        }
        let mut results = vec![];
        if let Some(actual) = arrays(current.get("routes"))
            .into_iter()
            .find(|route| route_key(route) == route_key(new_route))
        {
            results.extend(arrays(Some(&remove_route_record(&route_remove_payload(
                &actual,
            ))?)));
        }
        let refreshed = get_routes()?;
        if !arrays(refreshed.get("routes"))
            .iter()
            .any(|route| route_key(route) == route_key(old))
        {
            results.push(
                json!({"status":"success","kind":"restore-route","result":add_route_record(old)?}),
            );
        }
        return Ok(json!(results));
    }
    if let Some(old) = scope.get("removeRoute") {
        if arrays(current.get("routes"))
            .iter()
            .any(|route| route_key(route) == route_key(old))
        {
            return Ok(
                json!([{"status":"skipped","kind":"restore-route","message":"Route already exists."}]),
            );
        }
        return Ok(
            json!([{"status":"success","kind":"restore-route","result":add_route_record(old)?}]),
        );
    }
    Ok(json!([]))
}
fn scope_touches_interface(scope: &Value, target: &Value) -> bool {
    let index = field_i64(target, "interfaceIndex");
    let alias = field_string(target, "alias");
    if scope.get("restoreProfile").is_some() || scope.get("restoreRecent").is_some() {
        return true;
    }
    arrays(scope.get("metrics"))
        .iter()
        .any(|item| field_i64(item, "interfaceIndex") == index)
        || scope
            .get("adapterControl")
            .map(|item| field_i64(item, "interfaceIndex") == index)
            .unwrap_or(false)
        || arrays(scope.get("wifi"))
            .iter()
            .any(|item| field_string(item, "interfaceName") == alias)
        || arrays(scope.pointer("/windowsDefaults/interfaces"))
            .iter()
            .any(|item| field_i64(item, "interfaceIndex") == index)
}
fn restore_snapshot(state: &State<AppState>, snapshot: &Value, scope: &Value) -> AppResult<Value> {
    let target_interfaces = arrays(snapshot.pointer("/overview/interfaces"));
    let current = get_network_overview()?;
    let current_interfaces = arrays(current.get("interfaces"));
    let mut results = vec![];
    for target in target_interfaces {
        let index = field_i64(&target, "interfaceIndex");
        if !scope_touches_interface(scope, &target) {
            continue;
        }
        if !current_interfaces
            .iter()
            .any(|item| field_i64(item, "interfaceIndex") == index)
        {
            results.push(json!({"status":"skipped","kind":"interface","interfaceIndex":index,"message":"Interface no longer exists."}));
            continue;
        }
        for (family, key) in [("IPv4", "ipv4"), ("IPv6", "ipv6")] {
            if let Some(metric) = target.pointer(&format!("/metrics/{key}")) {
                if metric.is_null() {
                    continue;
                }
                let payload = json!({"interfaceIndex":index,"addressFamily":family,"automatic":metric.get("automaticMetric").and_then(Value::as_bool).unwrap_or(true),"metric":metric.get("interfaceMetric").and_then(Value::as_i64)});
                match ps_set_metric(&payload){Ok(value)=>results.push(json!({"status":"success","kind":"metric","result":value})),Err(error)=>results.push(json!({"status":"failed","kind":"metric","interfaceIndex":index,"addressFamily":family,"error":error}))}
            }
        }
        let target_disabled = field_string(&target, "adminStatus").eq_ignore_ascii_case("down");
        if let Some(actual) = current_interfaces
            .iter()
            .find(|item| field_i64(item, "interfaceIndex") == index)
        {
            let actual_disabled = field_string(actual, "adminStatus").eq_ignore_ascii_case("down");
            if target_disabled != actual_disabled && !field_bool(&target, "systemInterface") {
                let action = if target_disabled { "disable" } else { "enable" };
                let script = r#"$item=Get-NetAdapter -IncludeHidden -InterfaceIndex ([int]$InputData.interfaceIndex) -ErrorAction Stop;if($InputData.action -eq 'enable'){$item|Enable-NetAdapter -Confirm:$false -ErrorAction Stop}else{$item|Disable-NetAdapter -Confirm:$false -ErrorAction Stop};[PSCustomObject]@{interfaceIndex=[int]$InputData.interfaceIndex;status='success'}|ConvertTo-Json -Compress"#;
                match run_powershell_json(script, Some(&json!({"interfaceIndex":index,"action":action}))) {
                    Ok(value) => results.push(json!({"status":"success","kind":"adapter","result":value})),
                    Err(error) => results.push(json!({"status":"failed","kind":"adapter","interfaceIndex":index,"error":error})),
                }
            }
        }
        if let Some(wifi) = target.get("wifi") {
            let alias = field_string(&target, "alias");
            let connected = wifi
                .get("connected")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let profile = field_string(wifi, "currentProfile");
            let wifi_result = if connected && !profile.is_empty() {
                run_netsh(&[
                    "wlan".into(),
                    "connect".into(),
                    format!("name={profile}"),
                    format!("interface={alias}"),
                ])
            } else if !connected {
                run_netsh(&[
                    "wlan".into(),
                    "disconnect".into(),
                    format!("interface={alias}"),
                ])
            } else {
                continue;
            };
            match wifi_result {
                Ok(_) => results.push(json!({"status":"success","kind":"wifi","interface":alias})),
                Err(error) => results
                    .push(json!({"status":"failed","kind":"wifi","interface":alias,"error":error})),
            }
        }
    }
    if scope.get("restoreProfile").is_none() && scope.get("restoreRecent").is_none() {
        return Ok(json!({"restored":true,"results":results}));
    }
    let cfg = config(state);
    let current_routes = get_routes()?;
    let current_route_items = arrays(current_routes.get("routes"));
    let target_routes = arrays(snapshot.pointer("/routes/routes"));
    for managed in cfg.managed_routes {
        let key = route_key(&managed);
        let current_item = current_route_items
            .iter()
            .find(|item| route_key(item) == key);
        let target_item = target_routes.iter().find(|item| route_key(item) == key);
        if current_item.is_some() && target_item.is_none() {
            let mut remove = managed.clone();
            remove.as_object_mut().unwrap().insert(
                "policyStores".into(),
                current_item
                    .and_then(|x| x.get("policyStores"))
                    .cloned()
                    .unwrap_or(json!(["ActiveStore"])),
            );
            results.extend(arrays(Some(&remove_route_record(&remove)?)));
        } else if current_item.is_none() {
            if let Some(route) = target_item {
                let add = json!({"destinationPrefix":field_string(route,"destinationPrefix"),"nextHop":field_string(route,"nextHop"),"interfaceIndex":field_i64(route,"interfaceIndex"),"routeMetric":field_i64(route,"routeMetric"),"persistent":route.get("persistent").and_then(Value::as_bool).unwrap_or(false)});
                match add_route_record(&add) {
                    Ok(value) => {
                        results.push(json!({"status":"success","kind":"route","result":value}))
                    }
                    Err(error) => results
                        .push(json!({"status":"failed","kind":"route","route":add,"error":error})),
                }
            }
        }
    }
    Ok(json!({"restored":true,"results":results}))
}

fn has_failed_result(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            map.get("status").and_then(Value::as_str) == Some("failed")
                || map.values().any(has_failed_result)
        }
        Value::Array(items) => items.iter().any(has_failed_result),
        _ => false,
    }
}
fn store_recent(state: &State<AppState>, name: String, snapshot: Value) -> AppResult<Snapshot> {
    let hash = fingerprint(&snapshot);
    let now = Utc::now().to_rfc3339();
    let mut cfg = config(state);
    if let Some(index) = cfg.recent_snapshots.iter().position(|x| x.hash == hash) {
        let mut item = cfg.recent_snapshots.remove(index);
        item.last_used_at = now;
        item.use_count += 1;
        item.name = name;
        cfg.recent_snapshots.insert(0, item.clone());
        persist(state, &cfg)?;
        return Ok(item);
    }
    let item = Snapshot {
        id: Uuid::new_v4().to_string(),
        hash,
        name,
        created_at: now.clone(),
        last_used_at: now,
        use_count: 1,
        state: snapshot,
    };
    cfg.recent_snapshots.insert(0, item.clone());
    cfg.recent_snapshots.truncate(10);
    persist(state, &cfg)?;
    Ok(item)
}
#[tauri::command]
fn capture_recent_snapshot(state: State<AppState>, name: Option<String>) -> AppResult<Snapshot> {
    store_recent(
        &state,
        name.unwrap_or_else(|| "Current network state".into()),
        snapshot_current(&state)?,
    )
}
#[tauri::command]
fn restore_recent_preview(state: State<AppState>, id: String) -> AppResult<Value> {
    let cfg = config(&state);
    let item = cfg
        .recent_snapshots
        .iter()
        .find(|x| x.id == id)
        .ok_or_else(|| err("NOT_FOUND", "Recent configuration not found."))?;
    Ok(
        json!({"snapshot":item,"summary":"The snapshot will restore saved Metric settings and app-managed routes. Wi-Fi actions require explicit confirmation.","confirmationToken":issue_confirmation(&state,"restore-recent",&json!({"id":id,"hash":item.hash}))?}),
    )
}
#[tauri::command]
fn restore_recent_apply(state: State<AppState>, input: Value) -> AppResult<Value> {
    let id = input
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| err("VALIDATION_ERROR", "Configuration id is required."))?;
    let cfg = config(&state);
    let item = cfg
        .recent_snapshots
        .iter()
        .find(|x| x.id == id)
        .ok_or_else(|| err("NOT_FOUND", "Recent configuration not found."))?
        .clone();
    let payload = json!({"id":id,"hash":item.hash});
    consume_confirmation(
        &state,
        input
            .get("confirmationToken")
            .and_then(Value::as_str)
            .unwrap_or(""),
        "restore-recent",
        &payload,
    )?;
    require_admin()?;
    capture_baseline(&state, json!({"restoreRecent":id}))?;
    store_recent(
        &state,
        "Before recent restore".into(),
        snapshot_current(&state)?,
    )?;
    restore_snapshot(&state, &item.state, &json!({"restoreRecent":id}))
}

#[tauri::command]
fn reset_preview(state: State<AppState>) -> AppResult<Value> {
    let cfg = config(&state);
    let baseline = cfg
        .baseline
        .ok_or_else(|| err("NOT_FOUND", "No pre-change baseline is available."))?;
    Ok(
        json!({"baseline":baseline.snapshot,"scope":baseline.scope,"confirmationToken":issue_confirmation(&state,"reset-baseline",&json!({"hash":baseline.snapshot.hash,"scope":baseline.scope}))?}),
    )
}
#[tauri::command]
fn reset_apply(state: State<AppState>, input: Value) -> AppResult<Value> {
    let mut cfg = config(&state);
    let baseline = cfg
        .baseline
        .clone()
        .ok_or_else(|| err("NOT_FOUND", "No pre-change baseline is available."))?;
    let payload = json!({"hash":baseline.snapshot.hash,"scope":baseline.scope});
    consume_confirmation(
        &state,
        input
            .get("confirmationToken")
            .and_then(Value::as_str)
            .unwrap_or(""),
        "reset-baseline",
        &payload,
    )?;
    require_admin()?;
    store_recent(
        &state,
        "Before baseline reset".into(),
        snapshot_current(&state)?,
    )?;
    let result = restore_snapshot(&state, &baseline.snapshot.state, &baseline.scope)?;
    let route_result = restore_route_scope(&baseline.scope)?;
    let complete = !has_failed_result(&result) && !has_failed_result(&route_result);
    if complete {
        cfg.baseline = None;
        persist(&state, &cfg)?;
    }
    Ok(
        json!({"reset":complete,"partial":!complete,"result":result,"routeResult":route_result,"message":if complete {"The last app change was undone."} else {"Undo was only partially completed. Recovery remains available."}}),
    )
}
fn windows_defaults_payload(state: &State<AppState>, input: &Value) -> AppResult<Value> {
    let overview = get_network_overview()?;
    let cfg = config(state);
    let eligible=arrays(overview.get("interfaces")).into_iter().filter(|item|!item.get("systemInterface").and_then(Value::as_bool).unwrap_or(false)).map(|item|json!({"interfaceIndex":field_i64(&item,"interfaceIndex"),"alias":field_string(&item,"alias")})).collect::<Vec<_>>();
    let requested = arrays(input.get("interfaceIndexes"))
        .into_iter()
        .map(|item| valid_index(&item))
        .collect::<AppResult<Vec<_>>>()?;
    let selected = if input.get("interfaceIndexes").is_none() {
        eligible.clone()
    } else {
        eligible
            .iter()
            .filter(|item| requested.contains(&field_i64(item, "interfaceIndex")))
            .cloned()
            .collect::<Vec<_>>()
    };
    if selected.len() != requested.len() {
        return Err(err(
            "VALIDATION_ERROR",
            "Only eligible adapters can be restored.",
        ));
    }
    let remove_routes = input
        .get("removeManagedRoutes")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let managed_routes = if remove_routes {
        cfg.managed_routes
    } else {
        vec![]
    };
    if selected.is_empty() && managed_routes.is_empty() {
        return Err(err(
            "VALIDATION_ERROR",
            "Select at least one adapter or choose app-managed routes to restore.",
        ));
    }
    Ok(
        json!({"interfaces":selected,"managedRoutes":managed_routes,"removeManagedRoutes":remove_routes}),
    )
}
#[tauri::command]
fn windows_defaults_preview(state: State<AppState>, input: Option<Value>) -> AppResult<Value> {
    let payload = windows_defaults_payload(&state, input.as_ref().unwrap_or(&Value::Null))?;
    Ok(
        json!({"metrics":"Enable AutomaticMetric for the selected IPv4/IPv6 interfaces.","routes":"Remove only routes recorded as app-managed when selected.","wifi":"Wi-Fi is unchanged.","interfaces":payload["interfaces"],"managedRoutes":payload["managedRoutes"],"removeManagedRoutes":payload["removeManagedRoutes"],"confirmationToken":issue_confirmation(&state,"windows-defaults",&payload)?}),
    )
}
#[tauri::command]
fn windows_defaults_apply(state: State<AppState>, input: Value) -> AppResult<Value> {
    let payload = windows_defaults_payload(&state, &input)?;
    consume_confirmation(
        &state,
        input
            .get("confirmationToken")
            .and_then(Value::as_str)
            .unwrap_or(""),
        "windows-defaults",
        &payload,
    )?;
    require_admin()?;
    capture_baseline(&state, json!({"windowsDefaults":payload.clone()}))?;
    let script = r#"$results=@();foreach($i in @($InputData.interfaces)){foreach($family in @('IPv4','IPv6')){try{Set-NetIPInterface -InterfaceIndex ([int]$i.interfaceIndex) -AddressFamily $family -AutomaticMetric Enabled -Confirm:$false -ErrorAction Stop;$results+=[PSCustomObject]@{interfaceIndex=$i.interfaceIndex;addressFamily=$family;status='success'}}catch{$results+=[PSCustomObject]@{interfaceIndex=$i.interfaceIndex;addressFamily=$family;status='skipped'}}}};$results|ConvertTo-Json -Compress"#;
    let metric_results = run_powershell_json(script, Some(&payload))?;
    let current_routes = get_routes()?;
    let mut route_results = vec![];
    for managed in arrays(payload.get("managedRoutes")) {
        if let Some(actual) = arrays(current_routes.get("routes"))
            .into_iter()
            .find(|route| route_key(route) == route_key(&managed))
        {
            let mut route = managed.clone();
            route.as_object_mut().unwrap().insert(
                "policyStores".into(),
                actual
                    .get("policyStores")
                    .cloned()
                    .unwrap_or(json!(["ActiveStore"])),
            );
            route_results.extend(arrays(Some(&remove_route_record(&route)?)));
        }
    }
    if payload
        .get("removeManagedRoutes")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let mut new_cfg = config(&state);
        new_cfg.managed_routes.clear();
        persist(&state, &new_cfg)?;
    }
    Ok(
        json!({"metrics":metric_results,"routeResults":route_results,"message":"Windows-managed automatic Metric has been enabled. Only routes created by this app were removed."}),
    )
}

#[tauri::command]
fn get_diagnostics(state: State<AppState>) -> AppResult<Value> {
    let overview = get_network_overview()?;
    let routes = get_routes()?;
    let cfg = config(&state);
    let os=run_powershell_json("Get-CimInstance Win32_OperatingSystem|Select-Object Caption,Version,BuildNumber,OSArchitecture|ConvertTo-Json -Compress",None).unwrap_or(Value::Null);
    let changes = arrays(overview.get("interfaces"))
        .into_iter()
        .filter(|x| {
            x.pointer("/deviations/manualIpv4Metric")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || x.pointer("/deviations/manualIpv6Metric")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                || x.pointer("/deviations/adapterDisabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let static_routes = arrays(routes.get("routes"))
        .into_iter()
        .filter(|x| {
            x.get("persistent")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || field_string(x, "protocol") == "NetMgmt"
        })
        .collect::<Vec<_>>();
    Ok(
        json!({"generatedAt":Utc::now().to_rfc3339(),"os":os,"privilege":{"isAdmin":is_admin()},"changes":{"interfaces":changes,"staticRoutes":static_routes},"overview":overview,"routes":routes,"app":{"schemaVersion":cfg.schema_version,"recentSnapshots":cfg.recent_snapshots.len()}}),
    )
}
#[tauri::command]
fn restart_as_administrator(app: AppHandle) -> AppResult<Value> {
    if is_admin() {
        return Ok(json!({"started":false,"alreadyAdmin":true}));
    }
    let exe = std::env::current_exe().map_err(|_| {
        err(
            "COMMAND_FAILED",
            "Cannot locate the application executable.",
        )
    })?;
    let verb = wide_null(OsStr::new("runas"));
    let executable = wide_null(exe.as_os_str());
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            verb.as_ptr(),
            executable.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        return Err(err_with(
            "ELEVATION_FAILED",
            "Windows could not restart the application as administrator.",
            json!({"shellExecuteCode":result as isize,"hint":"Accept the Windows UAC prompt. Installation in a user folder is supported."}),
        ));
    }
    app.exit(0);
    Ok(json!({"started":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn route_edit_validation_and_live_state_are_checked() {
        let old = json!({"addressFamily":"IPv4","destinationPrefix":"172.16.0.0/12","nextHop":"192.168.3.1","interfaceIndex":26,"routeMetric":5,"persistent":false,"removable":true,"policyStores":["ActiveStore"]});
        let routes = vec![old.clone()];
        let unchanged = prepare_route_edit(&routes, &old, &old).unwrap();
        assert!(route_edit_unchanged(&unchanged));
        let mut changed = old.clone();
        changed["destinationPrefix"] = json!("172.20.1.2/16");
        changed["routeMetric"] = json!(9);
        changed["persistent"] = json!(true);
        let payload = prepare_route_edit(&routes, &old, &changed).unwrap();
        assert_eq!(payload["route"]["destinationPrefix"], "172.20.0.0/16");
        assert!(!route_edit_unchanged(&payload));
        let mut live = old.clone();
        live["routeMetric"] = json!(7);
        assert_ne!(
            fingerprint(&payload),
            fingerprint(&prepare_route_edit(&[live], &old, &changed).unwrap())
        );
        let mut duplicate = payload["route"].clone();
        duplicate["removable"] = json!(true);
        assert!(prepare_route_edit(&[old.clone(), duplicate], &old, &changed).is_err());
        assert!(prepare_route_edit(&[], &old, &changed).is_err());
        let mut protected = old.clone();
        protected["removable"] = json!(false);
        assert!(prepare_route_edit(&[protected], &old, &changed).is_err());
        for (field, value) in [
            ("nextHop", json!("0.0.0.0")),
            ("destinationPrefix", json!("0.0.0.0/0")),
            ("destinationPrefix", json!("224.0.0.0/4")),
            ("routeMetric", json!(0)),
        ] {
            let mut invalid = changed.clone();
            invalid[field] = value;
            assert!(prepare_route_edit(&routes, &old, &invalid).is_err());
        }
    }
    #[test]
    fn route_edits_roll_back_creation_and_partial_removal_failures() {
        // All Windows networking cmdlets used by the script are shadowed.
        let mock = r#"
$script:failNew=[bool]$InputData.failNew
$script:failRemove=[bool]$InputData.failRemove
$script:failRestore=[bool]$InputData.failRestore
$script:routes=@{}
function Mock-Key($prefix,$hop,$idx,$store){"$prefix|$hop|$idx|$store"}
foreach($store in $InputData.oldRoute.policyStores){
    $script:routes[(Mock-Key $InputData.oldRoute.destinationPrefix $InputData.oldRoute.nextHop $InputData.oldRoute.interfaceIndex $store)]=[int]$InputData.oldRoute.routeMetric
}
function Mock-Snapshot { @($script:routes.GetEnumerator()|Sort-Object Key|ForEach-Object {"$($_.Key)=$($_.Value)"}) -join ';' }
$script:original=Mock-Snapshot
function Get-NetRoute {
    [CmdletBinding()] param($AddressFamily,$DestinationPrefix,$NextHop,$InterfaceIndex,$PolicyStore)
    $key=Mock-Key $DestinationPrefix $NextHop $InterfaceIndex $PolicyStore
    if($script:routes.ContainsKey($key)){[PSCustomObject]@{RouteMetric=$script:routes[$key]}}
}
function Remove-NetRoute {
    [CmdletBinding(SupportsShouldProcess=$true)] param($AddressFamily,$DestinationPrefix,$NextHop,$InterfaceIndex,$PolicyStore)
    if($PolicyStore -eq 'PersistentStore' -and $script:failRemove){$script:failRemove=$false;throw 'Simulated removal failure'}
    $script:routes.Remove((Mock-Key $DestinationPrefix $NextHop $InterfaceIndex $PolicyStore))
}
function New-NetRoute {
    [CmdletBinding(SupportsShouldProcess=$true)] param($AddressFamily,$DestinationPrefix,$NextHop,$InterfaceIndex,$RouteMetric,$PolicyStore)
    if($RouteMetric -eq 5 -and $script:failRestore){throw 'Simulated restoration failure'}
    $script:routes[(Mock-Key $DestinationPrefix $NextHop $InterfaceIndex 'ActiveStore')]=[int]$RouteMetric
    if($RouteMetric -eq 9 -and $script:failNew){throw 'Simulated creation failure after writing ActiveStore'}
    if(-not $PolicyStore){$script:routes[(Mock-Key $DestinationPrefix $NextHop $InterfaceIndex 'PersistentStore')]=[int]$RouteMetric}
    [PSCustomObject]@{RouteMetric=$RouteMetric}
}
"#;
        for persistent in [false, true] {
            for same_identity in [false, true] {
                for failure in ["none", "create", "remove", "restore"] {
                    if failure == "remove" && !persistent {
                        continue;
                    }
                    let old = json!({"addressFamily":"IPv4","destinationPrefix":"172.16.0.0/12","nextHop":"192.168.3.1","interfaceIndex":26,"routeMetric":5,"persistent":persistent,"policyStores":if persistent {json!(["ActiveStore","PersistentStore"])} else {json!(["ActiveStore"])}});
                    let mut new = old.clone();
                    new["persistent"] = json!(!persistent);
                    new["routeMetric"] = json!(9);
                    if !same_identity {
                        new["destinationPrefix"] = json!("172.20.0.0/16");
                    }
                    let input = json!({"oldRoute":old,"route":new,"failNew":failure=="create" || failure=="restore","failRemove":failure=="remove","failRestore":failure=="restore"});
                    if failure == "restore" {
                        let error = run_powershell_json(
                            &format!("{mock}\n{}", edit_route_script()),
                            Some(&input),
                        )
                        .unwrap_err();
                        assert!(error
                            .message
                            .contains("Restoring the original route also failed"));
                        assert!(error.message.contains("Undo last app change"));
                        continue;
                    }
                    let script = if failure == "none" {
                        format!(
                            r#"{mock}
{}
if($script:routes.Count -ne $(if($InputData.route.persistent){{2}}else{{1}})){{throw 'Unexpected remaining route stores'}}
foreach($entry in $script:routes.GetEnumerator()){{if($entry.Value -ne 9 -or -not $entry.Key.StartsWith([string]$InputData.route.destinationPrefix+'|')){{throw 'Original route remains after edit'}}}}
"#,
                            edit_route_script()
                        )
                    } else {
                        format!(
                            r#"{mock}
$caught=$null
try {{ & {{ {} }} | Out-Null }} catch {{ $caught=$_.Exception.Message }}
if(-not $caught -or -not $caught.Contains('original route was restored')){{throw 'Expected a recoverable edit failure'}}
if((Mock-Snapshot) -ne $script:original){{throw 'Rollback did not restore all original stores and metrics'}}
[PSCustomObject]@{{restored=$true}}|ConvertTo-Json -Compress
"#,
                            edit_route_script()
                        )
                    };
                    let result = run_powershell_json(&script, Some(&input)).unwrap_or_else(|e| panic!("persistent={persistent}, same_identity={same_identity}, failure={failure}: {e:?}"));
                    assert_eq!(
                        result[if failure == "none" {
                            "edited"
                        } else {
                            "restored"
                        }],
                        true
                    );
                }
            }
        }
    }
    #[test]
    fn route_persistence_conversion_and_rollback_use_mock_cmdlets() {
        let mock = r#"
$script:removed=$false
$script:restored=$false
function Remove-NetRoute { $script:removed=$true }
function Get-NetRoute { }
function New-NetRoute {
    [CmdletBinding(SupportsShouldProcess=$true)]
    param($AddressFamily,$DestinationPrefix,$NextHop,$InterfaceIndex,$RouteMetric,$PolicyStore)
    if(-not $script:removed){throw 'Must replace the active route first'}
    if($InputData.persistent -and $InputData.fail){throw 'Simulated persistent creation failure'}
    if(-not $InputData.persistent){
        if($PolicyStore -ne 'ActiveStore'){throw 'Rollback must restore a temporary route'}
        $script:restored=$true
    }
    [PSCustomObject]@{DestinationPrefix=$DestinationPrefix}
}
"#;
        let payload = json!({"destinationPrefix":"172.16.0.0/12","nextHop":"192.168.3.1","interfaceIndex":26,"routeMetric":5,"persistent":true});
        let script = format!("{mock}\n{}", persist_route_script());
        let result = run_powershell_json(&script, Some(&payload)).unwrap();
        assert_eq!(result["policyStore"], "PersistentStore");
        let mut failure = payload;
        failure["fail"] = json!(true);
        let script = format!(
            r#"{mock}
try {{ {} }} catch {{
    if(-not $script:restored){{throw 'Temporary route was not restored'}}
    [PSCustomObject]@{{restored=$script:restored;message=$_.Exception.Message}}|ConvertTo-Json -Compress
}}"#,
            persist_route_script()
        );
        let result = run_powershell_json(&script, Some(&failure)).unwrap();
        assert_eq!(result["restored"], true);
        assert!(field_string(&result, "message").contains("Simulated persistent creation failure"));
    }
    #[test]
    fn persistence_conversion_rejects_system_and_persistent_routes() {
        let route = json!({"addressFamily":"IPv4","destinationPrefix":"172.16.0.0/12","nextHop":"192.168.3.1","interfaceIndex":26,"routeMetric":5,"persistent":false,"removable":true});
        assert_eq!(
            persistable_route_payload(&route).unwrap()["persistent"],
            true
        );
        assert_eq!(
            route_key(&persistable_route_payload(&route).unwrap()),
            route_key(&route)
        );
        let mut protected = route.clone();
        protected["removable"] = json!(false);
        assert!(persistable_route_payload(&protected).is_err());
        let mut persistent = route;
        persistent["persistent"] = json!(true);
        assert!(persistable_route_payload(&persistent).is_err());
    }
    #[test]
    fn route_creation_uses_supported_stores_and_returns_clean_json() {
        // Shadow the Windows cmdlet: never modify real routes during validation.
        let mock = r#"
function New-NetRoute {
    [CmdletBinding(SupportsShouldProcess=$true)]
    param([string]$AddressFamily,[string]$DestinationPrefix,[string]$NextHop,
          [int]$InterfaceIndex,[int]$RouteMetric,[ValidateSet('ActiveStore')][string]$PolicyStore)
    if($AddressFamily -ne 'IPv4' -or $DestinationPrefix -ne '172.16.0.0/12' -or
       $NextHop -ne '192.168.3.1' -or $InterfaceIndex -ne 26 -or $RouteMetric -ne 5){throw 'Incorrect route arguments'}
    if($InputData.persistent -and $PSBoundParameters.ContainsKey('PolicyStore')){throw 'Persistent routes must use the default stores'}
    if(-not $InputData.persistent -and $PolicyStore -ne 'ActiveStore'){throw 'Temporary routes must use ActiveStore'}
    [PSCustomObject]@{DestinationPrefix=$DestinationPrefix;RouteMetric=$RouteMetric}
}
"#;
        for persistent in [true, false] {
            let payload = route_payload(&json!({
                "destinationPrefix":"172.16.0.0/12", "nextHop":"192.168.3.1",
                "interfaceIndex":26, "routeMetric":5, "persistent":persistent
            }))
            .unwrap();
            let result =
                run_powershell_json(&format!("{mock}\n{ADD_ROUTE_SCRIPT}"), Some(&payload))
                    .expect("mock route creation should return valid JSON");
            assert_eq!(
                result,
                json!({"created":true,"policyStore":if persistent {"PersistentStore"} else {"ActiveStore"}})
            );
        }
    }
    #[test]
    fn powershell_failure_exposes_the_windows_reason() {
        let error = run_powershell_json("throw 'Route already exists: 测试'", None).unwrap_err();
        assert_eq!(error.code, "COMMAND_FAILED");
        assert!(error.message.contains("Route already exists: 测试"));
        assert!(!error.message.contains("CLIXML"));
    }
    #[test]
    fn cidr_is_normalized() {
        assert_eq!(normalize_cidr("172.31.9.10/12").unwrap(), "172.16.0.0/12");
    }
    #[test]
    fn invalid_metric_is_rejected() {
        assert!(valid_metric(&json!(0)).is_err());
        assert_eq!(valid_metric(&json!(5)).unwrap(), 5);
    }
    #[test]
    fn theme_and_interface_validation_are_strict() {
        assert_eq!(valid_theme("dark").unwrap(), "dark");
        assert!(valid_theme("blue").is_err());
        assert!(valid_interface_name("Wi-Fi\nname").is_err());
        assert_eq!(Settings::default().theme, "system");
    }
    #[test]
    fn adapter_control_validation_is_strict() {
        assert_eq!(
            field_string(
                &adapter_control_payload(&json!({"interfaceIndex":7,"action":"disable"})).unwrap(),
                "action"
            ),
            "disable"
        );
        assert!(adapter_control_payload(&json!({"interfaceIndex":0,"action":"disable"})).is_err());
        assert!(adapter_control_payload(&json!({"interfaceIndex":7,"action":"restart"})).is_err());
    }
    #[test]
    fn ui_contains_sidebar_and_operation_feedback() {
        let ui = include_str!("../frontend/src/network-manager-app.js");
        let html = include_str!("../frontend/index.html");
        assert!(ui.contains("operationComplete"));
        assert!(ui.contains("renderMaintenance"));
        assert!(ui.contains("adapterAction"));
        assert!(html.contains("network-manager-app"));
    }
    #[test]
    fn profile_summaries_do_not_include_snapshot_state() {
        let summary = profile_summary(
            &json!({"id":"a","name":"Home","createdAt":"x","updatedAt":"y","state":{"secret":"ignored"}}),
        );
        assert_eq!(summary["name"], "Home");
        assert!(summary.get("state").is_none());
    }
    #[test]
    fn canonical_hash_ignores_object_order() {
        assert_eq!(
            fingerprint(&json!({"a":1,"b":2})),
            fingerprint(&json!({"b":2,"a":1}))
        );
    }
    #[test]
    fn recovery_keeps_failed_steps_visible() {
        assert!(has_failed_result(
            &json!({"results":[{"status":"success"},{"status":"failed"}]})
        ));
        assert!(!has_failed_result(
            &json!({"results":[{"status":"success"},{"status":"skipped"}]})
        ));
    }
    #[test]
    fn scoped_undo_only_touches_affected_adapter() {
        let target = json!({"interfaceIndex":7,"alias":"Wi-Fi"});
        assert!(scope_touches_interface(
            &json!({"metrics":[{"interfaceIndex":7}]}),
            &target
        ));
        assert!(!scope_touches_interface(
            &json!({"metrics":[{"interfaceIndex":8}]}),
            &target
        ));
        assert!(scope_touches_interface(
            &json!({"restoreProfile":"profile"}),
            &target
        ));
    }
    #[test]
    fn wifi_parser_handles_chinese() {
        let items = parse_wifi_interfaces("名称 : WLAN\n状态 : 已连接\nSSID : 测试网络\n");
        assert_eq!(items[0]["alias"], "WLAN");
        assert_eq!(items[0]["connected"], true);
    }
    #[test]
    fn native_discovery_returns_a_structured_overview() {
        let overview = native_network_overview()
            .expect("native adapter discovery should be available on Windows");
        assert!(overview
            .get("interfaces")
            .and_then(Value::as_array)
            .is_some());
    }
}
