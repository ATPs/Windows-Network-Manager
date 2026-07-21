use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::Utc;
use encoding_rs::GBK;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, HashMap}, ffi::OsStr, fs, os::windows::{ffi::OsStrExt, process::CommandExt}, path::{Path, PathBuf}, process::Command, ptr, sync::Mutex, time::{Duration, Instant}};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;
use windows_sys::Win32::UI::{Shell::{IsUserAnAdmin, ShellExecuteW}, WindowsAndMessaging::SW_SHOWNORMAL};

const CONFIG_FILE: &str = "network-profiles.json";
const MAX_IMPORT_BYTES: u64 = 2 * 1024 * 1024;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PublicError { code: String, message: String, details: Option<Value>, recoverable: bool }
type AppResult<T> = Result<T, PublicError>;

fn err(code: &str, message: impl Into<String>) -> PublicError { PublicError { code: code.into(), message: message.into(), details: None, recoverable: true } }
fn err_with(code: &str, message: impl Into<String>, details: Value) -> PublicError { PublicError { code: code.into(), message: message.into(), details: Some(details), recoverable: true } }

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Settings { #[serde(default = "default_language")] language: String, #[serde(default = "default_theme")] theme: String, #[serde(default)] show_system_interfaces: bool, #[serde(default)] show_all_routes: bool }
fn default_language() -> String { "en".into() }
fn default_theme() -> String { "system".into() }
impl Default for Settings { fn default() -> Self { Self { language: default_language(), theme: default_theme(), show_system_interfaces: false, show_all_routes: false } } }

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Snapshot { id: String, hash: String, name: String, created_at: String, last_used_at: String, #[serde(default)] use_count: u32, state: Value }

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Baseline { snapshot: Snapshot, scope: Value }

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AppConfig {
  #[serde(default = "schema_version")] schema_version: u32,
  #[serde(default)] settings: Settings,
  #[serde(default)] profiles: Vec<Value>,
  #[serde(default)] recent_snapshots: Vec<Snapshot>,
  #[serde(default)] baseline: Option<Baseline>,
  #[serde(default)] managed_routes: Vec<Value>,
  #[serde(default)] route_notes: BTreeMap<String, String>,
}
fn schema_version() -> u32 { 2 }
impl Default for AppConfig { fn default() -> Self { Self { schema_version: 2, settings: Settings::default(), profiles: vec![], recent_snapshots: vec![], baseline: None, managed_routes: vec![], route_notes: BTreeMap::new() } } }

struct Confirmation { kind: String, hash: String, expires_at: Instant }
struct AppState { config_path: PathBuf, confirmations: Mutex<HashMap<String, Confirmation>> }

const OVERVIEW_SCRIPT: &str = r#"
$Adapters = @(Get-NetAdapter -IncludeHidden | ForEach-Object { [PSCustomObject]@{ interfaceIndex=[int]$_.InterfaceIndex; alias=[string]$_.Name; description=[string]$_.InterfaceDescription; guid=[string]$_.InterfaceGuid; macAddress=[string]$_.MacAddress; interfaceType=[int]$_.InterfaceType; hardwareInterface=[bool]$_.HardwareInterface; virtual=[bool]$_.Virtual; adminStatus=[string]$_.AdminStatus; operationalStatus=[string]$_.Status; mediaConnectionState=[string]$_.MediaConnectionState; linkSpeed=[string]$_.LinkSpeed } })
$IpInterfaces = @(Get-NetIPInterface | ForEach-Object { [PSCustomObject]@{ interfaceIndex=[int]$_.InterfaceIndex; alias=[string]$_.InterfaceAlias; addressFamily=[string]$_.AddressFamily; automaticMetric=([string]$_.AutomaticMetric -eq 'Enabled'); interfaceMetric=[long]$_.InterfaceMetric; connectionState=[string]$_.ConnectionState; dhcp=([string]$_.Dhcp -eq 'Enabled') } })
$IpConfigurations = @(Get-NetIPConfiguration -All | ForEach-Object { [PSCustomObject]@{ interfaceIndex=[int]$_.InterfaceIndex; alias=[string]$_.InterfaceAlias; ipv4Gateways=@($_.IPv4DefaultGateway | ForEach-Object { [string]$_.NextHop }); ipv6Gateways=@($_.IPv6DefaultGateway | ForEach-Object { [string]$_.NextHop }) } })
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

pub fn run() {
  tauri::Builder::default()
    .plugin(tauri_plugin_dialog::init())
    .setup(|app| {
      let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
      fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
      let config_path = config_dir.join(CONFIG_FILE);
      app.manage(AppState { config_path, confirmations: Mutex::new(HashMap::new()) });
      Ok(())
    })
    .invoke_handler(tauri::generate_handler![get_app_version, get_network_overview, get_wifi_profiles, get_routes, get_privilege_status, get_app_config, save_profile, delete_profile, restore_profile_preview, restore_profile_apply, set_language, set_theme, preview_metric_change, apply_metric_change, preview_add_route, add_route, preview_edit_route, apply_edit_route, preview_remove_route, remove_route, preview_adapter_control, apply_adapter_control, connect_wifi, disconnect_wifi, capture_recent_snapshot, restore_recent_preview, restore_recent_apply, reset_preview, reset_apply, windows_defaults_preview, windows_defaults_apply, import_configuration, export_configuration, get_diagnostics, restart_as_administrator])
    .run(tauri::generate_context!())
    .expect("error while running Windows Network Manager");
}

fn hidden_command(program: &str) -> Command {
  let mut command = Command::new(program);
  command.creation_flags(CREATE_NO_WINDOW);
  command
}

fn wide_null(value: &OsStr) -> Vec<u16> { value.encode_wide().chain(Some(0)).collect() }

fn run_powershell_json(script: &str, input: Option<&Value>) -> AppResult<Value> {
  let prefix = "$ErrorActionPreference='Stop';[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false);$OutputEncoding=[Console]::OutputEncoding;$InputData=$null;if($env:WNM_INPUT_BASE64){$InputData=([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($env:WNM_INPUT_BASE64))|ConvertFrom-Json);}";
  let encoded = BASE64.encode(format!("{}\n{}", prefix, script).encode_utf16().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>());
  let mut command = hidden_command("powershell.exe");
  command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-EncodedCommand", &encoded]);
  if let Some(payload) = input { command.env("WNM_INPUT_BASE64", BASE64.encode(serde_json::to_vec(payload).map_err(|_| err("SERIALIZE_ERROR", "Cannot serialize command input."))?)); }
  let output = command.output().map_err(|e| err_with("COMMAND_FAILED", "PowerShell could not start.", json!({"reason":e.to_string()})))?;
  if !output.status.success() { return Err(err_with("COMMAND_FAILED", "Windows command failed.", json!({"stderr":sanitize(&String::from_utf8_lossy(&output.stderr)),"exitCode":output.status.code()}))); }
  let text = String::from_utf8_lossy(&output.stdout).trim_start_matches('\u{feff}').trim().to_string();
  if text.is_empty() { return Ok(Value::Null); }
  serde_json::from_str(&text).map_err(|_| err_with("PARSE_ERROR", "Windows returned invalid JSON.", json!({"output":sanitize(&text)})))
}

fn run_netsh(args: &[String]) -> AppResult<String> {
  let output = hidden_command("netsh.exe").args(args).output().map_err(|e| err_with("COMMAND_FAILED", "netsh could not start.", json!({"reason":e.to_string()})))?;
  let stdout = decode_windows(&output.stdout);
  if !output.status.success() { return Err(err_with("COMMAND_FAILED", "netsh command failed.", json!({"stderr":sanitize(&decode_windows(&output.stderr)),"stdout":sanitize(&stdout)}))); }
  Ok(stdout)
}

fn decode_windows(bytes: &[u8]) -> String { String::from_utf8(bytes.to_vec()).unwrap_or_else(|_| { let (decoded, _, _) = GBK.decode(bytes); decoded.into_owned() }) }
fn sanitize(text: &str) -> String { text.chars().filter(|c| !c.is_control() || *c == '\n').take(2000).collect::<String>().trim().into() }
fn arrays(value: Option<&Value>) -> Vec<Value> { match value { Some(Value::Array(items)) => items.clone(), Some(Value::Null)|None => vec![], Some(item) => vec![item.clone()] } }
fn field_string(value: &Value, name: &str) -> String { value.get(name).and_then(Value::as_str).unwrap_or_default().trim().to_string() }
fn field_i64(value: &Value, name: &str) -> i64 { value.get(name).and_then(Value::as_i64).or_else(|| value.get(name).and_then(Value::as_u64).map(|x| x as i64)).unwrap_or_default() }
fn field_bool(value: &Value, name: &str) -> bool { value.get(name).and_then(Value::as_bool).unwrap_or(false) }

fn normalize_overview(raw: Value, wifi: Vec<Value>) -> Value {
  let mut entries: BTreeMap<i64, Map<String, Value>> = BTreeMap::new();
  for adapter in arrays(raw.get("adapters")) { let index=field_i64(&adapter,"interfaceIndex"); if index>0 { entries.entry(index).or_default().insert("adapter".into(),adapter); } }
  for ip in arrays(raw.get("ipInterfaces")) { let index=field_i64(&ip,"interfaceIndex"); if index>0 { let entry=entries.entry(index).or_default(); let family=field_string(&ip,"addressFamily"); entry.entry("ipInterfaces").or_insert_with(||json!({})).as_object_mut().unwrap().insert(family,ip); } }
  for item in arrays(raw.get("ipConfigurations")) { let index=field_i64(&item,"interfaceIndex"); if index>0 { entries.entry(index).or_default().insert("ipConfiguration".into(),item); } }
  for item in arrays(raw.get("addresses")) { let index=field_i64(&item,"interfaceIndex"); if index>0 { let entry=entries.entry(index).or_default(); entry.entry("addresses").or_insert_with(||json!([])).as_array_mut().unwrap().push(item); } }
  for item in arrays(raw.get("dns")) { let index=field_i64(&item,"interfaceIndex"); if index>0 { let entry=entries.entry(index).or_default(); entry.entry("dns").or_insert_with(||json!([])).as_array_mut().unwrap().push(item); } }
  for item in arrays(raw.get("connectionProfiles")) { let index=field_i64(&item,"interfaceIndex"); if index>0 { entries.entry(index).or_default().insert("connectionProfile".into(),item); } }
  let wifi_by_alias: HashMap<String, Value> = wifi.into_iter().filter_map(|item| Some((field_string(&item,"alias"),item))).collect();
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

fn parse_wifi_interfaces(output: &str) -> Vec<Value> {
  let mut result=vec![]; let mut current: Map<String,Value>=Map::new();
  for line in output.lines() { let Some((key,value))=line.split_once(':').or_else(||line.split_once('：')) else {continue}; let key=key.trim().to_lowercase(); let value=value.trim(); let name=match key.as_str(){"name"|"名称"=>Some("alias"),"description"|"描述"|"说明"=>Some("description"),"guid"=>Some("guid"),"state"|"状态"=>Some("state"),"ssid"=>Some("ssid"),"signal"|"信号"=>Some("signal"),"profile"|"配置文件"=>Some("currentProfile"),"radio type"|"无线电类型"=>Some("radioType"),_=>None}; if let Some(name)=name { if name=="alias"&&!current.is_empty(){ let connected=field_string(&Value::Object(current.clone()),"state").to_lowercase()=="connected"||field_string(&Value::Object(current.clone()),"state")=="已连接"; current.insert("connected".into(),json!(connected)); result.push(Value::Object(current)); current=Map::new(); } current.insert(name.into(),json!(value)); } }
  if !current.is_empty(){let connected=field_string(&Value::Object(current.clone()),"state").to_lowercase()=="connected"||field_string(&Value::Object(current.clone()),"state")=="已连接";current.insert("connected".into(),json!(connected));result.push(Value::Object(current));} result
}

fn parse_wifi_profiles(output: &str) -> Vec<String> { let mut profiles=vec![]; for line in output.lines(){let Some((key,value))=line.split_once(':').or_else(||line.split_once('：'))else{continue};let lower=key.to_lowercase();if lower.contains("profile")||key.contains("配置文件"){let value=value.trim().to_string();if !value.is_empty()&&!profiles.contains(&value){profiles.push(value)}}} profiles }

fn wifi_overview() -> AppResult<Vec<Value>> { let output=run_netsh(&["wlan".into(),"show".into(),"interfaces".into()])?; let mut interfaces=parse_wifi_interfaces(&output); for item in &mut interfaces { item.as_object_mut().unwrap().insert("profiles".into(),json!([])); item.as_object_mut().unwrap().insert("profilesLoaded".into(),json!(false)); } Ok(interfaces) }
fn valid_interface_name(value:&str)->AppResult<String>{let name=value.trim();if name.is_empty()||name.len()>256||name.chars().any(|c|c=='\0'||c=='\r'||c=='\n'||c.is_control()){Err(err("VALIDATION_ERROR","Invalid Wi-Fi interface name."))}else{Ok(name.to_string())}}
fn valid_theme(value:&str)->AppResult<&str>{if ["system","light","dark"].contains(&value){Ok(value)}else{Err(err("VALIDATION_ERROR","Unsupported theme."))}}

fn config_from_path(path: &Path) -> AppConfig { match fs::read_to_string(path) { Ok(text)=>match serde_json::from_str::<AppConfig>(&text){Ok(config)=>config,Err(_)=>{let backup=path.with_extension(format!("corrupt-{}.json",Utc::now().format("%Y%m%d%H%M%S")));let _=fs::rename(path,backup);AppConfig::default()}},Err(_)=>AppConfig::default() } }
fn save_config(path: &Path, config: &AppConfig) -> AppResult<()> { let text=serde_json::to_string_pretty(config).map_err(|_|err("SERIALIZE_ERROR","Cannot save configuration."))?; let temp=path.with_extension("tmp"); fs::write(&temp,format!("{text}\n")).map_err(|e|err_with("CONFIG_WRITE_FAILED","Cannot write configuration.",json!({"reason":e.to_string()})))?; fs::rename(temp,path).map_err(|e|err_with("CONFIG_WRITE_FAILED","Cannot replace configuration.",json!({"reason":e.to_string()}))) }
fn config(state:&State<AppState>) -> AppConfig { config_from_path(&state.config_path) }
fn persist(state:&State<AppState>, config:&AppConfig)->AppResult<()> { save_config(&state.config_path,config) }

fn canonical(value:&Value)->String { match value { Value::Object(map)=>{let mut keys=map.keys().collect::<Vec<_>>();keys.sort();format!("{{{}}}",keys.into_iter().map(|k|format!("{}:{}",serde_json::to_string(k).unwrap(),canonical(&map[k]))).collect::<Vec<_>>().join(","))},Value::Array(items)=>format!("[{}]",items.iter().map(canonical).collect::<Vec<_>>().join(",")),_=>value.to_string()} }
fn fingerprint(value:&Value)->String { format!("{:x}",Sha256::digest(canonical(value).as_bytes())) }
fn issue_confirmation(state:&State<AppState>,kind:&str,payload:&Value)->AppResult<String>{let token=Uuid::new_v4().to_string();state.confirmations.lock().map_err(|_|err("STATE_ERROR","Cannot create confirmation."))?.insert(token.clone(),Confirmation{kind:kind.into(),hash:fingerprint(payload),expires_at:Instant::now()+Duration::from_secs(120)});Ok(token)}
fn consume_confirmation(state:&State<AppState>,token:&str,kind:&str,payload:&Value)->AppResult<()> { let record=state.confirmations.lock().map_err(|_|err("STATE_ERROR","Cannot validate confirmation."))?.remove(token); match record { Some(item) if item.kind==kind&&item.hash==fingerprint(payload)&&item.expires_at>Instant::now()=>Ok(()),_=>Err(err("CONFIRMATION_EXPIRED","Confirmation expired. Preview the operation again.")) } }
fn is_admin()->bool { unsafe{IsUserAnAdmin()!=0} }
fn require_admin()->AppResult<()> { if is_admin(){Ok(())}else{Err(err("PERMISSION_REQUIRED","This operation requires administrator privileges."))} }
fn valid_index(value:&Value)->AppResult<i64>{value.as_i64().or_else(||value.as_str().and_then(|x|x.parse().ok())).filter(|x|*x>0).ok_or_else(||err("VALIDATION_ERROR","InterfaceIndex must be a positive integer."))}
fn valid_metric(value:&Value)->AppResult<i64>{value.as_i64().or_else(||value.as_str().and_then(|x|x.parse().ok())).filter(|x|*x>=1&&*x<=9999).ok_or_else(||err("VALIDATION_ERROR","Metric must be an integer from 1 to 9999."))}
fn valid_ipv4(value:&str)->bool { let parts=value.split('.').collect::<Vec<_>>();parts.len()==4&&parts.iter().all(|x|!x.is_empty()&&x.parse::<u8>().is_ok()) }
fn normalize_cidr(value:&str)->AppResult<String>{let(mut ip,prefix)=value.split_once('/').ok_or_else(||err("VALIDATION_ERROR","DestinationPrefix must be IPv4 CIDR."))?;ip=ip.trim();let prefix: u32=prefix.parse().map_err(|_|err("VALIDATION_ERROR","Invalid CIDR prefix."))?;if prefix>32||!valid_ipv4(ip){return Err(err("VALIDATION_ERROR","Invalid IPv4 CIDR."))}let nums=ip.split('.').map(|x|x.parse::<u32>().unwrap()).collect::<Vec<_>>();let number=(nums[0]<<24)|(nums[1]<<16)|(nums[2]<<8)|nums[3];let mask=if prefix==0{0}else{u32::MAX<<(32-prefix)};let n=number&mask;Ok(format!("{}.{}.{}.{}/{}",n>>24,(n>>16)&255,(n>>8)&255,n&255,prefix))}

#[tauri::command]
fn get_privilege_status()->Value{json!({"isAdmin":is_admin()})}
#[tauri::command]
fn get_app_version()->Value{json!({"version":env!("CARGO_PKG_VERSION")})}
#[tauri::command]
fn get_network_overview()->AppResult<Value>{let raw=run_powershell_json(OVERVIEW_SCRIPT,None)?;let wifi=wifi_overview().unwrap_or_default();Ok(normalize_overview(raw,wifi))}
#[tauri::command]
fn get_wifi_profiles(interface_name:String)->AppResult<Value>{let alias=valid_interface_name(&interface_name)?;let output=run_netsh(&["wlan".into(),"show".into(),"profiles".into(),format!("interface={alias}")])?;Ok(json!({"interfaceName":alias,"profiles":parse_wifi_profiles(&output),"profilesLoaded":true}))}
#[tauri::command]
fn get_routes()->AppResult<Value>{let raw=run_powershell_json(ROUTES_SCRIPT,None)?;let mut metrics=HashMap::new();for item in arrays(raw.get("metrics")){let family=field_string(&item,"addressFamily");let index=field_i64(&item,"interfaceIndex");metrics.insert(format!("{family}:{index}"),field_i64(&item,"interfaceMetric"));}
 let mut routes:BTreeMap<String,Value>=BTreeMap::new();let persistent=arrays(raw.get("persistent"));let persistent_keys=persistent.iter().map(route_key).collect::<Vec<_>>();for route in arrays(raw.get("active")).into_iter().chain(persistent){let key=route_key(&route);let old=routes.remove(&key);let mut stores=arrays(old.as_ref().and_then(|x|x.get("policyStores")));let store=field_string(&route,"policyStore");if !stores.iter().any(|x|x.as_str()==Some(&store)){stores.push(json!(store));}let family=field_string(&route,"addressFamily");let index=field_i64(&route,"interfaceIndex");let metric=metrics.get(&format!("{family}:{index}")).copied();let next=field_string(&route,"nextHop");let dest=field_string(&route,"destinationPrefix");let is_persistent=persistent_keys.contains(&key);let protocol=field_string(&route,"protocol");let default_route=dest=="0.0.0.0/0"||dest=="::/0";let on_link=next=="0.0.0.0"||next=="::";let multicast=dest=="224.0.0.0/4"||dest.to_lowercase()=="ff00::/8";let managed=is_persistent||protocol=="NetMgmt";let identity_editable=family=="IPv4"&&managed&&!on_link&&!multicast;let removable=identity_editable&&!default_route;routes.insert(key,json!({"addressFamily":family,"destinationPrefix":dest,"nextHop":next,"interfaceAlias":field_string(&route,"interfaceAlias"),"interfaceIndex":index,"routeMetric":field_i64(&route,"routeMetric"),"interfaceMetric":metric,"effectiveMetric":metric.map(|m|m+field_i64(&route,"routeMetric")),"protocol":protocol,"policyStores":stores,"persistent":is_persistent,"temporary":!is_persistent,"state":field_string(&route,"state"),"publish":field_string(&route,"publish"),"validLifetime":field_string(&route,"validLifetime"),"preferredLifetime":field_string(&route,"preferredLifetime"),"defaultRoute":default_route,"onLink":on_link,"systemManaged":!managed,"editable":true,"identityEditable":identity_editable,"removable":removable,"hiddenByDefault":multicast||(on_link&&!default_route&&!managed)}));}Ok(json!({"routes":routes.into_values().collect::<Vec<_>>(),"refreshedAt":Utc::now().to_rfc3339()}))}
fn route_key(route:&Value)->String{format!("{}|{}|{}|{}",field_string(route,"addressFamily"),field_string(route,"destinationPrefix"),field_string(route,"nextHop"),field_i64(route,"interfaceIndex"))}
fn is_app_managed_route(state:&State<AppState>,route:&Value)->bool{let key=route_key(route);config(state).managed_routes.iter().any(|item|route_key(item)==key)}
fn is_identity_editable_route(route:&Value)->bool{route.get("identityEditable").and_then(Value::as_bool).unwrap_or(false)}
fn is_removable_route(route:&Value)->bool{route.get("removable").and_then(Value::as_bool).unwrap_or(false)}
fn route_remove_payload(route:&Value)->Value{json!({"addressFamily":field_string(route,"addressFamily"),"destinationPrefix":field_string(route,"destinationPrefix"),"nextHop":field_string(route,"nextHop"),"interfaceIndex":field_i64(route,"interfaceIndex"),"routeMetric":field_i64(route,"routeMetric"),"persistent":route.get("persistent").and_then(Value::as_bool).unwrap_or(false),"policyStores":route.get("policyStores").cloned().unwrap_or(json!(["ActiveStore"]))})}
fn edit_route_payload(target:&Value,input:&Value)->AppResult<Value>{
 let metric=valid_metric(input.get("routeMetric").unwrap_or(&Value::Null))?;
 let destination=input.get("destinationPrefix").and_then(Value::as_str).unwrap_or_else(||target.get("destinationPrefix").and_then(Value::as_str).unwrap_or(""));
 let next_hop=input.get("nextHop").and_then(Value::as_str).unwrap_or_else(||target.get("nextHop").and_then(Value::as_str).unwrap_or(""));
 let index=valid_index(input.get("interfaceIndex").unwrap_or(&Value::Null))?;
 let persistent=input.get("persistent").and_then(Value::as_bool).unwrap_or_else(||target.get("persistent").and_then(Value::as_bool).unwrap_or(false));
 let identity_changed=destination!=field_string(target,"destinationPrefix")||next_hop!=field_string(target,"nextHop")||index!=field_i64(target,"interfaceIndex")||persistent!=target.get("persistent").and_then(Value::as_bool).unwrap_or(false);
 if identity_changed&&!is_identity_editable_route(target){return Err(err("VALIDATION_ERROR","This Windows-managed route only supports Route Metric editing."))}
 let route=if identity_changed{route_payload(input)?}else{json!({"addressFamily":field_string(target,"addressFamily"),"destinationPrefix":field_string(target,"destinationPrefix"),"nextHop":field_string(target,"nextHop"),"interfaceIndex":field_i64(target,"interfaceIndex"),"routeMetric":metric,"persistent":target.get("persistent").and_then(Value::as_bool).unwrap_or(false)})};
 Ok(json!({"mode":if identity_changed{"replace"}else{"metric"},"oldRoute":route_remove_payload(target),"route":route}))
}

fn metric_payload(input:&Value)->AppResult<Value>{let index=valid_index(input.get("interfaceIndex").unwrap_or(&Value::Null))?;let family=input.get("addressFamily").and_then(Value::as_str).filter(|x|*x=="IPv4"||*x=="IPv6").ok_or_else(||err("VALIDATION_ERROR","Address family must be IPv4 or IPv6."))?;let automatic=input.get("automatic").and_then(Value::as_bool).unwrap_or(false);let metric=if automatic{None}else{Some(valid_metric(input.get("metric").unwrap_or(&Value::Null))?)};Ok(json!({"interfaceIndex":index,"addressFamily":family,"automatic":automatic,"metric":metric}))}
fn route_payload(input:&Value)->AppResult<Value>{let destination=normalize_cidr(input.get("destinationPrefix").and_then(Value::as_str).ok_or_else(||err("VALIDATION_ERROR","DestinationPrefix is required."))?)?;let next=input.get("nextHop").and_then(Value::as_str).filter(|x|valid_ipv4(x)).ok_or_else(||err("VALIDATION_ERROR","NextHop must be an IPv4 address."))?;let index=valid_index(input.get("interfaceIndex").unwrap_or(&Value::Null))?;let metric=valid_metric(input.get("routeMetric").unwrap_or(&Value::Null))?;Ok(json!({"destinationPrefix":destination,"nextHop":next,"interfaceIndex":index,"routeMetric":metric,"persistent":input.get("persistent").and_then(Value::as_bool).unwrap_or(false),"note":input.get("note").and_then(Value::as_str).unwrap_or("")}))}
fn capture_baseline(state:&State<AppState>,scope:Value)->AppResult<()> { let snapshot_state=json!({"overview":get_network_overview()?,"routes":get_routes()?});let now=Utc::now().to_rfc3339();let snapshot=Snapshot{id:Uuid::new_v4().to_string(),hash:fingerprint(&snapshot_state),name:"Before latest change".into(),created_at:now.clone(),last_used_at:now,use_count:0,state:snapshot_state};let mut cfg=config(state);cfg.baseline=Some(Baseline{snapshot,scope});persist(state,&cfg) }
fn ps_set_metric(payload:&Value)->AppResult<Value>{let script=r#"$p=@{InterfaceIndex=[int]$InputData.interfaceIndex;AddressFamily=[string]$InputData.addressFamily;AutomaticMetric=$(if($InputData.automatic){'Enabled'}else{'Disabled'});Confirm=$false};if(-not $InputData.automatic){$p.InterfaceMetric=[int]$InputData.metric};Set-NetIPInterface @p -ErrorAction Stop;$r=Get-NetIPInterface -InterfaceIndex ([int]$InputData.interfaceIndex) -AddressFamily ([string]$InputData.addressFamily);[PSCustomObject]@{interfaceIndex=[int]$r.InterfaceIndex;addressFamily=[string]$r.AddressFamily;automaticMetric=([string]$r.AutomaticMetric -eq 'Enabled');interfaceMetric=[long]$r.InterfaceMetric}|ConvertTo-Json -Compress"#;run_powershell_json(script,Some(payload))}
#[tauri::command]
fn preview_metric_change(state:State<AppState>,input:Value)->AppResult<Value>{let payload=metric_payload(&input)?;let overview=get_network_overview()?;let index=field_i64(&payload,"interfaceIndex");let family=field_string(&payload,"addressFamily");let adapter=arrays(overview.get("interfaces")).into_iter().find(|x|field_i64(x,"interfaceIndex")==index).ok_or_else(||err("NOT_FOUND","Interface does not exist."))?;let key=if family=="IPv4"{"ipv4"}else{"ipv6"};let current=adapter.get("metrics").and_then(Value::as_object).and_then(|metrics|metrics.get(key)).cloned().unwrap_or(Value::Null);if current.is_null(){return Err(err("NOT_FOUND","The interface does not support this address family."))}let unchanged=current.get("automaticMetric").and_then(Value::as_bool)==payload.get("automatic").and_then(Value::as_bool)&&(payload.get("automatic").and_then(Value::as_bool).unwrap_or(false)||current.get("interfaceMetric")==payload.get("metric"));let mut warnings=vec![];if adapter.get("defaultRoute").and_then(Value::as_bool).unwrap_or(false){warnings.push("This interface carries a default gateway; changing Metric can alter the default network path.");}let token=if unchanged{None}else{Some(issue_confirmation(&state,"metric",&payload)?)};Ok(json!({"input":payload,"current":current,"unchanged":unchanged,"warnings":warnings,"confirmationToken":token,"requiresAdmin":true}))}
#[tauri::command]
fn apply_metric_change(state:State<AppState>,input:Value)->AppResult<Value>{let payload=metric_payload(input.get("metric").unwrap_or(&input))?;let token=input.get("confirmationToken").and_then(Value::as_str).ok_or_else(||err("CONFIRMATION_EXPIRED","Preview is required."))?;consume_confirmation(&state,token,"metric",&payload)?;require_admin()?;capture_baseline(&state,json!({"metrics":[payload.clone()]}))?;ps_set_metric(&payload)}

#[tauri::command]
fn preview_add_route(state:State<AppState>,input:Value)->AppResult<Value>{let payload=route_payload(&input)?;let routes=get_routes()?;let exact=arrays(routes.get("routes")).into_iter().find(|r|field_string(r,"destinationPrefix")==field_string(&payload,"destinationPrefix")&&field_string(r,"nextHop")==field_string(&payload,"nextHop")&&field_i64(r,"interfaceIndex")==field_i64(&payload,"interfaceIndex"));if exact.is_some(){return Err(err("CONFLICT","The same route already exists."))}let conflicts=arrays(routes.get("routes")).into_iter().filter(|r|field_string(r,"destinationPrefix")==field_string(&payload,"destinationPrefix")).collect::<Vec<_>>();let mut warnings=vec![];if field_string(&payload,"destinationPrefix")=="0.0.0.0/0"{warnings.push("This is a default route and can change network connectivity.");}if !conflicts.is_empty(){warnings.push("Routes with the same destination prefix already exist.");}Ok(json!({"route":payload,"conflicts":conflicts,"warnings":warnings,"confirmationToken":issue_confirmation(&state,"add-route",&payload)?}))}
#[tauri::command]
fn add_route(state:State<AppState>,input:Value)->AppResult<Value>{let payload=route_payload(input.get("route").unwrap_or(&input))?;let token=input.get("confirmationToken").and_then(Value::as_str).ok_or_else(||err("CONFIRMATION_EXPIRED","Preview is required."))?;consume_confirmation(&state,token,"add-route",&payload)?;require_admin()?;capture_baseline(&state,json!({"routes":[payload.clone()]}))?;let script=r#"$s=if($InputData.persistent){'PersistentStore'}else{'ActiveStore'};New-NetRoute -AddressFamily IPv4 -DestinationPrefix ([string]$InputData.destinationPrefix) -NextHop ([string]$InputData.nextHop) -InterfaceIndex ([int]$InputData.interfaceIndex) -RouteMetric ([int]$InputData.routeMetric) -PolicyStore $s -Confirm:$false -ErrorAction Stop;[PSCustomObject]@{created=$true;policyStore=$s}|ConvertTo-Json -Compress"#;let result=run_powershell_json(script,Some(&payload))?;let mut cfg=config(&state);if payload.get("persistent").and_then(Value::as_bool).unwrap_or(false)||true{cfg.managed_routes.push(json!({"addressFamily":"IPv4","destinationPrefix":field_string(&payload,"destinationPrefix"),"nextHop":field_string(&payload,"nextHop"),"interfaceIndex":field_i64(&payload,"interfaceIndex")}));}persist(&state,&cfg)?;Ok(result)}
#[tauri::command]
fn preview_edit_route(state:State<AppState>,input:Value)->AppResult<Value>{let old=input.get("oldRoute").ok_or_else(||err("VALIDATION_ERROR","Original route is required."))?;let routes=get_routes()?;let target=arrays(routes.get("routes")).into_iter().find(|route|route_key(route)==route_key(old)).ok_or_else(||err("NOT_FOUND","Route no longer exists."))?;let payload=edit_route_payload(&target,input.get("route").unwrap_or(&Value::Null))?;let new_route=payload.get("route").cloned().unwrap_or(Value::Null);let conflicts=if field_string(&payload,"mode")=="replace"{arrays(routes.get("routes")).into_iter().filter(|route|route_key(route)!=route_key(&target)&&field_string(route,"destinationPrefix")==field_string(&new_route,"destinationPrefix")).collect::<Vec<_>>()}else{vec![]};let mut warnings=vec![];if target.get("defaultRoute").and_then(Value::as_bool).unwrap_or(false){warnings.push("This is a default route. Changing it can immediately interrupt network connectivity.".to_string());}if target.get("systemManaged").and_then(Value::as_bool).unwrap_or(false)||target.get("onLink").and_then(Value::as_bool).unwrap_or(false){warnings.push("Windows or the owning network software may recreate or overwrite this route.".to_string());}if field_string(&payload,"mode")=="replace"{warnings.push("The route identity will be replaced. Route selection may change while the operation is applied.".to_string());}if !conflicts.is_empty(){warnings.push("Routes with the same destination prefix already exist.".into());}warnings.push("If connectivity is affected, reopen the app as administrator and use Reset to pre-change baseline.".into());Ok(json!({"oldRoute":target,"route":new_route,"mode":payload["mode"],"conflicts":conflicts,"warnings":warnings,"confirmationToken":issue_confirmation(&state,"edit-route",&payload)?}))}
#[tauri::command]
fn apply_edit_route(state:State<AppState>,input:Value)->AppResult<Value>{let raw_old=input.get("oldRoute").ok_or_else(||err("VALIDATION_ERROR","Original route is required."))?;let routes=get_routes()?;let target=arrays(routes.get("routes")).into_iter().find(|route|route_key(route)==route_key(raw_old)).ok_or_else(||err("NOT_FOUND","Route no longer exists."))?;let was_app_managed=is_app_managed_route(&state,&target);let payload=edit_route_payload(&target,input.get("route").unwrap_or(&Value::Null))?;consume_confirmation(&state,input.get("confirmationToken").and_then(Value::as_str).unwrap_or(""),"edit-route",&payload)?;require_admin()?;capture_baseline(&state,json!({"editRoute":{"oldRoute":target,"mode":payload["mode"],"route":payload["route"]}}))?;let old=payload.get("oldRoute").cloned().unwrap_or(Value::Null);let new_route=payload.get("route").cloned().unwrap_or(Value::Null);if field_string(&payload,"mode")=="metric"{let mut update=old.clone();update.as_object_mut().ok_or_else(||err("VALIDATION_ERROR","Invalid route record."))?.insert("routeMetric".into(),new_route.get("routeMetric").cloned().unwrap_or(Value::Null));let result=update_route_metric_record(&update)?;let completed=arrays(Some(&result)).iter().all(|item|field_string(item,"status")=="success");return Ok(json!({"updated":result,"completed":completed}))}let same_key=route_key(&old)==route_key(&new_route);let (added,removed)=if same_key{let removed=remove_route_record(&old)?;let removed_ok=arrays(Some(&removed)).iter().all(|item|field_string(item,"status")=="success");if !removed_ok{return Ok(json!({"removed":removed,"completed":false}))}match add_route_record(&new_route){Ok(added)=>(added,removed),Err(error)=>{let _=add_route_record(&old);return Err(error)}}}else{let added=add_route_record(&new_route)?;let removed=remove_route_record(&old)?;(added,removed)};let removed_ok=arrays(Some(&removed)).iter().all(|item|field_string(item,"status")=="success");if removed_ok&&was_app_managed{let mut cfg=config(&state);let old_key=route_key(&old);cfg.managed_routes.retain(|item|route_key(item)!=old_key);cfg.managed_routes.push(json!({"addressFamily":"IPv4","destinationPrefix":field_string(&new_route,"destinationPrefix"),"nextHop":field_string(&new_route,"nextHop"),"interfaceIndex":field_i64(&new_route,"interfaceIndex")}));persist(&state,&cfg)?;}Ok(json!({"added":added,"removed":removed,"completed":removed_ok}))}
#[tauri::command]
fn preview_remove_route(state:State<AppState>,route:Value)->AppResult<Value>{let routes=get_routes()?;let target=arrays(routes.get("routes")).into_iter().find(|r|route_key(r)==route_key(&route)).ok_or_else(||err("NOT_FOUND","Route no longer exists."))?;if !is_removable_route(&target){return Err(err("VALIDATION_ERROR","This route is Windows-managed, on-link, multicast, IPv6, or a default route and cannot be safely removed."))}let payload=route_remove_payload(&target);Ok(json!({"route":target,"warnings":["If connectivity is affected, reopen the app as administrator and use Reset to pre-change baseline."],"confirmationToken":issue_confirmation(&state,"remove-route",&payload)?}))}
#[tauri::command]
fn remove_route(state:State<AppState>,input:Value)->AppResult<Value>{let requested=input.get("route").cloned().unwrap_or(input.clone());let routes=get_routes()?;let target=arrays(routes.get("routes")).into_iter().find(|route|route_key(route)==route_key(&requested)).ok_or_else(||err("NOT_FOUND","Route no longer exists."))?;if !is_removable_route(&target){return Err(err("VALIDATION_ERROR","This route can no longer be safely removed."))}let payload=route_remove_payload(&target);let token=input.get("confirmationToken").and_then(Value::as_str).ok_or_else(||err("CONFIRMATION_EXPIRED","Preview is required."))?;consume_confirmation(&state,token,"remove-route",&payload)?;require_admin()?;capture_baseline(&state,json!({"removeRoute":target}))?;let result=remove_route_record(&payload)?;let completed=arrays(Some(&result)).iter().all(|item|field_string(item,"status")=="success");if completed{let mut cfg=config(&state);let key=route_key(&payload);cfg.managed_routes.retain(|item|route_key(item)!=key);persist(&state,&cfg)?;}Ok(json!({"results":result,"completed":completed}))}

#[tauri::command]
fn connect_wifi(state:State<AppState>,input:Value)->AppResult<Value>{let alias=input.get("interfaceName").and_then(Value::as_str).filter(|x|!x.contains(['\r','\n','\0'])).ok_or_else(||err("VALIDATION_ERROR","Invalid Wi-Fi interface name."))?.to_string();let profile=input.get("profileName").and_then(Value::as_str).filter(|x|!x.contains(['\r','\n','\0'])).ok_or_else(||err("VALIDATION_ERROR","Invalid Wi-Fi profile name."))?.to_string();capture_baseline(&state,json!({"wifi":[{"interfaceName":alias,"profileName":profile}]}))?;run_netsh(&["wlan".into(),"connect".into(),format!("name={profile}"),format!("interface={alias}")])?;Ok(json!({"message":format!("Connection request sent to {alias}.")}))}
#[tauri::command]
fn disconnect_wifi(state:State<AppState>,input:Value)->AppResult<Value>{let alias=input.get("interfaceName").and_then(Value::as_str).filter(|x|!x.contains(['\r','\n','\0'])).ok_or_else(||err("VALIDATION_ERROR","Invalid Wi-Fi interface name."))?.to_string();capture_baseline(&state,json!({"wifi":[{"interfaceName":alias}]}))?;run_netsh(&["wlan".into(),"disconnect".into(),format!("interface={alias}")])?;Ok(json!({"message":format!("Disconnect request sent to {alias}.")}))}

fn adapter_control_payload(input:&Value)->AppResult<Value>{let index=valid_index(input.get("interfaceIndex").unwrap_or(&Value::Null))?;let action=input.get("action").and_then(Value::as_str).filter(|value|*value=="enable"||*value=="disable").ok_or_else(||err("VALIDATION_ERROR","Adapter action must be enable or disable."))?;Ok(json!({"interfaceIndex":index,"action":action}))}
#[tauri::command]
fn preview_adapter_control(state:State<AppState>,input:Value)->AppResult<Value>{let payload=adapter_control_payload(&input)?;let overview=get_network_overview()?;let adapter=arrays(overview.get("interfaces")).into_iter().find(|item|field_i64(item,"interfaceIndex")==field_i64(&payload,"interfaceIndex")).ok_or_else(||err("NOT_FOUND","Interface no longer exists."))?;if adapter.get("systemInterface").and_then(Value::as_bool).unwrap_or(false){return Err(err("VALIDATION_ERROR","System interfaces cannot be enabled or disabled."))}let mut warnings=vec![];if payload.get("action").and_then(Value::as_str)==Some("disable"){warnings.push("Disabling this adapter can interrupt network connectivity.".to_string());if adapter.get("defaultRoute").and_then(Value::as_bool).unwrap_or(false){warnings.push("This adapter currently has a default gateway and may be your active network path.".to_string());}}Ok(json!({"adapter":adapter,"action":payload["action"],"warnings":warnings,"requiresAdmin":true,"confirmationToken":issue_confirmation(&state,"adapter-control",&payload)?}))}
#[tauri::command]
fn apply_adapter_control(state:State<AppState>,input:Value)->AppResult<Value>{let payload=adapter_control_payload(input.get("adapter").unwrap_or(&input))?;consume_confirmation(&state,input.get("confirmationToken").and_then(Value::as_str).unwrap_or(""),"adapter-control",&payload)?;require_admin()?;capture_baseline(&state,json!({"adapterControl":payload.clone()}))?;let script=r#"$item=Get-NetAdapter -IncludeHidden -InterfaceIndex ([int]$InputData.interfaceIndex) -ErrorAction Stop;if($InputData.action -eq 'enable'){$item|Enable-NetAdapter -Confirm:$false -ErrorAction Stop}else{$item|Disable-NetAdapter -Confirm:$false -ErrorAction Stop};$result=Get-NetAdapter -IncludeHidden -InterfaceIndex ([int]$InputData.interfaceIndex);[PSCustomObject]@{interfaceIndex=[int]$result.InterfaceIndex;adminStatus=[string]$result.AdminStatus;status=[string]$result.Status}|ConvertTo-Json -Compress"#;run_powershell_json(script,Some(&payload))}
#[tauri::command]
fn get_app_config(state:State<AppState>)->Value{let cfg=config(&state);json!({"settings":cfg.settings,"profiles":cfg.profiles,"recentSnapshots":cfg.recent_snapshots,"managedRoutes":cfg.managed_routes,"hasBaseline":cfg.baseline.is_some()})}
#[tauri::command]
fn set_language(state:State<AppState>,language:String)->AppResult<Settings>{if language!="en"&&language!="zh-CN"{return Err(err("VALIDATION_ERROR","Unsupported language."))}let mut cfg=config(&state);cfg.settings.language=language;persist(&state,&cfg)?;Ok(cfg.settings)}
#[tauri::command]
fn set_theme(state:State<AppState>,theme:String)->AppResult<Settings>{let theme=valid_theme(&theme)?.to_string();let mut cfg=config(&state);cfg.settings.theme=theme;persist(&state,&cfg)?;Ok(cfg.settings)}
#[tauri::command]
fn save_profile(state:State<AppState>,profile:Value)->AppResult<Value>{let name=profile.get("name").and_then(Value::as_str).map(str::trim).filter(|x|!x.is_empty()&&x.len()<=100).ok_or_else(||err("VALIDATION_ERROR","Configuration name is required."))?;let mut saved=profile.clone();let now=Utc::now().to_rfc3339();let id=saved.get("id").and_then(Value::as_str).filter(|x|!x.is_empty()).unwrap_or("").to_string();if id.is_empty(){saved.as_object_mut().unwrap().insert("id".into(),json!(Uuid::new_v4().to_string()));saved.as_object_mut().unwrap().insert("createdAt".into(),json!(now));}saved.as_object_mut().unwrap().insert("name".into(),json!(name));saved.as_object_mut().unwrap().insert("updatedAt".into(),json!(now));let id=field_string(&saved,"id");let mut cfg=config(&state);if let Some(item)=cfg.profiles.iter_mut().find(|item|field_string(item,"id")==id){*item=saved.clone()}else{cfg.profiles.push(saved.clone())}persist(&state,&cfg)?;Ok(saved)}
#[tauri::command]
fn delete_profile(state:State<AppState>,id:String)->AppResult<Value>{let mut cfg=config(&state);let before=cfg.profiles.len();cfg.profiles.retain(|x|field_string(x,"id")!=id);if cfg.profiles.len()==before{return Err(err("NOT_FOUND","Configuration does not exist."))}persist(&state,&cfg)?;Ok(json!({"deleted":true}))}
#[tauri::command]
fn restore_profile_preview(state:State<AppState>,id:String)->AppResult<Value>{let cfg=config(&state);let profile=cfg.profiles.iter().find(|item|field_string(item,"id")==id).ok_or_else(||err("NOT_FOUND","Configuration does not exist."))?;let snapshot=profile.get("state").ok_or_else(||err("VALIDATION_ERROR","This configuration has no restorable network state."))?;let payload=json!({"id":id,"hash":fingerprint(snapshot)});Ok(json!({"profile":profile,"summary":"The saved configuration will restore its Metric, Wi-Fi, and app-managed-route state.","confirmationToken":issue_confirmation(&state,"restore-profile",&payload)?}))}
#[tauri::command]
fn restore_profile_apply(state:State<AppState>,input:Value)->AppResult<Value>{let id=input.get("id").and_then(Value::as_str).ok_or_else(||err("VALIDATION_ERROR","Configuration id is required."))?;let cfg=config(&state);let profile=cfg.profiles.iter().find(|item|field_string(item,"id")==id).ok_or_else(||err("NOT_FOUND","Configuration does not exist."))?;let snapshot=profile.get("state").ok_or_else(||err("VALIDATION_ERROR","This configuration has no restorable network state."))?.clone();let payload=json!({"id":id,"hash":fingerprint(&snapshot)});consume_confirmation(&state,input.get("confirmationToken").and_then(Value::as_str).unwrap_or(""),"restore-profile",&payload)?;require_admin()?;capture_baseline(&state,json!({"restoreProfile":id}))?;store_recent(&state,"Before saved configuration restore".into(),snapshot_current()?)?;restore_snapshot(&state,&snapshot)}

fn snapshot_current()->AppResult<Value>{Ok(json!({"overview":get_network_overview()?,"routes":get_routes()?}))}
fn remove_route_record(route:&Value)->AppResult<Value>{
  let stores=arrays(route.get("policyStores")); let mut results=vec![];
  for store in stores { let mut input=route.clone(); input.as_object_mut().ok_or_else(||err("VALIDATION_ERROR","Invalid route record."))?.insert("policyStore".into(),store);
    let script=r#"Remove-NetRoute -AddressFamily ([string]$InputData.addressFamily) -DestinationPrefix ([string]$InputData.destinationPrefix) -NextHop ([string]$InputData.nextHop) -InterfaceIndex ([int]$InputData.interfaceIndex) -PolicyStore ([string]$InputData.policyStore) -Confirm:$false -ErrorAction Stop;[PSCustomObject]@{removed=$true;policyStore=$InputData.policyStore}|ConvertTo-Json -Compress"#;
    match run_powershell_json(script,Some(&input)){Ok(value)=>results.push(json!({"status":"success","route":route,"result":value})),Err(error)=>results.push(json!({"status":"failed","route":route,"error":error}))}
  } Ok(json!(results))
}
fn update_route_metric_record(route:&Value)->AppResult<Value>{
  let script=r#"$results=@();foreach($store in @($InputData.policyStores)){try{Set-NetRoute -AddressFamily ([string]$InputData.addressFamily) -DestinationPrefix ([string]$InputData.destinationPrefix) -NextHop ([string]$InputData.nextHop) -InterfaceIndex ([int]$InputData.interfaceIndex) -PolicyStore ([string]$store) -RouteMetric ([int]$InputData.routeMetric) -Confirm:$false -ErrorAction Stop;$results+=[PSCustomObject]@{status='success';policyStore=[string]$store}}catch{$results+=[PSCustomObject]@{status='failed';policyStore=[string]$store;message=$_.Exception.Message}}};$results|ConvertTo-Json -Compress"#;
  run_powershell_json(script,Some(route))
}
fn add_route_record(route:&Value)->AppResult<Value>{
  let script=r#"$s=if($InputData.persistent){'PersistentStore'}else{'ActiveStore'};New-NetRoute -AddressFamily IPv4 -DestinationPrefix ([string]$InputData.destinationPrefix) -NextHop ([string]$InputData.nextHop) -InterfaceIndex ([int]$InputData.interfaceIndex) -RouteMetric ([int]$InputData.routeMetric) -PolicyStore $s -Confirm:$false -ErrorAction Stop;[PSCustomObject]@{created=$true;policyStore=$s}|ConvertTo-Json -Compress"#;
  run_powershell_json(script,Some(route))
}
fn restore_route_scope(scope:&Value)->AppResult<Value>{
  let current=get_routes()?;
  if let Some(edit)=scope.get("editRoute"){
    let old=edit.get("oldRoute").ok_or_else(||err("VALIDATION_ERROR","Baseline route data is incomplete."))?;
    let new_route=edit.get("route").ok_or_else(||err("VALIDATION_ERROR","Baseline route data is incomplete."))?;
    if field_string(edit,"mode")=="metric"{
      let actual=arrays(current.get("routes")).into_iter().find(|route|route_key(route)==route_key(old)).ok_or_else(||err("NOT_FOUND","The edited route no longer exists."))?;
      let mut update=route_remove_payload(&actual);update.as_object_mut().unwrap().insert("routeMetric".into(),old.get("routeMetric").cloned().unwrap_or(json!(1)));
      return update_route_metric_record(&update)
    }
    let mut results=vec![];
    if let Some(actual)=arrays(current.get("routes")).into_iter().find(|route|route_key(route)==route_key(new_route)){results.extend(arrays(Some(&remove_route_record(&route_remove_payload(&actual))?)));}
    let refreshed=get_routes()?;
    if !arrays(refreshed.get("routes")).iter().any(|route|route_key(route)==route_key(old)){results.push(json!({"status":"success","kind":"restore-route","result":add_route_record(old)?}));}
    return Ok(json!(results))
  }
  if let Some(old)=scope.get("removeRoute"){
    if arrays(current.get("routes")).iter().any(|route|route_key(route)==route_key(old)){return Ok(json!([{"status":"skipped","kind":"restore-route","message":"Route already exists."}]))}
    return Ok(json!([{"status":"success","kind":"restore-route","result":add_route_record(old)?}]))
  }
  Ok(json!([]))
}
fn restore_snapshot(state:&State<AppState>, snapshot:&Value)->AppResult<Value>{
  let target_interfaces=arrays(snapshot.pointer("/overview/interfaces")); let current=get_network_overview()?; let current_interfaces=arrays(current.get("interfaces")); let mut results=vec![];
  for target in target_interfaces { let index=field_i64(&target,"interfaceIndex"); if !current_interfaces.iter().any(|item|field_i64(item,"interfaceIndex")==index){results.push(json!({"status":"skipped","kind":"interface","interfaceIndex":index,"message":"Interface no longer exists."}));continue}
    for (family,key) in [("IPv4","ipv4"),("IPv6","ipv6")] { if let Some(metric)=target.pointer(&format!("/metrics/{key}")){ if metric.is_null(){continue} let payload=json!({"interfaceIndex":index,"addressFamily":family,"automatic":metric.get("automaticMetric").and_then(Value::as_bool).unwrap_or(true),"metric":metric.get("interfaceMetric").and_then(Value::as_i64)}); match ps_set_metric(&payload){Ok(value)=>results.push(json!({"status":"success","kind":"metric","result":value})),Err(error)=>results.push(json!({"status":"failed","kind":"metric","interfaceIndex":index,"addressFamily":family,"error":error}))} } }
    if let Some(wifi)=target.get("wifi"){let alias=field_string(&target,"alias");let connected=wifi.get("connected").and_then(Value::as_bool).unwrap_or(false);let profile=field_string(wifi,"currentProfile");let wifi_result=if connected&&!profile.is_empty(){run_netsh(&["wlan".into(),"connect".into(),format!("name={profile}"),format!("interface={alias}")])}else if !connected{run_netsh(&["wlan".into(),"disconnect".into(),format!("interface={alias}")])}else{continue};match wifi_result{Ok(_)=>results.push(json!({"status":"success","kind":"wifi","interface":alias})),Err(error)=>results.push(json!({"status":"failed","kind":"wifi","interface":alias,"error":error}))}}
  }
  let cfg=config(state); let current_routes=get_routes()?; let current_route_items=arrays(current_routes.get("routes")); let target_routes=arrays(snapshot.pointer("/routes/routes"));
  for managed in cfg.managed_routes { let key=route_key(&managed); let current_item=current_route_items.iter().find(|item|route_key(item)==key); let target_item=target_routes.iter().find(|item|route_key(item)==key); if current_item.is_some()&&target_item.is_none(){let mut remove=managed.clone();remove.as_object_mut().unwrap().insert("policyStores".into(),current_item.and_then(|x|x.get("policyStores")).cloned().unwrap_or(json!(["ActiveStore"])));results.extend(arrays(Some(&remove_route_record(&remove)?)));} else if current_item.is_none(){if let Some(route)=target_item {let add=json!({"destinationPrefix":field_string(route,"destinationPrefix"),"nextHop":field_string(route,"nextHop"),"interfaceIndex":field_i64(route,"interfaceIndex"),"routeMetric":field_i64(route,"routeMetric"),"persistent":route.get("persistent").and_then(Value::as_bool).unwrap_or(false)});match add_route_record(&add){Ok(value)=>results.push(json!({"status":"success","kind":"route","result":value})),Err(error)=>results.push(json!({"status":"failed","kind":"route","route":add,"error":error}))}}} }
  Ok(json!({"restored":true,"results":results,"currentState":snapshot_current()?}))
}
fn store_recent(state:&State<AppState>,name:String,snapshot:Value)->AppResult<Snapshot>{let hash=fingerprint(&snapshot);let now=Utc::now().to_rfc3339();let mut cfg=config(state);if let Some(index)=cfg.recent_snapshots.iter().position(|x|x.hash==hash){let mut item=cfg.recent_snapshots.remove(index);item.last_used_at=now;item.use_count+=1;item.name=name;cfg.recent_snapshots.insert(0,item.clone());persist(state,&cfg)?;return Ok(item)}let item=Snapshot{id:Uuid::new_v4().to_string(),hash,name,created_at:now.clone(),last_used_at:now,use_count:1,state:snapshot};cfg.recent_snapshots.insert(0,item.clone());cfg.recent_snapshots.truncate(10);persist(state,&cfg)?;Ok(item)}
#[tauri::command]
fn capture_recent_snapshot(state:State<AppState>,name:Option<String>)->AppResult<Snapshot>{store_recent(&state,name.unwrap_or_else(||"Current network state".into()),snapshot_current()?)}
#[tauri::command]
fn restore_recent_preview(state:State<AppState>,id:String)->AppResult<Value>{let cfg=config(&state);let item=cfg.recent_snapshots.iter().find(|x|x.id==id).ok_or_else(||err("NOT_FOUND","Recent configuration not found."))?;Ok(json!({"snapshot":item,"summary":"The snapshot will restore saved Metric settings and app-managed routes. Wi-Fi actions require explicit confirmation.","confirmationToken":issue_confirmation(&state,"restore-recent",&json!({"id":id,"hash":item.hash}))?}))}
#[tauri::command]
fn restore_recent_apply(state:State<AppState>,input:Value)->AppResult<Value>{let id=input.get("id").and_then(Value::as_str).ok_or_else(||err("VALIDATION_ERROR","Snapshot id is required."))?;let cfg=config(&state);let item=cfg.recent_snapshots.iter().find(|x|x.id==id).ok_or_else(||err("NOT_FOUND","Recent configuration not found."))?.clone();let payload=json!({"id":id,"hash":item.hash});consume_confirmation(&state,input.get("confirmationToken").and_then(Value::as_str).unwrap_or(""),"restore-recent",&payload)?;require_admin()?;capture_baseline(&state,json!({"restoreRecent":id}))?;store_recent(&state,"Before recent restore".into(),snapshot_current()?)?;restore_snapshot(&state,&item.state)}

#[tauri::command]
fn reset_preview(state:State<AppState>)->AppResult<Value>{let cfg=config(&state);let baseline=cfg.baseline.ok_or_else(||err("NOT_FOUND","No pre-change baseline is available."))?;Ok(json!({"baseline":baseline.snapshot,"scope":baseline.scope,"confirmationToken":issue_confirmation(&state,"reset-baseline",&json!({"hash":baseline.snapshot.hash,"scope":baseline.scope}))?}))}
#[tauri::command]
fn reset_apply(state:State<AppState>,input:Value)->AppResult<Value>{let mut cfg=config(&state);let baseline=cfg.baseline.clone().ok_or_else(||err("NOT_FOUND","No pre-change baseline is available."))?;let payload=json!({"hash":baseline.snapshot.hash,"scope":baseline.scope});consume_confirmation(&state,input.get("confirmationToken").and_then(Value::as_str).unwrap_or(""),"reset-baseline",&payload)?;require_admin()?;store_recent(&state,"Before baseline reset".into(),snapshot_current()?)?;let result=restore_snapshot(&state,&baseline.snapshot.state)?;let route_result=restore_route_scope(&baseline.scope)?;cfg.baseline=None;persist(&state,&cfg)?;Ok(json!({"reset":true,"result":result,"routeResult":route_result}))}
#[tauri::command]
fn windows_defaults_preview(state:State<AppState>)->AppResult<Value>{let overview=get_network_overview()?;let cfg=config(&state);let interfaces=arrays(overview.get("interfaces")).into_iter().filter(|x|!x.get("systemInterface").and_then(Value::as_bool).unwrap_or(false)).map(|x|json!({"interfaceIndex":field_i64(&x,"interfaceIndex"),"alias":field_string(&x,"alias")})).collect::<Vec<_>>();Ok(json!({"metrics":"Enable AutomaticMetric for selected IPv4/IPv6 interfaces.","routes":"Remove only routes recorded as app-managed.","wifi":"Wi-Fi is unchanged unless explicitly selected in the confirmation dialog.","interfaces":interfaces,"managedRoutes":cfg.managed_routes,"confirmationToken":issue_confirmation(&state,"windows-defaults",&json!({"interfaces":interfaces,"managedRoutes":cfg.managed_routes}))?}))}
#[tauri::command]
fn windows_defaults_apply(state:State<AppState>,input:Value)->AppResult<Value>{let overview=get_network_overview()?;let cfg=config(&state);let interfaces=arrays(overview.get("interfaces")).into_iter().filter(|x|!x.get("systemInterface").and_then(Value::as_bool).unwrap_or(false)).map(|x|json!({"interfaceIndex":field_i64(&x,"interfaceIndex"),"alias":field_string(&x,"alias")})).collect::<Vec<_>>();let payload=json!({"interfaces":interfaces,"managedRoutes":cfg.managed_routes});consume_confirmation(&state,input.get("confirmationToken").and_then(Value::as_str).unwrap_or(""),"windows-defaults",&payload)?;require_admin()?;capture_baseline(&state,json!({"windowsDefaults":true}))?;let script=r#"$results=@();foreach($i in @($InputData.interfaces)){foreach($family in @('IPv4','IPv6')){try{Set-NetIPInterface -InterfaceIndex ([int]$i.interfaceIndex) -AddressFamily $family -AutomaticMetric Enabled -Confirm:$false -ErrorAction Stop;$results+=[PSCustomObject]@{interfaceIndex=$i.interfaceIndex;addressFamily=$family;status='success'}}catch{$results+=[PSCustomObject]@{interfaceIndex=$i.interfaceIndex;addressFamily=$family;status='skipped'}}}};$results|ConvertTo-Json -Compress"#;let metric_results=run_powershell_json(script,Some(&payload))?;let current_routes=get_routes()?;let mut route_results=vec![];for managed in arrays(payload.get("managedRoutes")){if let Some(actual)=arrays(current_routes.get("routes")).into_iter().find(|route|route_key(route)==route_key(&managed)){let mut route=managed.clone();route.as_object_mut().unwrap().insert("policyStores".into(),actual.get("policyStores").cloned().unwrap_or(json!(["ActiveStore"])));route_results.extend(arrays(Some(&remove_route_record(&route)?)));}}let mut new_cfg=config(&state);new_cfg.managed_routes.clear();persist(&state,&new_cfg)?;Ok(json!({"metrics":metric_results,"routeResults":route_results,"message":"Windows-managed automatic Metric has been enabled. Only routes created by this app were removed."}))}

fn safe_config_path(path:&str)->AppResult<PathBuf>{let result=PathBuf::from(path);if result.extension().and_then(|x|x.to_str()).map(|x|x.eq_ignore_ascii_case("json")).unwrap_or(false){Ok(result)}else{Err(err("VALIDATION_ERROR","Configuration files must use the .json extension."))}}
#[tauri::command]
fn export_configuration(state:State<AppState>,path:String,mode:String,id:Option<String>)->AppResult<Value>{let path=safe_config_path(&path)?;let cfg=config(&state);let content=match mode.as_str(){"profile"=>json!({"format":"wnm-config","version":2,"profiles":cfg.profiles.into_iter().filter(|x|Some(field_string(x,"id"))==id).collect::<Vec<_>>() }),"recent"=>json!({"format":"wnm-config","version":2,"recentSnapshots":cfg.recent_snapshots.into_iter().filter(|x|Some(x.id.clone())==id).collect::<Vec<_>>() }),_=>json!({"format":"wnm-config","version":2,"settings":cfg.settings,"profiles":cfg.profiles,"recentSnapshots":cfg.recent_snapshots})};fs::write(&path,serde_json::to_string_pretty(&content).map_err(|_|err("SERIALIZE_ERROR","Cannot export configuration."))?).map_err(|e|err_with("EXPORT_FAILED","Cannot write export file.",json!({"reason":e.to_string()})))?;Ok(json!({"exported":true,"path":path}))}
#[tauri::command]
fn import_configuration(state:State<AppState>,path:String)->AppResult<Value>{let path=safe_config_path(&path)?;let meta=fs::metadata(&path).map_err(|_|err("IMPORT_FAILED","Cannot access import file."))?;if meta.len()>MAX_IMPORT_BYTES{return Err(err("VALIDATION_ERROR","Import file is too large."))}let value:Value=serde_json::from_str(&fs::read_to_string(&path).map_err(|_|err("IMPORT_FAILED","Cannot read import file."))?).map_err(|_|err("VALIDATION_ERROR","Import file is not valid JSON."))?;if value.get("format").and_then(Value::as_str)!=Some("wnm-config"){return Err(err("VALIDATION_ERROR","Unsupported configuration format."))}let mut cfg=config(&state);let mut added=0;for mut profile in arrays(value.get("profiles")){let hash=fingerprint(&profile);if cfg.profiles.iter().any(|x|fingerprint(x)==hash){continue}let name=field_string(&profile,"name");if cfg.profiles.iter().any(|x|field_string(x,"name")==name){profile.as_object_mut().unwrap().insert("name".into(),json!(format!("{} (Imported)",name)));}profile.as_object_mut().unwrap().insert("id".into(),json!(Uuid::new_v4().to_string()));cfg.profiles.push(profile);added+=1;}for snapshot in arrays(value.get("recentSnapshots")){if let Ok(item)=serde_json::from_value::<Snapshot>(snapshot){if !cfg.recent_snapshots.iter().any(|x|x.hash==item.hash){cfg.recent_snapshots.push(item)}}}cfg.recent_snapshots.sort_by(|a,b|b.last_used_at.cmp(&a.last_used_at));cfg.recent_snapshots.truncate(10);persist(&state,&cfg)?;Ok(json!({"imported":added,"recentSnapshots":cfg.recent_snapshots.len()}))}
#[tauri::command]
fn get_diagnostics(state:State<AppState>)->AppResult<Value>{let overview=get_network_overview()?;let routes=get_routes()?;let cfg=config(&state);let os=run_powershell_json("Get-CimInstance Win32_OperatingSystem|Select-Object Caption,Version,BuildNumber,OSArchitecture|ConvertTo-Json -Compress",None).unwrap_or(Value::Null);let changes=arrays(overview.get("interfaces")).into_iter().filter(|x|x.pointer("/deviations/manualIpv4Metric").and_then(Value::as_bool).unwrap_or(false)||x.pointer("/deviations/manualIpv6Metric").and_then(Value::as_bool).unwrap_or(false)||x.pointer("/deviations/adapterDisabled").and_then(Value::as_bool).unwrap_or(false)).collect::<Vec<_>>();let static_routes=arrays(routes.get("routes")).into_iter().filter(|x|x.get("persistent").and_then(Value::as_bool).unwrap_or(false)||field_string(x,"protocol")=="NetMgmt").collect::<Vec<_>>();Ok(json!({"generatedAt":Utc::now().to_rfc3339(),"os":os,"privilege":{"isAdmin":is_admin()},"changes":{"interfaces":changes,"staticRoutes":static_routes},"overview":overview,"routes":routes,"app":{"schemaVersion":cfg.schema_version,"recentSnapshots":cfg.recent_snapshots.len()}}))}
#[tauri::command]
fn restart_as_administrator(app:AppHandle)->AppResult<Value>{
  if is_admin(){return Ok(json!({"started":false,"alreadyAdmin":true}))}
  let exe=std::env::current_exe().map_err(|_|err("COMMAND_FAILED","Cannot locate the application executable."))?;
  let verb=wide_null(OsStr::new("runas")); let executable=wide_null(exe.as_os_str());
  let result=unsafe{ShellExecuteW(ptr::null_mut(),verb.as_ptr(),executable.as_ptr(),ptr::null(),ptr::null(),SW_SHOWNORMAL)};
  if result as isize<=32{return Err(err_with("ELEVATION_FAILED","Windows could not restart the application as administrator.",json!({"shellExecuteCode":result as isize,"hint":"Accept the Windows UAC prompt. Installation in a user folder is supported."})))}
  app.exit(0); Ok(json!({"started":true}))
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test] fn cidr_is_normalized(){assert_eq!(normalize_cidr("172.31.9.10/12").unwrap(),"172.16.0.0/12");}
  #[test] fn invalid_metric_is_rejected(){assert!(valid_metric(&json!(0)).is_err());assert_eq!(valid_metric(&json!(5)).unwrap(),5);}
  #[test] fn theme_and_interface_validation_are_strict(){assert_eq!(valid_theme("dark").unwrap(),"dark");assert!(valid_theme("blue").is_err());assert!(valid_interface_name("Wi-Fi\nname").is_err());assert_eq!(Settings::default().theme,"system");}
  #[test] fn adapter_control_validation_is_strict(){assert_eq!(field_string(&adapter_control_payload(&json!({"interfaceIndex":7,"action":"disable"})).unwrap(),"action"),"disable");assert!(adapter_control_payload(&json!({"interfaceIndex":0,"action":"disable"})).is_err());assert!(adapter_control_payload(&json!({"interfaceIndex":7,"action":"restart"})).is_err());}
  #[test] fn route_editing_allows_metric_but_protects_system_identity(){let system=json!({"addressFamily":"IPv6","destinationPrefix":"::/0","nextHop":"fe80::1","interfaceIndex":7,"routeMetric":25,"persistent":false,"policyStores":["ActiveStore"],"editable":true,"identityEditable":false,"removable":false});let metric=edit_route_payload(&system,&json!({"destinationPrefix":"::/0","nextHop":"fe80::1","interfaceIndex":7,"routeMetric":30,"persistent":false})).unwrap();assert_eq!(field_string(&metric,"mode"),"metric");assert!(edit_route_payload(&system,&json!({"destinationPrefix":"2001:db8::/32","nextHop":"fe80::1","interfaceIndex":7,"routeMetric":30,"persistent":false})).is_err());}
  #[test] fn existing_static_route_can_be_fully_edited(){let route=json!({"addressFamily":"IPv4","destinationPrefix":"172.16.0.0/12","nextHop":"192.168.3.1","interfaceIndex":24,"routeMetric":40,"persistent":true,"policyStores":["ActiveStore","PersistentStore"],"editable":true,"identityEditable":true,"removable":true});let edit=edit_route_payload(&route,&json!({"destinationPrefix":"172.16.0.0/12","nextHop":"192.168.3.254","interfaceIndex":24,"routeMetric":20,"persistent":true})).unwrap();assert_eq!(field_string(&edit,"mode"),"replace");}
  #[test] fn ui_contains_route_adapter_actions_and_version(){let ui=include_str!("../frontend/main.js");let html=include_str!("../frontend/index.html");assert!(ui.contains("action.append(edit)"));assert!(ui.contains("box.append(enable,disable)"));assert!(html.contains("id=\"app-version\""));assert!(html.contains("v1.1.3"));}
  #[test] fn canonical_hash_ignores_object_order(){assert_eq!(fingerprint(&json!({"a":1,"b":2})),fingerprint(&json!({"b":2,"a":1})));}
  #[test] fn wifi_parser_handles_chinese(){let items=parse_wifi_interfaces("名称 : WLAN\n状态 : 已连接\nSSID : 测试网络\n");assert_eq!(items[0]["alias"],"WLAN");assert_eq!(items[0]["connected"],true);}
}
