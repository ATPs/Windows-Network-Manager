//! App automation: the opt-in "Start with Windows" entry and configuration
//! import/export. Startup uses the current user's Run key only (no scheduled
//! task, no elevation). Import/export opens native file dialogs from Rust and
//! touches only the file the user picks; the frontend gets no file access.
//! Exports never contain passwords, recovery points or app-managed route
//! records, and imported auto-connect rules start disabled.

use crate::{
    blocking, config, consume_confirmation, err, err_with, fingerprint, issue_confirmation, mutate_auto_rules,
    network_insights::notice, normalized_wifi_guid, persist, valid_theme, wifi_auto_connect, AppResult, AppState,
    Settings,
};
use serde_json::{json, Map, Value};
use std::{fs, path::Path, ptr};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

pub const MINIMIZED_ARG: &str = "--minimized";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Windows Network Manager";
pub const EXPORT_FORMAT: &str = "windows-network-manager-config";
const MAX_IMPORT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PROFILES: usize = 200;

// ---------------------------------------------------------------------------
// Start with Windows
// ---------------------------------------------------------------------------

pub fn startup_command(exe: &Path) -> String {
    format!("\"{}\" {MINIMIZED_ARG}", exe.display())
}

pub fn started_minimized(args: &[String]) -> bool {
    args.iter().skip(1).any(|arg| arg == MINIMIZED_ARG)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn read_run_value() -> Option<String> {
    let (key, name) = (wide(RUN_KEY), wide(RUN_VALUE));
    let mut buffer = vec![0u16; 2048];
    let mut size = (buffer.len() * 2) as u32;
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if code != 0 {
        return None;
    }
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

fn startup_status() -> Value {
    let expected = std::env::current_exe().ok().map(|exe| startup_command(&exe));
    let current = read_run_value();
    json!({
        "enabled":current.is_some(),
        "matchesCurrent":current.is_some() && current == expected,
        "available":!cfg!(debug_assertions),
    })
}

#[tauri::command]
pub fn get_startup_status() -> Value {
    startup_status()
}

// Writes only the current user's Run value; never touches the network.
#[tauri::command]
pub fn set_start_with_windows(enabled: bool) -> AppResult<Value> {
    if cfg!(debug_assertions) {
        return Err(err("NOT_AVAILABLE", "Start with Windows is unavailable in development builds."));
    }
    let (key, name) = (wide(RUN_KEY), wide(RUN_VALUE));
    let code = if enabled {
        let exe = std::env::current_exe().map_err(|_| err("COMMAND_FAILED", "Cannot locate the application executable."))?;
        let data = wide(&startup_command(&exe));
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        }
    } else {
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) } {
            2 => 0, // ERROR_FILE_NOT_FOUND: already off
            code => code,
        }
    };
    if code != 0 {
        return Err(err_with("COMMAND_FAILED", "Windows could not update the startup setting.", json!({"code":code})));
    }
    let message = if enabled {
        notice("startup.enabled", json!({}), "The app starts in the tray when you sign in to Windows.")
    } else {
        notice("startup.disabled", json!({}), "The app no longer starts with Windows.")
    };
    let mut status = startup_status();
    status["message"] = message["text"].clone();
    status["messageCode"] = message;
    Ok(status)
}

// ---------------------------------------------------------------------------
// Import / export
// ---------------------------------------------------------------------------

/// The exported document: settings, saved configurations and auto-connect
/// rules. History snapshots, recovery points and route ownership records are
/// specific to this PC and are not exported.
pub fn export_document(cfg: &crate::AppConfig, app_version: &str, now: &str) -> Value {
    json!({
        "format":EXPORT_FORMAT,
        "formatVersion":1,
        "appVersion":app_version,
        "exportedAt":now,
        "settings":cfg.settings,
        "profiles":cfg.profiles,
        "wifiAutoConnect":cfg.wifi_auto_connect,
    })
}

fn bounded_string(value: Option<&Value>, max: usize) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty() && text.chars().count() <= max && !text.chars().any(char::is_control))
        .map(str::to_owned)
}

fn validate_profile(value: &Value) -> Result<Value, String> {
    let item = value.as_object().ok_or("A saved configuration is not an object.")?;
    for key in item.keys() {
        if !["id", "name", "createdAt", "updatedAt", "state"].contains(&key.as_str()) {
            return Err(format!("Unknown saved-configuration field \"{key}\"."));
        }
    }
    let id = bounded_string(item.get("id"), 64).ok_or("A saved configuration has an invalid id.")?;
    let name = bounded_string(item.get("name"), 100).ok_or("A saved configuration has an invalid name.")?;
    let mut clean = Map::new();
    clean.insert("id".into(), json!(id));
    clean.insert("name".into(), json!(name));
    for key in ["createdAt", "updatedAt"] {
        if let Some(text) = bounded_string(item.get(key), 64) {
            clean.insert(key.into(), json!(text));
        }
    }
    if let Some(state) = item.get("state") {
        let valid = state.is_object()
            && state.get("overview").map(Value::is_object).unwrap_or(false)
            && state.pointer("/overview/interfaces").map(Value::is_array).unwrap_or(false)
            && state.get("routes").map(|routes| routes.is_object() || routes.is_null()).unwrap_or(true);
        if !valid {
            return Err(format!("Saved configuration \"{name}\" has an invalid network state."));
        }
        clean.insert("state".into(), state.clone());
    }
    Ok(Value::Object(clean))
}

/// Strictly validate an imported document and return the normalized import:
/// `{settings, profiles, wifiAutoConnect}` with every rule disabled.
pub fn validate_import(document: &Value) -> Result<Value, String> {
    let root = document.as_object().ok_or("The file is not a configuration export.")?;
    if root.get("format").and_then(Value::as_str) != Some(EXPORT_FORMAT) {
        return Err("The file is not a Windows Network Manager configuration export.".into());
    }
    if root.get("formatVersion").and_then(Value::as_u64) != Some(1) {
        return Err("This export was made by an unsupported version.".into());
    }
    for key in root.keys() {
        if !["format", "formatVersion", "appVersion", "exportedAt", "settings", "profiles", "wifiAutoConnect"]
            .contains(&key.as_str())
        {
            return Err(format!("Unknown field \"{key}\" in the export."));
        }
    }
    let settings = match root.get("settings").filter(|value| !value.is_null()) {
        Some(value) => {
            let settings: Settings =
                serde_json::from_value(value.clone()).map_err(|_| "The exported settings are invalid.".to_string())?;
            if settings.language != "en" && settings.language != "zh-CN" {
                return Err("The exported language is not supported.".into());
            }
            valid_theme(&settings.theme).map_err(|e| e.message)?;
            Some(settings)
        }
        None => None,
    };
    let profiles = match root.get("profiles") {
        Some(Value::Array(items)) if items.len() <= MAX_PROFILES => {
            items.iter().map(validate_profile).collect::<Result<Vec<_>, _>>()?
        }
        Some(Value::Array(_)) => return Err(format!("An export can contain at most {MAX_PROFILES} saved configurations.")),
        Some(_) => return Err("Saved configurations must be a list.".into()),
        None => vec![],
    };
    let mut ids = profiles.iter().map(|item| item["id"].as_str().unwrap_or_default()).collect::<Vec<_>>();
    ids.sort();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("The export contains duplicate saved configurations.".into());
    }
    let mut rules = vec![];
    let mut guids = vec![];
    for value in root.get("wifiAutoConnect").and_then(Value::as_array).cloned().unwrap_or_default() {
        let mut rule: wifi_auto_connect::AutoConnectRule =
            serde_json::from_value(value).map_err(|_| "An auto-connect rule is invalid.".to_string())?;
        wifi_auto_connect::validate_rule(&rule)?;
        rule.adapter_guid = normalized_wifi_guid(&rule.adapter_guid).map_err(|e| e.message)?;
        rule.enabled = false;
        if guids.contains(&rule.adapter_guid) {
            return Err("The export contains duplicate auto-connect rules.".into());
        }
        guids.push(rule.adapter_guid.clone());
        rules.push(rule);
    }
    if rules.len() > 64 {
        return Err("The export contains too many auto-connect rules.".into());
    }
    Ok(json!({"settings":settings,"profiles":profiles,"wifiAutoConnect":rules}))
}

/// Preview rows: how the import changes the current configuration.
pub fn import_summary(import: &Value, cfg: &crate::AppConfig) -> Value {
    let existing = cfg.profiles.iter().filter_map(|item| item.get("id").and_then(Value::as_str)).collect::<Vec<_>>();
    let profiles = import["profiles"].as_array().cloned().unwrap_or_default();
    let replaced = profiles.iter().filter(|item| existing.contains(&item["id"].as_str().unwrap_or_default())).count();
    json!({
        "settings":!import["settings"].is_null(),
        "profilesAdded":profiles.len() - replaced,
        "profilesReplaced":replaced,
        "rules":import["wifiAutoConnect"].as_array().map(Vec::len).unwrap_or(0),
    })
}

fn main_window_dialog(app: &AppHandle) -> tauri_plugin_dialog::FileDialogBuilder<tauri::Wry> {
    let dialog = app.dialog().file().add_filter("JSON", &["json"]);
    match app.get_webview_window("main") {
        Some(window) => dialog.set_parent(&window),
        None => dialog,
    }
}

// Writes only the file chosen in the save dialog.
#[tauri::command]
pub async fn export_config(app: AppHandle) -> AppResult<Value> {
    blocking(move || {
        let cfg = config(&app.state::<AppState>());
        let now = chrono::Utc::now();
        let document = export_document(&cfg, &app.package_info().version.to_string(), &now.to_rfc3339());
        let file_name = format!("windows-network-manager-{}.json", now.format("%Y%m%d"));
        let Some(path) = main_window_dialog(&app).set_file_name(file_name).blocking_save_file() else {
            return Ok(json!({"cancelled":true}));
        };
        let path = path.into_path().map_err(|_| err("VALIDATION_ERROR", "Choose a file on this PC."))?;
        let text = serde_json::to_string_pretty(&document).map_err(|_| err("SERIALIZE_ERROR", "Cannot export the configuration."))?;
        fs::write(&path, format!("{text}\n"))
            .map_err(|e| err_with("EXPORT_FAILED", "Cannot write the export file.", json!({"reason":e.to_string()})))?;
        let message = notice("config.exported", json!({"path":path.display().to_string()}), format!("Configuration exported to {}.", path.display()));
        Ok(json!({"exported":true,"path":path.display().to_string(),"message":message["text"],"messageCode":message}))
    })
    .await
}

// Reads only the file chosen in the open dialog; nothing is saved until apply.
#[tauri::command]
pub async fn import_config_preview(app: AppHandle) -> AppResult<Value> {
    blocking(move || {
        let Some(path) = main_window_dialog(&app).blocking_pick_file() else {
            return Ok(json!({"cancelled":true}));
        };
        let path = path.into_path().map_err(|_| err("VALIDATION_ERROR", "Choose a file on this PC."))?;
        let size = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(u64::MAX);
        if size > MAX_IMPORT_BYTES {
            return Err(err("VALIDATION_ERROR", "The file is too large to be a configuration export."));
        }
        let text = fs::read_to_string(&path).map_err(|_| err("VALIDATION_ERROR", "The file cannot be read as text."))?;
        let document: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|_| err("VALIDATION_ERROR", "The file is not valid JSON."))?;
        let import = validate_import(&document).map_err(|message| err("VALIDATION_ERROR", message))?;
        let state = app.state::<AppState>();
        let summary = import_summary(&import, &config(&state));
        Ok(json!({
            "import":import,
            "summary":summary,
            "fileName":path.file_name().map(|name| name.to_string_lossy().to_string()),
            "exportedAt":document.get("exportedAt"),
            "confirmationToken":issue_confirmation(&state, "import-config", &import)?
        }))
    })
    .await
}

// Configuration-only write: settings, saved configurations and disabled rules.
#[tauri::command]
pub async fn import_config_apply(app: AppHandle, input: Value) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        let submitted = input.get("import").cloned().unwrap_or(Value::Null);
        // Re-validate the submitted import; the token proves it was previewed.
        let import = validate_import(&json!({
            "format":EXPORT_FORMAT,
            "formatVersion":1,
            "settings":submitted.get("settings").filter(|value| !value.is_null()),
            "profiles":submitted.get("profiles"),
            "wifiAutoConnect":submitted.get("wifiAutoConnect"),
        }))
        .map_err(|message| err("VALIDATION_ERROR", message))?;
        if fingerprint(&import) != fingerprint(&submitted) {
            return Err(err("CONFIRMATION_EXPIRED", "Confirmation expired. Preview the import again."));
        }
        let token = input.get("confirmationToken").and_then(Value::as_str).unwrap_or("");
        consume_confirmation(&state, token, "import-config", &import)?;
        let mut cfg = config(&state);
        if let Ok(settings) = serde_json::from_value::<Settings>(import["settings"].clone()) {
            cfg.settings = settings;
        }
        for profile in import["profiles"].as_array().cloned().unwrap_or_default() {
            let id = profile["id"].as_str().unwrap_or_default().to_string();
            cfg.profiles.retain(|item| item.get("id").and_then(Value::as_str) != Some(id.as_str()));
            cfg.profiles.push(profile);
        }
        persist(&state, &cfg)?;
        let rules: Vec<wifi_auto_connect::AutoConnectRule> =
            serde_json::from_value(import["wifiAutoConnect"].clone()).unwrap_or_default();
        if !rules.is_empty() {
            mutate_auto_rules(&state, |current| {
                current.retain(|old| {
                    let key = normalized_wifi_guid(&old.adapter_guid).ok();
                    !rules.iter().any(|rule| Some(&rule.adapter_guid) == key.as_ref())
                });
                current.extend(rules.iter().cloned());
                Ok(())
            })?;
        }
        let message = notice("config.imported", json!({}), "Configuration imported. Imported auto-connect rules are disabled until you enable them.");
        Ok(json!({"imported":true,"settings":cfg.settings,"message":message["text"],"messageCode":message}))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> Value {
        json!({
            "format":EXPORT_FORMAT,
            "formatVersion":1,
            "appVersion":"2.5.0",
            "exportedAt":"2026-11-02T00:00:00Z",
            "settings":{"language":"zh-CN","theme":"dark"},
            "profiles":[{"id":"a","name":"Office","createdAt":"x","state":{"overview":{"interfaces":[]},"routes":{"routes":[]}}}],
            "wifiAutoConnect":[{"adapterGuid":"{255BC7A7-A80C-47E6-902B-386AA6EE90DB}","profileName":"Home","intervalSeconds":60,"enabled":true}]
        })
    }

    #[test]
    fn startup_command_launches_minimized() {
        let command = startup_command(Path::new(r"C:\Apps\Windows Network Manager.exe"));
        assert_eq!(command, "\"C:\\Apps\\Windows Network Manager.exe\" --minimized");
        assert!(started_minimized(&["app.exe".into(), "--minimized".into()]));
        assert!(!started_minimized(&["--minimized".into()]), "argv[0] is the program");
        assert!(!started_minimized(&["app.exe".into(), "--wnm-wait-parent=4".into()]));
    }

    #[test]
    fn exports_omit_machine_specific_records() {
        let mut cfg = crate::AppConfig::default();
        cfg.managed_routes.push(json!({"destinationPrefix":format!("{}/8", std::net::Ipv4Addr::new(10, 0, 0, 0))}));
        cfg.profiles.push(json!({"id":"a","name":"Office"}));
        let document = export_document(&cfg, "2.5.0", "now");
        assert!(document.get("managedRoutes").is_none());
        assert!(document.get("baseline").is_none());
        assert!(document.get("recentSnapshots").is_none());
        assert_eq!(validate_import(&document).unwrap()["profiles"][0]["name"], "Office");
    }

    #[test]
    fn imports_are_strictly_validated_and_rules_start_disabled() {
        let import = validate_import(&document()).unwrap();
        assert_eq!(import["settings"]["language"], "zh-CN");
        assert_eq!(import["wifiAutoConnect"][0]["enabled"], false);
        assert_eq!(import["wifiAutoConnect"][0]["adapterGuid"], "255bc7a7-a80c-47e6-902b-386aa6ee90db");
        let broken = |change: fn(&mut Value)| {
            let mut value = document();
            change(&mut value);
            validate_import(&value).is_err()
        };
        assert!(broken(|v| v["format"] = json!("other")));
        assert!(broken(|v| v["formatVersion"] = json!(2)));
        assert!(broken(|v| v["managedRoutes"] = json!([])));
        assert!(broken(|v| v["settings"]["language"] = json!("fr")));
        assert!(broken(|v| v["settings"]["theme"] = json!("neon")));
        assert!(broken(|v| v["profiles"][0]["name"] = json!("")));
        assert!(broken(|v| v["profiles"][0]["password"] = json!("secret")));
        assert!(broken(|v| v["profiles"][0]["state"] = json!({"overview":"x"})));
        assert!(broken(|v| v["profiles"] = json!([{"id":"a","name":"A"},{"id":"a","name":"B"}])));
        assert!(broken(|v| v["wifiAutoConnect"][0]["profileName"] = json!("bad\nname")));
        assert!(broken(|v| v["wifiAutoConnect"][0]["intervalSeconds"] = json!(1)));
        let mut cfg = crate::AppConfig::default();
        cfg.profiles.push(json!({"id":"a","name":"Old"}));
        let summary = import_summary(&import, &cfg);
        assert_eq!(summary["profilesReplaced"], 1);
        assert_eq!(summary["profilesAdded"], 0);
        assert_eq!(summary["rules"], 1);
    }
}
