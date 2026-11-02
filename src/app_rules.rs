//! Per-app rules for v2rayN. The user lists programs (by file name, full path
//! or folder) and picks where their traffic goes: v2rayN's proxy, direct,
//! block, or a local proxy such as `localhost:8889`. The app only produces text
//! for v2rayN's "Import Rules From Clipboard" and a `socks://` link for the
//! local proxy; it writes nothing to v2rayN, the system proxy or Windows.
//!
//! Xray (v2rayN's default core) matches a process by file name, by full path,
//! or by folder when the entry ends with "/". Matching is case-sensitive, so
//! paths are normalized to forward slashes with an upper-case drive letter,
//! the form Xray compares against.

use crate::{blocking, config, err, field_string, network_insights::notice, persist, run_powershell_json, AppResult, AppState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::Ipv4Addr;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

pub const MAX_APP_RULES: usize = 50;
pub const MAX_PROGRAMS: usize = 32;
const TARGETS: [&str; 4] = ["proxy", "direct", "block", "local"];
const FORBIDDEN: [char; 6] = ['*', '?', '"', '<', '>', '|'];
/// Processes whose own port must not be used as the local proxy (it would loop).
const LOOP_PROCESSES: [&str; 2] = ["xray", "v2rayn"];

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalProxy {
    pub protocol: String,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppRule {
    pub id: String,
    pub name: String,
    pub programs: Vec<String>,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_proxy: Option<LocalProxy>,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
}
fn enabled_default() -> bool {
    true
}

/// Normalizes one program entry: `chrome.exe`, `C:/Apps/Tool/tool.exe` or a
/// folder `C:/Apps/Tool/` (matches every program inside it).
pub fn normalize_program(entry: &str) -> Result<String, String> {
    let entry = entry.trim();
    if entry.is_empty() || entry.chars().count() > 260 || entry.chars().any(|c| c.is_control() || FORBIDDEN.contains(&c)) {
        return Err(format!("\"{entry}\" is not a valid program name, path or folder."));
    }
    if !entry.contains(['/', '\\', ':']) {
        if !entry.to_ascii_lowercase().ends_with(".exe") || entry.len() <= 4 {
            return Err(format!("\"{entry}\" must be a program file name such as chrome.exe."));
        }
        return Ok(entry.to_string());
    }
    let mut path = entry.replace('\\', "/");
    let bytes = path.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'/' {
        return Err(format!("\"{entry}\" must be a full path such as C:\\Apps\\tool.exe."));
    }
    path.replace_range(0..1, &path[0..1].to_ascii_uppercase());
    if path[2..].contains(':') || path.split('/').any(|part| part == ".." || part == ".") || path.contains("//") {
        return Err(format!("\"{entry}\" is not a valid path."));
    }
    let folder = path.ends_with('/');
    if !folder && !path.to_ascii_lowercase().ends_with(".exe") {
        return Err(format!("\"{entry}\" must end with .exe, or with \\ to match a whole folder."));
    }
    if folder && path.len() <= 3 {
        return Err("A whole drive cannot be used as a folder rule.".into());
    }
    Ok(path)
}

fn valid_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host.parse::<Ipv4Addr>().is_ok()
}

/// Validates and normalizes a rule. `others` are the other saved rules.
pub fn validate_rule(rule: &mut AppRule, others: &[AppRule]) -> Result<(), String> {
    rule.name = rule.name.trim().to_string();
    if rule.name.is_empty() || rule.name.chars().count() > 64 || rule.name.chars().any(char::is_control) {
        return Err("Give the rule a name of up to 64 characters.".into());
    }
    if rule.id.is_empty() || rule.id.len() > 64 || !rule.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("The rule ID is invalid.".into());
    }
    if !TARGETS.contains(&rule.target.as_str()) {
        return Err("Choose proxy, direct, block or a local proxy.".into());
    }
    if rule.programs.is_empty() || rule.programs.len() > MAX_PROGRAMS {
        return Err(format!("Add between 1 and {MAX_PROGRAMS} programs."));
    }
    let mut programs: Vec<String> = vec![];
    for entry in &rule.programs {
        let program = normalize_program(entry)?;
        if !programs.contains(&program) {
            programs.push(program);
        }
    }
    rule.programs = programs;
    if rule.target == "local" {
        let proxy = rule.local_proxy.as_mut().ok_or("Enter the local proxy address and port.")?;
        proxy.host = proxy.host.trim().to_string();
        if proxy.protocol != "socks" && proxy.protocol != "http" {
            return Err("The local proxy must be SOCKS or HTTP.".into());
        }
        if !valid_host(&proxy.host) {
            return Err("The local proxy host must be localhost or an IPv4 address.".into());
        }
        if proxy.port == 0 {
            return Err("The local proxy port must be between 1 and 65535.".into());
        }
    } else {
        rule.local_proxy = None;
    }
    if others.iter().any(|other| other.id != rule.id && other.name.eq_ignore_ascii_case(&rule.name)) {
        return Err("Another rule already has this name.".into());
    }
    if !others.iter().any(|other| other.id == rule.id) && others.len() >= MAX_APP_RULES {
        return Err(format!("At most {MAX_APP_RULES} per-app rules can be saved."));
    }
    Ok(())
}

/// The v2rayN server name (remark) used for a local proxy; routing rules point
/// at it by this name.
pub fn server_remark(proxy: &LocalProxy) -> String {
    format!("WNM-{}-{}-{}", proxy.protocol, proxy.host.to_ascii_lowercase(), proxy.port)
}

/// Text for v2rayN: the routing rules (JSON array for "Import Rules From
/// Clipboard") and the local proxy servers the rules point at. With
/// `others_direct`, a last catch-all rule sends every other program direct,
/// so only the listed programs use a proxy.
pub fn v2rayn_export(rules: &[AppRule], others_direct: bool) -> Value {
    let enabled = rules.iter().filter(|rule| rule.enabled).collect::<Vec<_>>();
    let mut items = enabled
        .iter()
        .map(|rule| {
            let outbound = match (&rule.target[..], &rule.local_proxy) {
                ("local", Some(proxy)) => server_remark(proxy),
                (target, _) => target.to_string(),
            };
            json!({"remarks":format!("WNM: {}", rule.name),"outboundTag":outbound,"process":rule.programs,"enabled":true})
        })
        .collect::<Vec<_>>();
    if others_direct {
        items.push(json!({"remarks":"WNM: everything else","outboundTag":"direct","port":"0-65535","enabled":true}));
    }
    let mut servers: Vec<Value> = vec![];
    for proxy in enabled.iter().filter(|rule| rule.target == "local").filter_map(|rule| rule.local_proxy.as_ref()) {
        let remark = server_remark(proxy);
        if servers.iter().any(|server| server["remark"] == remark) {
            continue;
        }
        let link = (proxy.protocol == "socks").then(|| format!("socks://{}:{}#{}", proxy.host, proxy.port, remark));
        servers.push(json!({"remark":remark,"protocol":proxy.protocol,"host":proxy.host,"port":proxy.port,"link":link}));
    }
    let folders = enabled.iter().flat_map(|rule| rule.programs.iter()).any(|program| program.ends_with('/'));
    json!({
        "rulesJson":serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into()),
        "ruleCount":items.len(),
        "servers":servers,
        "usesFolders":folders,
    })
}

fn saved_rules(cfg: &crate::AppConfig) -> Vec<AppRule> {
    cfg.app_rules.iter().filter_map(|item| serde_json::from_value(item.clone()).ok()).collect()
}

#[tauri::command]
pub fn get_app_rules(state: State<AppState>, others_direct: Option<bool>) -> AppResult<Value> {
    let rules = saved_rules(&config(&state));
    Ok(json!({"rules":rules,"export":v2rayn_export(&rules, others_direct.unwrap_or(false))}))
}

// Configuration-only write: stores a rule in the app's own settings file.
#[tauri::command]
pub fn save_app_rule(state: State<AppState>, rule: Value) -> AppResult<Value> {
    let mut rule: AppRule = serde_json::from_value(rule).map_err(|_| err("VALIDATION_ERROR", "The per-app rule is invalid."))?;
    let mut cfg = config(&state);
    let mut rules = saved_rules(&cfg);
    validate_rule(&mut rule, &rules).map_err(|message| err("VALIDATION_ERROR", message))?;
    match rules.iter_mut().find(|item| item.id == rule.id) {
        Some(existing) => *existing = rule.clone(),
        None => rules.push(rule.clone()),
    }
    cfg.app_rules = rules.iter().filter_map(|item| serde_json::to_value(item).ok()).collect();
    persist(&state, &cfg)?;
    let message = notice("appRule.saved", json!({"name":rule.name}), format!("Per-app rule \"{}\" saved.", rule.name));
    Ok(json!({"rules":rules,"message":message["text"],"messageCode":message}))
}

// Configuration-only write: removes a stored rule.
#[tauri::command]
pub fn remove_app_rule(state: State<AppState>, id: String) -> AppResult<Value> {
    let mut cfg = config(&state);
    let before = cfg.app_rules.len();
    cfg.app_rules.retain(|item| field_string(item, "id") != id);
    if cfg.app_rules.len() == before {
        return Err(err("NOT_FOUND", "The per-app rule no longer exists."));
    }
    persist(&state, &cfg)?;
    let rules = saved_rules(&cfg);
    let message = notice("appRule.removed", json!({}), "Per-app rule removed.");
    Ok(json!({"rules":rules,"message":message["text"],"messageCode":message}))
}

// Read-only: the native open dialog returns the chosen program's path.
#[tauri::command]
pub async fn pick_program(app: AppHandle) -> AppResult<Value> {
    blocking(move || {
        let mut dialog = app.dialog().file().add_filter("Programs", &["exe"]);
        if let Some(window) = app.get_webview_window("main") {
            dialog = dialog.set_parent(&window);
        }
        let Some(path) = dialog.blocking_pick_file() else {
            return Ok(json!({"cancelled":true}));
        };
        let path = path.into_path().map_err(|_| err("VALIDATION_ERROR", "Choose a program on this PC."))?;
        let program = normalize_program(&path.display().to_string()).map_err(|message| err("VALIDATION_ERROR", message))?;
        Ok(json!({"path":program}))
    })
    .await
}

const RUNNING_PROGRAMS_SCRIPT: &str = r#"$list=@(Get-Process -ErrorAction SilentlyContinue|Where-Object{$_.Path}|Group-Object Path|Sort-Object Name|Select-Object -First 400|ForEach-Object{[PSCustomObject]@{path=[string]$_.Name;name=[IO.Path]::GetFileName([string]$_.Name);count=[int]$_.Count}});[PSCustomObject]@{programs=$list}|ConvertTo-Json -Compress -Depth 4"#;

// Read-only: running programs with a known path (other users' elevated
// programs are not visible unless this app runs as administrator).
#[tauri::command]
pub async fn list_running_programs() -> AppResult<Value> {
    blocking(|| {
        let result = run_powershell_json(RUNNING_PROGRAMS_SCRIPT, None)?;
        let programs = crate::arrays(result.get("programs"))
            .into_iter()
            .filter_map(|item| {
                let path = normalize_program(item.get("path")?.as_str()?).ok()?;
                Some(json!({"path":path,"name":item["name"],"count":item["count"]}))
            })
            .collect::<Vec<_>>();
        Ok(json!({"programs":programs}))
    })
    .await
}

const PORT_OWNER_SCRIPT: &str = r#"$c=Get-NetTCPConnection -State Listen -LocalPort ([int]$InputData.port) -ErrorAction SilentlyContinue|Select-Object -First 1;if(-not $c){[PSCustomObject]@{listening=$false}|ConvertTo-Json -Compress}else{$p=Get-Process -Id $c.OwningProcess -ErrorAction SilentlyContinue;[PSCustomObject]@{listening=$true;process=[string]$p.ProcessName;pid=[int]$c.OwningProcess}|ConvertTo-Json -Compress}"#;

/// True when the port belongs to v2rayN's own core: pointing v2rayN at it loops.
pub fn is_loop_process(name: &str) -> bool {
    LOOP_PROCESSES.contains(&name.trim().to_ascii_lowercase().trim_end_matches(".exe"))
}

// Read-only: which program listens on the local proxy port.
#[tauri::command]
pub async fn check_local_proxy(port: u16) -> AppResult<Value> {
    if port == 0 {
        return Err(err("VALIDATION_ERROR", "The local proxy port must be between 1 and 65535."));
    }
    blocking(move || {
        let result = run_powershell_json(PORT_OWNER_SCRIPT, Some(&json!({"port":port})))?;
        let listening = result.get("listening").and_then(Value::as_bool).unwrap_or(false);
        let process = result.get("process").and_then(Value::as_str).unwrap_or("").to_string();
        let looped = listening && is_loop_process(&process);
        let message = if !listening {
            notice("appRule.portFree", json!({"port":port}), format!("Nothing is listening on port {port}. Start the proxy program first."))
        } else if looped {
            notice("appRule.portLoop", json!({"port":port,"process":process}), format!("Port {port} belongs to {process} (v2rayN itself). Sending v2rayN traffic back to it would loop; use the proxy program's own port."))
        } else {
            notice("appRule.portOwner", json!({"port":port,"process":process}), format!("Port {port} is served by {process}."))
        };
        Ok(json!({"listening":listening,"process":process,"loop":looped,"message":message["text"],"messageCode":message}))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> String {
        Ipv4Addr::new(a, b, c, d).to_string()
    }

    fn rule(target: &str, programs: &[&str]) -> AppRule {
        AppRule {
            id: "r1".into(),
            name: "Chat".into(),
            programs: programs.iter().map(|item| item.to_string()).collect(),
            target: target.into(),
            local_proxy: None,
            enabled: true,
        }
    }

    #[test]
    fn programs_are_names_paths_or_folders() {
        assert_eq!(normalize_program(" chrome.exe ").unwrap(), "chrome.exe");
        assert_eq!(normalize_program(r"c:\Apps\Tool\tool.exe").unwrap(), "C:/Apps/Tool/tool.exe");
        assert_eq!(normalize_program(r"D:\Apps\Tool\").unwrap(), "D:/Apps/Tool/");
        for bad in ["", "chrome", "*.exe", r"Apps\tool.exe", r"C:\Apps\..\tool.exe", r"C:\Apps\readme.txt", r"C:\", "C:/a//b.exe", r"\\server\share\a.exe"] {
            assert!(normalize_program(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rules_are_validated_and_deduplicated() {
        let mut item = rule("proxy", &["chrome.exe", "chrome.exe", r"C:\Apps\"]);
        validate_rule(&mut item, &[]).unwrap();
        assert_eq!(item.programs, vec!["chrome.exe", "C:/Apps/"]);
        let mut bad = rule("tunnel", &["chrome.exe"]);
        assert!(validate_rule(&mut bad, &[]).is_err());
        let mut empty = rule("proxy", &[]);
        assert!(validate_rule(&mut empty, &[]).is_err());
        let mut local = rule("local", &["git.exe"]);
        assert!(validate_rule(&mut local, &[]).is_err());
        local.local_proxy = Some(LocalProxy { protocol: "socks".into(), host: "example.com".into(), port: 8889 });
        assert!(validate_rule(&mut local, &[]).is_err());
        local.local_proxy = Some(LocalProxy { protocol: "socks".into(), host: ip(127, 0, 0, 1), port: 8889 });
        validate_rule(&mut local, &[]).unwrap();
        let mut direct = rule("direct", &["git.exe"]);
        direct.local_proxy = local.local_proxy.clone();
        validate_rule(&mut direct, &[]).unwrap();
        assert_eq!(direct.local_proxy, None);
        let mut duplicate = rule("block", &["a.exe"]);
        duplicate.id = "r2".into();
        assert!(validate_rule(&mut duplicate, &[item]).is_err());
    }

    #[test]
    fn export_points_rules_at_local_proxy_servers() {
        let mut local = rule("local", &["git.exe", "C:/Tools/"]);
        local.local_proxy = Some(LocalProxy { protocol: "socks".into(), host: "localhost".into(), port: 8889 });
        let mut http = rule("local", &["npm.exe"]);
        http.name = "Node".into();
        http.local_proxy = Some(LocalProxy { protocol: "http".into(), host: ip(127, 0, 0, 1), port: 8080 });
        let mut off = rule("block", &["game.exe"]);
        off.enabled = false;
        let export = v2rayn_export(&[local, http, off, rule("direct", &["Teams.exe"])], false);
        let rules: Value = serde_json::from_str(export["rulesJson"].as_str().unwrap()).unwrap();
        assert_eq!(export["ruleCount"], 3);
        assert_eq!(rules[0]["outboundTag"], "WNM-socks-localhost-8889");
        assert_eq!(rules[0]["process"], json!(["git.exe", "C:/Tools/"]));
        assert_eq!(rules[2]["outboundTag"], "direct");
        assert_eq!(rules[2]["remarks"], "WNM: Chat");
        assert_eq!(export["servers"][0]["link"], "socks://localhost:8889#WNM-socks-localhost-8889");
        assert_eq!(export["servers"][1]["link"], Value::Null);
        assert_eq!(export["usesFolders"], true);
        let only = v2rayn_export(&[rule("proxy", &["chrome.exe"])], true);
        let rules: Value = serde_json::from_str(only["rulesJson"].as_str().unwrap()).unwrap();
        assert_eq!(rules[1], json!({"remarks":"WNM: everything else","outboundTag":"direct","port":"0-65535","enabled":true}));
    }

    // Read-only PowerShell checks; run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn read_only_running_programs_and_port_owner() {
        let result = run_powershell_json(RUNNING_PROGRAMS_SCRIPT, None).unwrap();
        let programs = crate::arrays(result.get("programs"));
        let valid = programs.iter().filter(|item| normalize_program(item["path"].as_str().unwrap_or("")).is_ok()).count();
        println!("running programs: {} ({} valid)", programs.len(), valid);
        assert!(valid > 0);
        for port in [10808u16, 8889] {
            let owner = run_powershell_json(PORT_OWNER_SCRIPT, Some(&json!({"port":port}))).unwrap();
            println!("port {port}: {owner}");
        }
    }

    #[test]
    fn v2rayn_own_port_is_a_loop() {
        assert!(is_loop_process("xray"));
        assert!(is_loop_process("v2rayN.exe"));
        assert!(!is_loop_process("clash-verge"));
    }
}
