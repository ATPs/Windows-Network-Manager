//! Network backups: per-adapter IPv4/DHCP and DNS settings, interface Metric,
//! network category, app-managed routes and the hosts file. Backups are made
//! on request and automatically before every risky app change, and are
//! stored next to the configuration (`network-backups.json` plus hosts copies
//! in `network-backups\`). Nothing here stores passwords or Wi-Fi keys.
//!
//! A restore is planned from live state (pure, unit tested), previewed with
//! Rust-computed warnings, needs an acknowledgement, and is reverted after
//! `REVERT_SECONDS` unless kept. Coexistence: IP/DNS of adapters owned by
//! tunnel/proxy software are never restored; Metric and category may change
//! there, with a warning. Capture routes of TUN software are never touched.

use crate::{
    advanced, arrays, blocking, coexistence, config, config_edit, consume_confirmation, current_snapshot, err,
    err_with, field_bool, field_i64, field_string, fingerprint, issue_confirmation, live_snapshot,
    network_insights::{normalized_guid_key, notice, notice_texts},
    persist, require_admin, route_key, run_powershell_json, AppResult, AppState,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
};
use tauri::{AppHandle, Manager, State};

pub const MAX_MANUAL: usize = 50;
pub const MAX_AUTOMATIC: usize = 20;
const STORE_FILE: &str = "network-backups.json";
const HOSTS_DIR: &str = "network-backups";

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub id: String,
    pub name: String,
    /// "manual" or "automatic".
    pub kind: String,
    pub created_at: String,
    #[serde(default)]
    pub app_version: String,
    /// Automatic backups: the change that triggered them, as a notice.
    #[serde(default)]
    pub reason: Option<Value>,
    #[serde(default)]
    pub adapters: Vec<Value>,
    /// App-managed routes present when the backup was made (full records).
    #[serde(default)]
    pub routes: Vec<Value>,
    /// `{hash, entries}` or `{unavailable}`; the text lives in `network-backups\`.
    #[serde(default)]
    pub hosts: Option<Value>,
    /// Content fingerprint, used to skip identical automatic backups.
    #[serde(default)]
    pub hash: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Store {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    backups: Vec<Backup>,
}

// ---------------------------------------------------------------------------
// Pure helpers (unit tested)
// ---------------------------------------------------------------------------

/// Names are optional; when given they are trimmed, 1-80 characters and free
/// of control characters.
pub fn valid_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err("Backup names can have at most 80 characters and no control characters.".into());
    }
    Ok(name.to_string())
}

/// Hosts copies are addressed only by their SHA-256 fingerprint.
pub fn valid_hosts_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Why an app change takes an automatic backup first, or None when it does
/// not (Wi-Fi connections, adapter power and saved Wi-Fi profiles are not
/// part of a backup).
pub fn auto_reason(scope: &Value) -> Option<Value> {
    let alias = |key: &str| scope.get(key).map(|item| field_string(item, "alias")).unwrap_or_default();
    if scope.get("ipSettings").is_some() {
        let alias = alias("ipSettings");
        return Some(notice("backup.reason.ipSettings", json!({"alias":alias}), format!("Before changing IP/DNS on {alias}")));
    }
    if scope.get("networkCategory").is_some() {
        let alias = alias("networkCategory");
        return Some(notice("backup.reason.category", json!({"alias":alias}), format!("Before changing the network category of {alias}")));
    }
    if scope.get("metrics").is_some() {
        return Some(notice("backup.reason.metric", json!({}), "Before changing interface Metric"));
    }
    if ["addRoute", "editRoute", "removeRoute", "domainRoute"].iter().any(|key| scope.get(*key).is_some()) {
        return Some(notice("backup.reason.route", json!({}), "Before changing routes"));
    }
    if scope.get("hosts").is_some() {
        return Some(notice("backup.reason.hosts", json!({}), "Before editing the hosts file"));
    }
    if scope.get("restoreProfile").is_some() || scope.get("restoreRecent").is_some() {
        return Some(if field_bool(scope, "trigger") {
            notice("backup.reason.trigger", json!({}), "Before a network trigger applied a saved configuration")
        } else {
            notice("backup.reason.restore", json!({}), "Before restoring a saved configuration")
        });
    }
    if scope.get("windowsDefaults").is_some() {
        return Some(notice("backup.reason.defaults", json!({}), "Before restoring Windows defaults"));
    }
    if let Some(item) = scope.get("backupRestore") {
        let name = field_string(item, "name");
        return Some(notice("backup.reason.backupRestore", json!({"name":name}), format!("Before restoring backup \"{name}\"")));
    }
    None
}

/// Insert a backup newest-first. An automatic backup identical to the newest
/// automatic one only refreshes it; automatic backups rotate at
/// `MAX_AUTOMATIC` and never remove manual ones. Returns the stored backup.
fn insert(backups: &mut Vec<Backup>, backup: Backup) -> AppResult<Backup> {
    if backup.kind == "automatic" {
        if let Some(index) = backups.iter().position(|item| item.kind == "automatic") {
            if backups[index].hash == backup.hash {
                let mut existing = backups.remove(index);
                existing.created_at = backup.created_at;
                existing.reason = backup.reason;
                backups.insert(0, existing.clone());
                return Ok(existing);
            }
        }
        backups.insert(0, backup.clone());
        let mut seen = 0;
        backups.retain(|item| {
            if item.kind != "automatic" {
                return true;
            }
            seen += 1;
            seen <= MAX_AUTOMATIC
        });
        return Ok(backup);
    }
    if backups.iter().filter(|item| item.kind == "manual").count() >= MAX_MANUAL {
        return Err(err_with(
            "BACKUP_LIMIT",
            format!("You can keep at most {MAX_MANUAL} manual backups. Delete an old one first."),
            json!({"max":MAX_MANUAL}),
        ));
    }
    backups.insert(0, backup.clone());
    Ok(backup)
}

fn content_hash(adapters: &[Value], routes: &[Value], hosts: &Option<Value>) -> String {
    let adapters = adapters
        .iter()
        .map(|item| json!([item["guid"], item["target"], item["metrics"], item["networkCategory"], item["ipError"]]))
        .collect::<Vec<_>>();
    fingerprint(&json!({"adapters":adapters,"routes":routes,"hosts":hosts.as_ref().map(|item| item["hash"].clone())}))
}

/// Which parts of a backup to restore (all by default).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Include {
    pub ip: bool,
    pub metric: bool,
    pub category: bool,
    pub routes: bool,
    pub hosts: bool,
}

impl Include {
    pub fn all() -> Self {
        Self { ip: true, metric: true, category: true, routes: true, hosts: true }
    }
    pub fn from(value: Option<&Value>) -> Self {
        let flag = |key: &str| value.and_then(|item| item.get(key)).and_then(Value::as_bool).unwrap_or(true);
        Self { ip: flag("ip"), metric: flag("metric"), category: flag("category"), routes: flag("routes"), hosts: flag("hosts") }
    }
    fn to_value(self) -> Value {
        json!({"ip":self.ip,"metric":self.metric,"category":self.category,"routes":self.routes,"hosts":self.hosts})
    }
}

/// Live state a restore is planned against.
pub struct Live<'a> {
    pub interfaces: &'a [Value],
    /// IPv4/DNS state by normalized GUID key (IP_STATE shape).
    pub ip_states: &'a HashMap<String, Value>,
    pub routes: &'a [Value],
    /// The app-managed route records in the configuration now.
    pub managed: &'a [Value],
    pub hosts_hash: Option<String>,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub items: Vec<Value>,
    pub skipped: Vec<Value>,
    pub unchanged: usize,
}

fn skip(kind: &str, alias: &str, code: &str, text: String) -> Value {
    let mut item = notice(code, json!({"alias":alias}), text);
    item["kind"] = json!(kind);
    item["alias"] = json!(alias);
    item
}

fn metric_value(metric: &Value) -> Option<Value> {
    if metric.is_null() {
        return None;
    }
    let automatic = metric.get("automaticMetric").and_then(Value::as_bool).unwrap_or(true);
    Some(json!({"automatic":automatic,"metric":metric.get("interfaceMetric").and_then(Value::as_i64)}))
}

fn same_metric(a: &Value, b: &Value) -> bool {
    a["automatic"] == b["automatic"] && (a["automatic"] == json!(true) || a["metric"] == b["metric"])
}

/// A usable IPv4 target from a backup, or None when it cannot be applied.
fn usable_ipv4(target: &Value) -> Option<Value> {
    match target.get("mode").and_then(Value::as_str) {
        Some("dhcp") => Some(json!({"mode":"dhcp"})),
        Some("static") => {
            let address = field_string(target, "address");
            let prefix = field_i64(target, "prefixLength");
            let gateway = target.get("gateway").and_then(Value::as_str).map(str::to_owned);
            config_edit::validate_static_ipv4(&address, prefix, gateway.as_deref()).ok()?;
            Some(json!({"mode":"static","address":address,"prefixLength":prefix,"gateway":gateway}))
        }
        _ => None,
    }
}

fn usable_dns(target: &Value) -> Option<Value> {
    match target.get("mode").and_then(Value::as_str) {
        Some("automatic") => Some(json!({"mode":"automatic"})),
        Some("manual") => config_edit::normalize_dns(target.get("servers").unwrap_or(&Value::Null))
            .ok()
            .map(|servers| json!({"mode":"manual","servers":servers})),
        _ => None,
    }
}

fn route_identity(route: &Value, index: i64) -> Value {
    json!({
        "addressFamily":field_string(route, "addressFamily"),
        "destinationPrefix":field_string(route, "destinationPrefix"),
        "nextHop":field_string(route, "nextHop"),
        "interfaceIndex":index,
    })
}

/// Compare a backup with live state and list what a restore would change.
pub fn plan_restore(backup: &Backup, live: &Live, include: Include) -> Plan {
    let mut plan = Plan::default();
    let by_guid = live
        .interfaces
        .iter()
        .map(|item| (normalized_guid_key(&field_string(item, "guid")), item))
        .filter(|(key, _)| !key.is_empty())
        .collect::<HashMap<_, _>>();
    for saved in &backup.adapters {
        let key = normalized_guid_key(&field_string(saved, "guid"));
        let alias = field_string(saved, "alias");
        let Some(actual) = by_guid.get(&key).copied() else {
            plan.skipped.push(skip("adapter", &alias, "restore.skip.missing", format!("{alias} no longer exists or was replaced by a different adapter.")));
            continue;
        };
        let index = field_i64(actual, "interfaceIndex");
        let live_alias = field_string(actual, "alias");
        let base = |kind: &str, suffix: &str| {
            json!({"key":format!("{kind}:{key}{suffix}"),"kind":kind,"guid":key,"alias":live_alias,"interfaceIndex":index})
        };
        let owner = coexistence::owner_of(actual).or_else(|| saved.get("owner").and_then(Value::as_str).map(str::to_owned));
        let system = field_bool(actual, "systemInterface") || field_bool(saved, "systemInterface");

        if include.ip && !saved["target"].is_null() {
            if let Some(owner) = owner.as_ref() {
                plan.skipped.push(skip("ip", &live_alias, "restore.skip.protected", format!("{live_alias} is managed by {owner}; its addresses and DNS are left to {owner}.")));
            } else if system {
                plan.skipped.push(skip("ip", &live_alias, "restore.skip.system", format!("{live_alias} is a system interface and is not changed.")));
            } else if let Some(current) = live.ip_states.get(&key) {
                let before = config_edit::current_target(current);
                let saved_v4 = usable_ipv4(&saved["target"]["ipv4"]);
                let saved_dns = usable_dns(&saved["target"]["dns"]);
                if saved_v4.is_none() || saved_dns.is_none() {
                    plan.skipped.push(skip("ip", &live_alias, "restore.skip.invalid", format!("The saved IP settings of {live_alias} are incomplete and cannot be restored.")));
                } else {
                    let ipv4 = saved_v4.filter(|value| value != &before["ipv4"]);
                    let dns = saved_dns.filter(|value| value != &before["dns"]);
                    let static_after = ipv4.as_ref().map(|value| value["mode"] == "static").unwrap_or(before["ipv4"]["mode"] == "static");
                    let dns_after = dns.as_ref().unwrap_or(&before["dns"]);
                    if ipv4.is_none() && dns.is_none() {
                        plan.unchanged += 1;
                    } else if static_after && dns_after["mode"] == "automatic" {
                        plan.skipped.push(skip("ip", &live_alias, "restore.skip.invalid", format!("The saved IP settings of {live_alias} are incomplete and cannot be restored.")));
                    } else {
                        let rollback = json!({
                            "ipv4":if ipv4.is_some() && (before["ipv4"]["mode"] == "dhcp" || !before["ipv4"]["address"].is_null()) { before["ipv4"].clone() } else { Value::Null },
                            "dns":if dns.is_some() { before["dns"].clone() } else { Value::Null },
                        });
                        let mut item = base("ip", "");
                        item["target"] = json!({"ipv4":ipv4,"dns":dns});
                        item["rollback"] = rollback;
                        item["before"] = before;
                        plan.items.push(item);
                    }
                }
            } else {
                plan.skipped.push(skip("ip", &live_alias, "restore.skip.unreadable", format!("The current IP settings of {live_alias} could not be read, so they were not changed.")));
            }
        }

        if include.metric {
            for (family, field) in [("IPv4", "ipv4"), ("IPv6", "ipv6")] {
                let (Some(after), Some(before)) = (metric_value(&saved["metrics"][field]), metric_value(&actual["metrics"][field])) else {
                    continue;
                };
                if same_metric(&after, &before) {
                    plan.unchanged += 1;
                    continue;
                }
                let mut item = base("metric", &format!(":{family}"));
                item["addressFamily"] = json!(family);
                item["before"] = before;
                item["after"] = after;
                item["owner"] = json!(owner);
                plan.items.push(item);
            }
        }

        if include.category {
            let after = field_string(saved, "networkCategory");
            let before = field_string(actual, "networkCategory");
            if !after.is_empty() && !before.is_empty() {
                if after == before {
                    plan.unchanged += 1;
                } else if !config_edit::CATEGORIES.contains(&after.as_str()) || !config_edit::CATEGORIES.contains(&before.as_str()) {
                    plan.skipped.push(skip("category", &live_alias, "restore.skip.domain", format!("{live_alias} is on a domain network; its category is managed by your organization.")));
                } else {
                    let mut item = base("category", "");
                    item["before"] = json!(before);
                    item["after"] = json!(after);
                    item["owner"] = json!(owner);
                    plan.items.push(item);
                }
            }
        }
    }

    if include.routes {
        let live_keys = live.routes.iter().map(route_key).collect::<HashSet<_>>();
        let index_of = |guid: &str| by_guid.get(&normalized_guid_key(guid)).map(|item| field_i64(item, "interfaceIndex"));
        let mut wanted = HashSet::new();
        for route in &backup.routes {
            let destination = field_string(route, "destinationPrefix");
            if coexistence::is_capture_prefix(&destination) {
                continue;
            }
            let Some(index) = index_of(&field_string(route, "interfaceGuid")) else {
                let alias = field_string(route, "interfaceAlias");
                plan.skipped.push(skip("route", &alias, "restore.skip.routeAdapter", format!("The route to {destination} used {alias}, which no longer exists.")));
                continue;
            };
            let identity = route_identity(route, index);
            let key = route_key(&identity);
            wanted.insert(key.clone());
            if live_keys.contains(&key) {
                plan.unchanged += 1;
                continue;
            }
            let mut record = identity;
            record["routeMetric"] = json!(field_i64(route, "routeMetric").max(0));
            record["persistent"] = json!(field_bool(route, "persistent"));
            record["interfaceAlias"] = json!(field_string(route, "interfaceAlias"));
            plan.items.push(json!({"key":format!("route:{key}"),"kind":"routeAdd","route":record}));
        }
        for managed in live.managed {
            let key = route_key(managed);
            if wanted.contains(&key) || coexistence::is_capture_prefix(&field_string(managed, "destinationPrefix")) {
                continue;
            }
            if let Some(actual) = live.routes.iter().find(|route| route_key(route) == key) {
                plan.items.push(json!({"key":format!("route:{key}"),"kind":"routeRemove","route":crate::route_remove_payload(actual)}));
            }
        }
    }

    if include.hosts {
        match backup.hosts.as_ref().and_then(|item| item.get("hash")).and_then(Value::as_str) {
            Some(hash) if valid_hosts_hash(hash) => match live.hosts_hash.as_deref() {
                Some(current) if current == hash => plan.unchanged += 1,
                Some(current) => plan.items.push(json!({"key":"hosts","kind":"hosts","hash":hash,"currentHash":current})),
                None => plan.skipped.push(skip("hosts", "", "restore.skip.hostsUnreadable", "The current hosts file cannot be read, so it was not restored.".into())),
            },
            _ => plan.skipped.push(skip("hosts", "", "restore.skip.hostsUnavailable", "This backup has no copy of the hosts file.".into())),
        }
    }
    plan
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

fn config_dir(state: &State<AppState>) -> PathBuf {
    state.config_path.parent().map(PathBuf::from).unwrap_or_default()
}
fn store_path(state: &State<AppState>) -> PathBuf {
    config_dir(state).join(STORE_FILE)
}
fn hosts_dir(state: &State<AppState>) -> PathBuf {
    config_dir(state).join(HOSTS_DIR)
}
fn hosts_file(state: &State<AppState>, hash: &str) -> Option<PathBuf> {
    valid_hosts_hash(hash).then(|| hosts_dir(state).join(format!("hosts-{hash}.txt")))
}

fn load(state: &State<AppState>) -> Store {
    let path = store_path(state);
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|_| {
            // Keep an unreadable file for inspection instead of overwriting it.
            let _ = fs::rename(&path, path.with_extension(format!("corrupt-{}.json", Utc::now().format("%Y%m%d%H%M%S"))));
            Store::default()
        }),
        Err(_) => Store::default(),
    }
}

fn save(state: &State<AppState>, store: &Store) -> AppResult<()> {
    let path = store_path(state);
    let temp = path.with_extension("tmp");
    let text = serde_json::to_string(store).map_err(|_| err("BACKUP_FAILED", "Cannot serialize network backups."))?;
    fs::write(&temp, text)
        .and_then(|_| fs::rename(&temp, &path))
        .map_err(|e| err_with("BACKUP_FAILED", "Cannot save network backups.", json!({"reason":e.to_string()})))?;
    // Remove hosts copies no backup refers to any more.
    let used = store
        .backups
        .iter()
        .filter_map(|item| item.hosts.as_ref().and_then(|hosts| hosts.get("hash")).and_then(Value::as_str))
        .map(|hash| format!("hosts-{hash}.txt"))
        .collect::<HashSet<_>>();
    if let Ok(entries) = fs::read_dir(hosts_dir(state)) {
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().to_string();
            let managed = name.strip_prefix("hosts-").and_then(|rest| rest.strip_suffix(".txt")).map(valid_hosts_hash).unwrap_or(false);
            if managed && !used.contains(&name) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    Ok(())
}

/// Read-modify-write the store under the backups lock.
fn with_store<T>(state: &State<AppState>, change: impl FnOnce(&mut Vec<Backup>) -> AppResult<T>) -> AppResult<T> {
    let _guard = state.backups_lock.lock().map_err(|_| err("STATE_ERROR", "Cannot access network backups."))?;
    let mut store = load(state);
    let result = change(&mut store.backups)?;
    store.version = 1;
    save(state, &store)?;
    Ok(result)
}

fn find(state: &State<AppState>, id: &str) -> AppResult<Backup> {
    load(state)
        .backups
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| err("NOT_FOUND", "The backup no longer exists."))
}

// ---------------------------------------------------------------------------
// Capture (read-only)
// ---------------------------------------------------------------------------

/// IPv4/DNS state for several adapters, keyed by interface index. Adapters
/// that cannot be read are simply missing from the result.
fn ip_states(indexes: &[i64]) -> HashMap<i64, Value> {
    if indexes.is_empty() {
        return HashMap::new();
    }
    run_powershell_json(config_edit::IP_STATE_ALL_SCRIPT, Some(&json!({"indexes":indexes})))
        .map(|value| {
            arrays(Some(&value))
                .into_iter()
                .filter(|item| item.get("state").is_some_and(Value::is_object))
                .map(|item| (field_i64(&item, "interfaceIndex"), item["state"].clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Adapters whose IP settings can be restored: not system, not owned by
/// tunnel/proxy software, and with IPv4 enabled.
fn ip_restorable(adapter: &Value) -> bool {
    !field_bool(adapter, "systemInterface") && coexistence::owner_of(adapter).is_none() && !adapter.pointer("/metrics/ipv4").unwrap_or(&Value::Null).is_null()
}

fn capture(state: &State<AppState>, snapshot: &Value, kind: &str, name: String, reason: Option<Value>) -> AppResult<Backup> {
    let interfaces = arrays(snapshot.pointer("/overview/interfaces"))
        .into_iter()
        .filter(|item| !field_bool(item, "systemInterface") && !field_string(item, "guid").is_empty())
        .collect::<Vec<_>>();
    let readable = interfaces.iter().filter(|item| ip_restorable(item)).map(|item| field_i64(item, "interfaceIndex")).collect::<Vec<_>>();
    let states = ip_states(&readable);
    let adapters = interfaces
        .iter()
        .map(|item| {
            let index = field_i64(item, "interfaceIndex");
            let mut adapter = json!({
                "guid":normalized_guid_key(&field_string(item, "guid")),
                "alias":field_string(item, "alias"),
                "interfaceIndex":index,
                "description":field_string(item, "description"),
                "owner":coexistence::owner_of(item),
                "systemInterface":false,
                "metrics":item.get("metrics").cloned().unwrap_or(Value::Null),
                "networkCategory":item.get("networkCategory").cloned().unwrap_or(Value::Null),
                "target":Value::Null,
            });
            if let Some(current) = states.get(&index) {
                adapter["ipState"] = current.clone();
                adapter["target"] = config_edit::current_target(current);
            } else if readable.contains(&index) {
                adapter["ipError"] = json!(true);
            }
            adapter
        })
        .collect::<Vec<_>>();
    let guid_of = |index: i64| {
        interfaces
            .iter()
            .find(|item| field_i64(item, "interfaceIndex") == index)
            .map(|item| normalized_guid_key(&field_string(item, "guid")))
    };
    let managed = config(state).managed_routes.iter().map(route_key).collect::<HashSet<_>>();
    let routes = arrays(snapshot.pointer("/routes/routes"))
        .into_iter()
        .filter(|route| managed.contains(&route_key(route)))
        .filter_map(|route| {
            let guid = guid_of(field_i64(&route, "interfaceIndex"))?;
            let mut record = route_identity(&route, field_i64(&route, "interfaceIndex"));
            record["routeMetric"] = json!(field_i64(&route, "routeMetric"));
            record["persistent"] = json!(field_bool(&route, "persistent"));
            record["interfaceAlias"] = json!(field_string(&route, "interfaceAlias"));
            record["interfaceGuid"] = json!(guid);
            Some(record)
        })
        .collect::<Vec<_>>();
    let hosts = match advanced::read_hosts() {
        Ok(text) => {
            let hash = fingerprint(&json!(text));
            let path = hosts_file(state, &hash).ok_or_else(|| err("BACKUP_FAILED", "Invalid hosts fingerprint."))?;
            if !path.exists() {
                fs::create_dir_all(hosts_dir(state))
                    .and_then(|_| fs::write(&path, &text))
                    .map_err(|e| err_with("BACKUP_FAILED", "Cannot save the hosts copy for the backup.", json!({"reason":e.to_string()})))?;
            }
            let entries = advanced::parse_hosts(&text).iter().filter(|line| line["kind"] == "entry").count();
            Some(json!({"hash":hash,"entries":entries}))
        }
        Err(_) => Some(json!({"unavailable":"unreadable"})),
    };
    let hash = content_hash(&adapters, &routes, &hosts);
    Ok(Backup {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        kind: kind.into(),
        created_at: Utc::now().to_rfc3339(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        reason,
        adapters,
        routes,
        hosts,
        hash,
    })
}

/// Create and store a backup from an already-read network snapshot.
pub fn create(state: &State<AppState>, snapshot: &Value, kind: &str, name: String, reason: Option<Value>) -> AppResult<Backup> {
    let backup = capture(state, snapshot, kind, name, reason)?;
    with_store(state, |backups| insert(backups, backup))
}

// ---------------------------------------------------------------------------
// Restore
// ---------------------------------------------------------------------------

/// Plan a restore against fresh live state.
fn live_plan(state: &State<AppState>, backup: &Backup, include: Include) -> AppResult<(Plan, Vec<Value>)> {
    let snapshot = live_snapshot()?;
    let interfaces = arrays(snapshot.pointer("/overview/interfaces"));
    let routes = arrays(snapshot.pointer("/routes/routes"));
    let mut ip_states_by_guid = HashMap::new();
    if include.ip {
        let wanted = backup.adapters.iter().map(|item| normalized_guid_key(&field_string(item, "guid"))).collect::<HashSet<_>>();
        let targets = interfaces
            .iter()
            .filter(|item| wanted.contains(&normalized_guid_key(&field_string(item, "guid"))) && ip_restorable(item))
            .map(|item| (field_i64(item, "interfaceIndex"), normalized_guid_key(&field_string(item, "guid"))))
            .collect::<Vec<_>>();
        let states = ip_states(&targets.iter().map(|(index, _)| *index).collect::<Vec<_>>());
        for (index, key) in targets {
            if let Some(value) = states.get(&index) {
                ip_states_by_guid.insert(key, value.clone());
            }
        }
    }
    let managed = config(state).managed_routes;
    let hosts_hash = advanced::read_hosts().ok().map(|text| fingerprint(&json!(text)));
    let live = Live { interfaces: &interfaces, ip_states: &ip_states_by_guid, routes: &routes, managed: &managed, hosts_hash };
    Ok((plan_restore(backup, &live, include), interfaces))
}

fn adapter_for<'a>(interfaces: &'a [Value], item: &Value) -> Option<&'a Value> {
    interfaces.iter().find(|adapter| normalized_guid_key(&field_string(adapter, "guid")) == field_string(item, "guid"))
}

/// Warnings for a planned restore (computed in Rust, shown in the dialog).
fn plan_notices(plan: &Plan, interfaces: &[Value]) -> Vec<Value> {
    let mut notices = vec![];
    for item in &plan.items {
        let alias = field_string(item, "alias");
        match field_string(item, "kind").as_str() {
            "ip" => {
                if let Some(adapter) = adapter_for(interfaces, item) {
                    notices.extend(config_edit::ip_change_notices(adapter, item));
                }
            }
            "metric" => {
                if adapter_for(interfaces, item).is_some_and(|adapter| field_bool(adapter, "defaultRoute")) {
                    notices.push(notice("metric.defaultGateway", json!({"alias":alias}), "This interface carries a default gateway; changing Metric can alter the default network path."));
                }
                if let Some(owner) = item.get("owner").and_then(Value::as_str) {
                    notices.push(notice("metric.protected", json!({"alias":alias,"owner":owner}), format!("{alias} is managed by {owner}. {owner} may reset this Metric when it restarts or recreates the adapter.")));
                }
            }
            "category" => {
                if let Some(adapter) = adapter_for(interfaces, item) {
                    notices.extend(config_edit::category_notices(adapter, &field_string(item, "after")));
                }
            }
            "routeAdd" | "routeRemove" => {
                if field_string(&item["route"], "destinationPrefix").ends_with("/0") {
                    notices.push(notice("route.add.defaultRoute", json!({}), "This is a default route and can change network connectivity."));
                }
            }
            "hosts" => notices.push(notice("restore.hostsOverwrite", json!({}), "The hosts file is replaced by the copy in this backup. Entries added since then are removed.")),
            _ => {}
        }
    }
    notices.push(notice(
        "restore.notIncluded",
        json!({}),
        "Not restored: Wi-Fi passwords and saved Wi-Fi profiles, the system proxy, adapters turned on or off, IPv6 static addresses, routes not created by this app, and the addresses and DNS of tunnel/proxy adapters.",
    ));
    notices.push(config_edit::revert_notice());
    // The same warning for two adapters reads as noise; keep one per text.
    let mut seen = HashSet::new();
    notices.retain(|item| seen.insert(field_string(item, "text")));
    notices
}

fn result_of(item: &Value, result: AppResult<Value>) -> Value {
    let mut value = match result {
        Ok(result) => json!({"status":"success","result":result}),
        Err(error) => json!({"status":"failed","error":error}),
    };
    value["kind"] = item["kind"].clone();
    value["alias"] = item["alias"].clone();
    value["key"] = item["key"].clone();
    value
}

/// Apply planned items in a safe order: hosts, route removals, category,
/// Metric, IPv4/DNS, then route additions.
fn execute(state: &State<AppState>, items: &[Value]) -> Vec<Value> {
    let order = ["hosts", "routeRemove", "category", "metric", "ip", "routeAdd"];
    let mut results = vec![];
    let mut added = vec![];
    let mut removed = vec![];
    for kind in order {
        for item in items.iter().filter(|item| item["kind"] == kind) {
            let result = match kind {
                "hosts" => {
                    let hash = field_string(item, "hash");
                    hosts_file(state, &hash)
                        .ok_or_else(|| err("NOT_FOUND", "The hosts copy is missing."))
                        .and_then(|path| fs::read_to_string(path).map_err(|_| err("NOT_FOUND", "The hosts copy for this backup is missing.")))
                        .and_then(|text| {
                            if fingerprint(&json!(text)) != hash {
                                return Err(err("VALIDATION_ERROR", "The hosts copy for this backup is damaged."));
                            }
                            advanced::write_hosts(&text).map(|_| json!({"restored":true}))
                        })
                }
                "routeRemove" => crate::remove_route_record(&item["route"]).inspect(|_| removed.push(route_key(&item["route"]))),
                "routeAdd" => crate::add_route_record(&item["route"]).inspect(|_| added.push(item["route"].clone())),
                "category" => run_powershell_json(
                    config_edit::CATEGORY_SCRIPT,
                    Some(&json!({"interfaceIndex":item["interfaceIndex"],"category":item["after"]})),
                ),
                "metric" => crate::ps_set_metric(&json!({
                    "interfaceIndex":item["interfaceIndex"],
                    "addressFamily":item["addressFamily"],
                    "automatic":item["after"]["automatic"],
                    "metric":item["after"]["metric"],
                })),
                "ip" => run_powershell_json(
                    config_edit::APPLY_IP_SCRIPT,
                    Some(&json!({"interfaceIndex":item["interfaceIndex"],"target":item["target"],"rollback":item["rollback"]})),
                ),
                _ => continue,
            };
            results.push(result_of(item, result));
        }
    }
    if !added.is_empty() || !removed.is_empty() {
        let mut cfg = config(state);
        cfg.managed_routes.retain(|route| !removed.contains(&route_key(route)));
        for route in added {
            let key = route_key(&route);
            if !cfg.managed_routes.iter().any(|item| route_key(item) == key) {
                cfg.managed_routes.push(route_identity(&route, field_i64(&route, "interfaceIndex")));
            }
        }
        if let Err(error) = persist(state, &cfg) {
            results.push(json!({"status":"failed","kind":"config","error":error}));
        }
    }
    results
}

/// Re-apply the parts of a backup listed in `item.keys` (automatic revert,
/// "Revert now" and Undo after a backup restore).
pub fn revert(state: &State<AppState>, item: &Value) -> Vec<Value> {
    let id = field_string(item, "backupId");
    let keys = arrays(item.get("keys")).iter().filter_map(Value::as_str).map(str::to_owned).collect::<HashSet<_>>();
    let backup = match find(state, &id) {
        Ok(backup) => backup,
        Err(_) => return vec![json!({"status":"skipped","kind":"backupRevert","message":"The backup taken before the restore no longer exists."})],
    };
    match live_plan(state, &backup, Include::all()) {
        Ok((plan, _)) => {
            let items = plan.items.into_iter().filter(|item| keys.contains(&field_string(item, "key"))).collect::<Vec<_>>();
            execute(state, &items)
        }
        Err(error) => vec![json!({"status":"failed","kind":"backupRevert","error":error})],
    }
}

/// Undo last app change for a backup restore: re-apply the automatic backup
/// taken just before it.
pub fn restore_baseline_scope(state: &State<AppState>, scope: &Value) -> Vec<Value> {
    match scope.get("backupRestore") {
        Some(item) => revert(state, &json!({"backupId":item["preBackupId"],"keys":item["keys"]})),
        None => vec![],
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn summary(backup: &Backup) -> Value {
    json!({
        "id":backup.id,
        "name":backup.name,
        "kind":backup.kind,
        "createdAt":backup.created_at,
        "appVersion":backup.app_version,
        "reason":backup.reason,
        "adapters":backup.adapters.iter().map(|item| json!({
            "alias":item["alias"],
            "owner":item["owner"],
            "target":item["target"],
            "metrics":item["metrics"],
            "networkCategory":item["networkCategory"],
            "ipError":item.get("ipError").cloned().unwrap_or(json!(false)),
        })).collect::<Vec<_>>(),
        "routes":backup.routes.iter().map(|route| json!({
            "destinationPrefix":route["destinationPrefix"],
            "nextHop":route["nextHop"],
            "interfaceAlias":route["interfaceAlias"],
        })).collect::<Vec<_>>(),
        "hosts":backup.hosts,
    })
}

#[tauri::command]
pub fn list_network_backups(state: State<AppState>) -> AppResult<Value> {
    let backups = load(&state).backups;
    Ok(json!({"backups":backups.iter().map(summary).collect::<Vec<_>>(),"limits":{"manual":MAX_MANUAL,"automatic":MAX_AUTOMATIC}}))
}

// Configuration-only write: reads the network, writes only the backup store.
#[tauri::command]
pub async fn create_network_backup(app: AppHandle, input: Option<Value>) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        let name = valid_name(input.as_ref().map(|item| field_string(item, "name")).as_deref().unwrap_or(""))
            .map_err(|message| err("VALIDATION_ERROR", message))?;
        let snapshot = current_snapshot(&state)?;
        let backup = create(&state, &snapshot, "manual", name, None)?;
        let message = notice("backup.created", json!({}), "Backup created.");
        Ok(json!({"backup":summary(&backup),"message":message["text"],"messageCode":message}))
    })
    .await
}

#[tauri::command]
pub fn rename_network_backup(state: State<AppState>, input: Value) -> AppResult<Value> {
    let id = field_string(&input, "id");
    let name = valid_name(&field_string(&input, "name")).map_err(|message| err("VALIDATION_ERROR", message))?;
    with_store(&state, |backups| {
        let item = backups.iter_mut().find(|item| item.id == id).ok_or_else(|| err("NOT_FOUND", "The backup no longer exists."))?;
        item.name = name;
        Ok(())
    })?;
    let message = notice("backup.renamed", json!({}), "Backup renamed.");
    Ok(json!({"renamed":true,"message":message["text"],"messageCode":message}))
}

/// Turn an automatic backup into a manual one so rotation never removes it.
#[tauri::command]
pub fn keep_network_backup(state: State<AppState>, input: Value) -> AppResult<Value> {
    let id = field_string(&input, "id");
    with_store(&state, |backups| {
        if backups.iter().filter(|item| item.kind == "manual").count() >= MAX_MANUAL {
            return Err(err_with("BACKUP_LIMIT", format!("You can keep at most {MAX_MANUAL} manual backups. Delete an old one first."), json!({"max":MAX_MANUAL})));
        }
        let item = backups.iter_mut().find(|item| item.id == id).ok_or_else(|| err("NOT_FOUND", "The backup no longer exists."))?;
        item.kind = "manual".into();
        Ok(())
    })?;
    let message = notice("backup.kept", json!({}), "The backup is now kept until you delete it.");
    Ok(json!({"kept":true,"message":message["text"],"messageCode":message}))
}

#[tauri::command]
pub fn delete_network_backup(state: State<AppState>, input: Value) -> AppResult<Value> {
    let id = field_string(&input, "id");
    with_store(&state, |backups| {
        let before = backups.len();
        backups.retain(|item| item.id != id);
        if backups.len() == before {
            return Err(err("NOT_FOUND", "The backup no longer exists."));
        }
        Ok(())
    })?;
    let message = notice("backup.deleted", json!({}), "Backup deleted.");
    Ok(json!({"deleted":true,"message":message["text"],"messageCode":message}))
}

fn token_payload(backup: &Backup, include: Include, plan: &Plan) -> Value {
    json!({"id":backup.id,"hash":backup.hash,"include":include.to_value(),"plan":fingerprint(&json!(plan.items))})
}

#[tauri::command]
pub async fn preview_backup_restore(app: AppHandle, input: Value) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        let backup = find(&state, &field_string(&input, "id"))?;
        let include = Include::from(input.get("include"));
        let (plan, interfaces) = live_plan(&state, &backup, include)?;
        let notices = plan_notices(&plan, &interfaces);
        let token = if plan.items.is_empty() { None } else { Some(issue_confirmation(&state, "backup-restore", &token_payload(&backup, include, &plan))?) };
        Ok(json!({
            "backup":summary(&backup),
            "include":include.to_value(),
            "changes":plan.items,
            "skipped":plan.skipped,
            "unchangedCount":plan.unchanged,
            "warnings":notice_texts(&notices),
            "notices":notices,
            "confirmationToken":token,
            "requiresAdmin":true,
            "requiresAcknowledgement":true,
        }))
    })
    .await
}

#[tauri::command]
pub async fn apply_backup_restore(app: AppHandle, input: Value) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        if input.get("acknowledged").and_then(Value::as_bool) != Some(true) {
            return Err(err("VALIDATION_ERROR", "Confirm that you understand the restore can interrupt connections."));
        }
        let backup = find(&state, &field_string(&input, "id"))?;
        let include = Include::from(input.get("include"));
        let (plan, _) = live_plan(&state, &backup, include)?;
        consume_confirmation(&state, input.get("confirmationToken").and_then(Value::as_str).unwrap_or(""), "backup-restore", &token_payload(&backup, include, &plan))?;
        require_admin()?;
        config_edit::ensure_no_pending_revert()?;
        let keys = plan.items.iter().map(|item| field_string(item, "key")).collect::<Vec<_>>();
        let pre_id = crate::capture_baseline_with_backup(
            &state,
            json!({"backupRestore":{"id":backup.id,"name":backup.name,"keys":keys}}),
        )?
        .unwrap_or_default();
        let results = execute(&state, &plan.items);
        let revert = config_edit::schedule_revert(&app, json!({"backupRevert":{"backupId":pre_id,"keys":keys}}));
        let partial = results.iter().any(|item| item["status"] == "failed");
        let message = if partial {
            notice("restore.backup.partial", json!({"name":backup.name}), "The backup was only partly restored. Choose Revert now, or Keep and check the failed items.")
        } else {
            notice("restore.backup.complete", json!({"name":backup.name}), "The backup was restored. Keep the settings, or they are reverted automatically.")
        };
        Ok(json!({"results":results,"revert":revert,"partial":partial,"message":message["text"],"messageCode":message}))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUID: &str = "255bc7a7-a80c-47e6-902b-386aa6ee90db";
    const TUN: &str = "11111111-2222-3333-4444-555555555555";

    fn ip(a: u8, b: u8, c: u8, d: u8) -> String {
        std::net::Ipv4Addr::new(a, b, c, d).to_string()
    }
    fn backup(kind: &str, hash: &str) -> Backup {
        Backup { id: uuid::Uuid::new_v4().to_string(), name: String::new(), kind: kind.into(), created_at: "t".into(), app_version: String::new(), reason: None, adapters: vec![], routes: vec![], hosts: None, hash: hash.into() }
    }
    fn static_target(address: &str) -> Value {
        json!({"ipv4":{"mode":"static","address":address,"prefixLength":24,"gateway":ip(192,168,1,1)},"dns":{"mode":"manual","servers":[ip(192,168,1,1)]}})
    }
    fn live_adapter(guid: &str, alias: &str, extra: Value) -> Value {
        let mut adapter = json!({"guid":guid,"alias":alias,"interfaceIndex":7,"metrics":{"ipv4":{"automaticMetric":true,"interfaceMetric":25},"ipv6":null},"networkCategory":"Public"});
        for (key, value) in extra.as_object().unwrap() {
            adapter[key] = value.clone();
        }
        adapter
    }
    fn dhcp_state() -> Value {
        json!({"dhcp":true,"addresses":[],"gateways":[],"dnsServers":[],"dnsStatic":false})
    }
    fn plan_for(backup: &Backup, interfaces: &[Value], states: &HashMap<String, Value>, routes: &[Value], managed: &[Value]) -> Plan {
        plan_restore(backup, &Live { interfaces, ip_states: states, routes, managed, hosts_hash: None }, Include::all())
    }

    #[test]
    fn names_and_hosts_hashes_are_validated() {
        assert_eq!(valid_name("  Home  ").unwrap(), "Home");
        assert_eq!(valid_name("").unwrap(), "");
        assert!(valid_name("a\nb").is_err());
        assert!(valid_name(&"x".repeat(81)).is_err());
        assert!(valid_hosts_hash(&"a".repeat(64)));
        assert!(!valid_hosts_hash(&"A".repeat(64)));
        assert!(!valid_hosts_hash("../hosts"));
    }

    #[test]
    fn automatic_backups_are_taken_only_for_backed_up_settings() {
        for scope in [
            json!({"ipSettings":{"alias":"Wi-Fi"}}),
            json!({"networkCategory":{"alias":"Wi-Fi"}}),
            json!({"metrics":[{}]}),
            json!({"addRoute":{}}),
            json!({"editRoute":{}}),
            json!({"removeRoute":{}}),
            json!({"domainRoute":{}}),
            json!({"hosts":{}}),
            json!({"restoreProfile":"id"}),
            json!({"restoreRecent":"id"}),
            json!({"windowsDefaults":{}}),
            json!({"backupRestore":{"name":"x"}}),
        ] {
            assert!(auto_reason(&scope).is_some(), "{scope}");
        }
        assert_eq!(auto_reason(&json!({"restoreProfile":"id","trigger":true})).unwrap()["code"], "backup.reason.trigger");
        for scope in [json!({"wifi":[]}), json!({"adapterControl":{}}), json!({"wifiProfileMode":{}}), json!({"wifiProfileOrder":{}})] {
            assert!(auto_reason(&scope).is_none(), "{scope}");
        }
    }

    #[test]
    fn automatic_backups_rotate_without_touching_manual_ones() {
        let mut backups = vec![backup("manual", "m")];
        for n in 0..(MAX_AUTOMATIC + 5) {
            insert(&mut backups, backup("automatic", &format!("h{n}"))).unwrap();
        }
        assert_eq!(backups.iter().filter(|item| item.kind == "automatic").count(), MAX_AUTOMATIC);
        assert_eq!(backups.iter().filter(|item| item.kind == "manual").count(), 1);
        assert_eq!(backups[0].hash, format!("h{}", MAX_AUTOMATIC + 4));
    }

    #[test]
    fn identical_automatic_backups_are_refreshed_not_duplicated() {
        let mut backups = vec![];
        let first = insert(&mut backups, backup("automatic", "same")).unwrap();
        let mut again = backup("automatic", "same");
        again.created_at = "later".into();
        let stored = insert(&mut backups, again).unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(stored.id, first.id);
        assert_eq!(backups[0].created_at, "later");
    }

    #[test]
    fn manual_backups_are_capped() {
        let mut backups = (0..MAX_MANUAL).map(|n| backup("manual", &n.to_string())).collect::<Vec<_>>();
        let error = insert(&mut backups, backup("manual", "new")).unwrap_err();
        assert_eq!(error.code, "BACKUP_LIMIT");
        assert!(insert(&mut backups, backup("automatic", "auto")).is_ok());
    }

    #[test]
    fn ip_settings_are_restored_on_normal_adapters() {
        let mut saved = backup("manual", "x");
        saved.adapters = vec![json!({"guid":GUID,"alias":"Ethernet","target":static_target(&ip(192,168,1,20)),"metrics":{"ipv4":{"automaticMetric":true}}})];
        let interfaces = vec![live_adapter(GUID, "Ethernet", json!({}))];
        let states = HashMap::from([(GUID.to_string(), dhcp_state())]);
        let plan = plan_for(&saved, &interfaces, &states, &[], &[]);
        let item = plan.items.iter().find(|item| item["kind"] == "ip").expect("ip item");
        assert_eq!(item["target"]["ipv4"]["address"], ip(192, 168, 1, 20));
        assert_eq!(item["target"]["dns"]["mode"], "manual");
        assert_eq!(item["rollback"]["ipv4"]["mode"], "dhcp");
        assert_eq!(item["rollback"]["dns"]["mode"], "automatic");
        assert_eq!(item["key"], format!("ip:{GUID}"));
    }

    #[test]
    fn ip_settings_of_tunnel_adapters_are_never_restored() {
        let mut saved = backup("manual", "x");
        saved.adapters = vec![json!({
            "guid":TUN,"alias":"EasyTier","owner":"EasyTier","target":static_target(&ip(10,126,126,2)),
            "metrics":{"ipv4":{"automaticMetric":false,"interfaceMetric":5}},"networkCategory":"Private"
        })];
        let interfaces = vec![live_adapter(TUN, "EasyTier", json!({"managedBy":"EasyTier"}))];
        let states = HashMap::from([(TUN.to_string(), dhcp_state())]);
        let plan = plan_for(&saved, &interfaces, &states, &[], &[]);
        assert!(plan.items.iter().all(|item| item["kind"] != "ip"));
        assert!(plan.skipped.iter().any(|item| item["code"] == "restore.skip.protected"));
        // Metric and category may still change there, with the owner recorded for the warning.
        let metric = plan.items.iter().find(|item| item["kind"] == "metric").expect("metric item");
        assert_eq!(metric["owner"], "EasyTier");
        assert!(plan.items.iter().any(|item| item["kind"] == "category" && item["after"] == "Private"));
    }

    #[test]
    fn a_live_owner_protects_an_adapter_even_if_the_backup_did_not_record_one() {
        let mut saved = backup("manual", "x");
        saved.adapters = vec![json!({"guid":TUN,"alias":"Meta","target":static_target(&ip(192,168,1,20))})];
        let interfaces = vec![live_adapter(TUN, "Meta", json!({"managedBy":"mihomo"}))];
        let plan = plan_for(&saved, &interfaces, &HashMap::new(), &[], &[]);
        assert!(plan.items.iter().all(|item| item["kind"] != "ip"));
    }

    #[test]
    fn missing_adapters_domain_networks_and_invalid_targets_are_skipped() {
        let mut saved = backup("manual", "x");
        saved.adapters = vec![
            json!({"guid":"aaaaaaaa-0000-0000-0000-000000000000","alias":"Gone"}),
            json!({"guid":GUID,"alias":"Ethernet","networkCategory":"Private","target":{"ipv4":{"mode":"static","address":null},"dns":{"mode":"automatic"}}}),
        ];
        let interfaces = vec![live_adapter(GUID, "Ethernet", json!({"networkCategory":"DomainAuthenticated"}))];
        let states = HashMap::from([(GUID.to_string(), dhcp_state())]);
        let plan = plan_for(&saved, &interfaces, &states, &[], &[]);
        let codes = plan.skipped.iter().map(|item| field_string(item, "code")).collect::<Vec<_>>();
        assert!(codes.contains(&"restore.skip.missing".to_string()));
        assert!(codes.contains(&"restore.skip.domain".to_string()));
        assert!(codes.contains(&"restore.skip.invalid".to_string()));
        assert!(plan.items.is_empty());
    }

    #[test]
    fn unchanged_settings_produce_no_items() {
        let mut saved = backup("manual", "x");
        saved.adapters = vec![json!({"guid":GUID,"alias":"Ethernet","target":config_edit::current_target(&dhcp_state()),"metrics":{"ipv4":{"automaticMetric":true,"interfaceMetric":40}},"networkCategory":"Public"})];
        let interfaces = vec![live_adapter(GUID, "Ethernet", json!({}))];
        let states = HashMap::from([(GUID.to_string(), dhcp_state())]);
        let plan = plan_for(&saved, &interfaces, &states, &[], &[]);
        assert!(plan.items.is_empty(), "{:?}", plan.items);
        assert_eq!(plan.unchanged, 3);
    }

    #[test]
    fn only_app_managed_routes_are_restored_and_capture_routes_never() {
        let route = |dest: &str| json!({"addressFamily":"IPv4","destinationPrefix":dest,"nextHop":ip(192,168,1,1),"interfaceIndex":3,"routeMetric":5,"persistent":true,"interfaceAlias":"Old","interfaceGuid":GUID});
        let mut saved = backup("manual", "x");
        saved.routes = vec![route(&format!("{}/16", ip(10, 20, 0, 0))), route(&format!("{}/1", ip(0, 0, 0, 0)))];
        let interfaces = vec![live_adapter(GUID, "Ethernet", json!({}))];
        // A route the app added after the backup, present live: it is removed.
        let newer = json!({"addressFamily":"IPv4","destinationPrefix":format!("{}/24", ip(172,16,5,0)),"nextHop":ip(192,168,1,1),"interfaceIndex":7,"policyStores":["ActiveStore"]});
        let plan = plan_for(&saved, &interfaces, &HashMap::new(), &[newer.clone()], &[newer]);
        let adds = plan.items.iter().filter(|item| item["kind"] == "routeAdd").collect::<Vec<_>>();
        assert_eq!(adds.len(), 1);
        // The saved route is re-created on the adapter's current index.
        assert_eq!(adds[0]["route"]["interfaceIndex"], 7);
        assert_eq!(adds[0]["route"]["routeMetric"], 5);
        assert!(plan.items.iter().any(|item| item["kind"] == "routeRemove"));
        assert!(plan.items.iter().all(|item| !field_string(&item["route"], "destinationPrefix").ends_with("/1")));
    }

    #[test]
    fn hosts_are_restored_only_when_different() {
        let mut saved = backup("manual", "x");
        let hash = "b".repeat(64);
        saved.hosts = Some(json!({"hash":hash}));
        let empty = HashMap::new();
        let live = |current: Option<String>| Live { interfaces: &[], ip_states: &empty, routes: &[], managed: &[], hosts_hash: current };
        assert!(plan_restore(&saved, &live(Some(hash.clone())), Include::all()).items.is_empty());
        assert_eq!(plan_restore(&saved, &live(Some("c".repeat(64))), Include::all()).items[0]["kind"], "hosts");
        let mut without = Include::all();
        without.hosts = false;
        assert!(plan_restore(&saved, &live(Some("c".repeat(64))), without).items.is_empty());
    }

    #[test]
    fn include_flags_default_to_everything() {
        assert_eq!(Include::from(None), Include::all());
        let only = Include::from(Some(&json!({"ip":false})));
        assert!(!only.ip && only.metric && only.hosts);
    }

    #[test]
    fn older_backup_files_still_load() {
        let store: Store = serde_json::from_value(json!({"backups":[{"id":"1","name":"n","kind":"manual","createdAt":"t"}]})).unwrap();
        assert_eq!(store.backups[0].adapters.len(), 0);
        assert!(store.backups[0].hosts.is_none());
    }
}
