//! Read-only helpers that turn raw network state into user-facing details:
//! connectivity, status labels, translatable notices, restore previews,
//! undo descriptions and the tray tooltip. Nothing here changes the network.

use crate::{arrays, coexistence::owner_of, field_bool, field_i64, field_string, route_key};
use serde_json::{json, Map, Value};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

// ---------------------------------------------------------------------------
// Translatable notices
// ---------------------------------------------------------------------------

/// A stable code plus parameters lets the UI translate a backend message while
/// `text` keeps the existing English wording as a fallback.
pub fn notice(code: &str, params: Value, text: impl Into<String>) -> Value {
    let params = if params.is_object() { params } else { json!({}) };
    json!({"code":code,"params":params,"text":text.into()})
}

pub fn notice_texts(notices: &[Value]) -> Vec<String> {
    notices.iter().map(|item| field_string(item, "text")).collect()
}

// ---------------------------------------------------------------------------
// Interface status
// ---------------------------------------------------------------------------

/// IF_OPER_STATUS values from GetAdaptersAddresses.
pub fn oper_status_text(value: i32) -> &'static str {
    match value {
        1 => "Up",
        2 => "Down",
        3 => "Testing",
        5 => "Dormant",
        6 => "NotPresent",
        7 => "LowerLayerDown",
        _ => "Unknown",
    }
}

pub fn media_state_text(oper_status: &str) -> &'static str {
    match oper_status {
        "Up" => "Connected",
        "Down" | "LowerLayerDown" | "NotPresent" => "Disconnected",
        _ => "Unknown",
    }
}

pub fn normalized_guid_key(value: &str) -> String {
    value.trim().trim_matches(|c| c == '{' || c == '}').to_ascii_lowercase()
}

pub fn braced_guid(value: &str) -> String {
    let key = value.trim().trim_matches(|c| c == '{' || c == '}').to_ascii_uppercase();
    if key.is_empty() { String::new() } else { format!("{{{key}}}") }
}

// ---------------------------------------------------------------------------
// Connectivity (Network List Manager, read-only)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Connectivity {
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub category: Option<String>,
    pub name: Option<String>,
}

const NLM_IPV4_NOTRAFFIC: i32 = 0x1;
const NLM_IPV6_NOTRAFFIC: i32 = 0x2;
const NLM_IPV4_SUBNET: i32 = 0x10;
const NLM_IPV4_LOCALNETWORK: i32 = 0x20;
const NLM_IPV4_INTERNET: i32 = 0x40;
const NLM_IPV6_SUBNET: i32 = 0x100;
const NLM_IPV6_LOCALNETWORK: i32 = 0x200;
const NLM_IPV6_INTERNET: i32 = 0x400;

/// Map NLM_CONNECTIVITY flags to the same names Get-NetConnectionProfile uses.
pub fn connectivity_level(flags: i32, ipv6: bool) -> &'static str {
    let (internet, local, subnet, no_traffic) = if ipv6 {
        (NLM_IPV6_INTERNET, NLM_IPV6_LOCALNETWORK, NLM_IPV6_SUBNET, NLM_IPV6_NOTRAFFIC)
    } else {
        (NLM_IPV4_INTERNET, NLM_IPV4_LOCALNETWORK, NLM_IPV4_SUBNET, NLM_IPV4_NOTRAFFIC)
    };
    if flags & internet != 0 {
        "Internet"
    } else if flags & local != 0 {
        "LocalNetwork"
    } else if flags & subnet != 0 {
        "Subnet"
    } else if flags & no_traffic != 0 {
        "NoTraffic"
    } else {
        "Disconnected"
    }
}

pub fn category_text(value: i32) -> &'static str {
    match value {
        1 => "Private",
        2 => "DomainAuthenticated",
        _ => "Public",
    }
}

fn level_rank(value: Option<&str>) -> u8 {
    match value {
        Some("Internet") => 5,
        Some("LocalNetwork") => 4,
        Some("Subnet") => 3,
        Some("NoTraffic") => 2,
        Some("Disconnected") => 1,
        _ => 0,
    }
}

/// Several NLM connections can share an adapter; keep the best reachability.
pub fn merge_connectivity(existing: Option<Connectivity>, next: Connectivity) -> Connectivity {
    let Some(mut current) = existing else { return next };
    if level_rank(next.ipv4.as_deref()) > level_rank(current.ipv4.as_deref()) {
        current.ipv4 = next.ipv4;
    }
    if level_rank(next.ipv6.as_deref()) > level_rank(current.ipv6.as_deref()) {
        current.ipv6 = next.ipv6;
    }
    if current.name.is_none() {
        current.name = next.name;
    }
    if current.category.is_none() {
        current.category = next.category;
    }
    current
}

const CONNECTIVITY_TTL: Duration = Duration::from_secs(10);
static CONNECTIVITY_CACHE: Mutex<Option<(Instant, HashMap<String, Connectivity>)>> = Mutex::new(None);

/// Connectivity keyed by lowercase adapter GUID without braces. Failures yield
/// an empty map so discovery never fails because of this optional detail.
pub fn connectivity_by_adapter() -> HashMap<String, Connectivity> {
    if let Ok(cache) = CONNECTIVITY_CACHE.lock() {
        if let Some((at, value)) = cache.as_ref() {
            if at.elapsed() <= CONNECTIVITY_TTL {
                return value.clone();
            }
        }
    }
    // COM is initialized on a private MTA thread so the caller's apartment
    // (possibly the UI thread) is never changed.
    let value = std::thread::spawn(query_network_list_manager)
        .join()
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    if let Ok(mut cache) = CONNECTIVITY_CACHE.lock() {
        *cache = Some((Instant::now(), value.clone()));
    }
    value
}

fn guid_key(guid: &windows::core::GUID) -> String {
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        guid.data1,
        guid.data2,
        guid.data3,
        guid.data4[0],
        guid.data4[1],
        guid.data4[2],
        guid.data4[3],
        guid.data4[4],
        guid.data4[5],
        guid.data4[6],
        guid.data4[7]
    )
}

fn query_network_list_manager() -> windows::core::Result<HashMap<String, Connectivity>> {
    use windows::Win32::Networking::NetworkListManager::{INetworkListManager, NetworkListManager};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };
    struct ComScope(bool);
    impl Drop for ComScope {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }
    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let _scope = ComScope(initialized);
    let mut result: HashMap<String, Connectivity> = HashMap::new();
    unsafe {
        let manager: INetworkListManager = CoCreateInstance(&NetworkListManager, None, CLSCTX_ALL)?;
        let connections = manager.GetNetworkConnections()?;
        for _ in 0..256 {
            let mut items = [None];
            let mut fetched = 0u32;
            connections.Next(&mut items, Some(&mut fetched))?;
            if fetched == 0 {
                break;
            }
            let Some(connection) = items[0].take() else { break };
            let Ok(adapter) = connection.GetAdapterId() else { continue };
            let flags = connection.GetConnectivity().map(|value| value.0).unwrap_or(0);
            let network = connection.GetNetwork().ok();
            let name = network
                .as_ref()
                .and_then(|item| item.GetName().ok())
                .map(|value| value.to_string())
                .filter(|value| !value.trim().is_empty());
            let category = network
                .as_ref()
                .and_then(|item| item.GetCategory().ok())
                .map(|value| category_text(value.0).to_string());
            let key = guid_key(&adapter);
            let entry = Connectivity {
                ipv4: Some(connectivity_level(flags, false).into()),
                ipv6: Some(connectivity_level(flags, true).into()),
                category,
                name,
            };
            let merged = merge_connectivity(result.remove(&key), entry);
            result.insert(key, merged);
        }
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Restore previews
// ---------------------------------------------------------------------------

fn metric_state(metric: &Value) -> (bool, Option<i64>) {
    let automatic = metric.get("automaticMetric").and_then(Value::as_bool).unwrap_or(true);
    let value = metric
        .get("interfaceMetric")
        .and_then(Value::as_i64)
        .or_else(|| metric.get("interfaceMetric").and_then(Value::as_u64).map(|x| x as i64));
    (automatic, value)
}

fn route_summary(route: &Value) -> Value {
    json!({
        "destinationPrefix":field_string(route,"destinationPrefix"),
        "nextHop":field_string(route,"nextHop"),
        "interfaceIndex":field_i64(route,"interfaceIndex"),
        "interfaceAlias":field_string(route,"interfaceAlias"),
        "routeMetric":field_i64(route,"routeMetric"),
        "persistent":field_bool(route,"persistent")
    })
}

/// Find the current adapter for a stored one. Tunnel software recreates its
/// adapters, so a stored GUID must match; an index reused by a different
/// adapter is not the same interface. Older snapshots without a GUID match by
/// index and alias together.
pub fn match_interface<'a>(wanted: &Value, current: &'a [Value]) -> Option<&'a Value> {
    let guid = normalized_guid_key(&field_string(wanted, "guid"));
    if !guid.is_empty() {
        return current
            .iter()
            .find(|item| normalized_guid_key(&field_string(item, "guid")) == guid);
    }
    let index = field_i64(wanted, "interfaceIndex");
    let alias = field_string(wanted, "alias");
    current.iter().find(|item| {
        field_i64(item, "interfaceIndex") == index && (alias.is_empty() || field_string(item, "alias") == alias)
    })
}

/// Describe what restoring `target` (a stored snapshot) would change, using the
/// same scope as `restore_snapshot` for saved configurations and history:
/// Metric, adapter enabled state, Wi-Fi connection and app-managed routes.
pub fn restore_changes(
    target: &Value,
    current_overview: &Value,
    current_routes: &Value,
    managed_routes: &[Value],
) -> (Vec<Value>, usize) {
    let mut changes = vec![];
    let mut unchanged = 0usize;
    let current_interfaces = arrays(current_overview.get("interfaces"));
    for wanted in arrays(target.pointer("/overview/interfaces")) {
        let stored_index = field_i64(&wanted, "interfaceIndex");
        let alias = field_string(&wanted, "alias");
        let Some(actual) = match_interface(&wanted, &current_interfaces) else {
            let reused = current_interfaces
                .iter()
                .any(|item| field_i64(item, "interfaceIndex") == stored_index);
            changes.push(if reused {
                json!({
                    "kind":"skipped","code":"restore.interfaceRecreated",
                    "params":{"alias":alias,"interfaceIndex":stored_index.to_string()},
                    "text":format!("{alias} (#{stored_index}) was replaced by a different adapter and will be skipped.")
                })
            } else {
                json!({
                    "kind":"skipped","code":"restore.interfaceMissing",
                    "params":{"alias":alias,"interfaceIndex":stored_index.to_string()},
                    "text":format!("{alias} (#{stored_index}) no longer exists and will be skipped.")
                })
            });
            continue;
        };
        let index = field_i64(actual, "interfaceIndex");
        for (family, key) in [("IPv4", "ipv4"), ("IPv6", "ipv6")] {
            let Some(metric) = wanted.pointer(&format!("/metrics/{key}")).filter(|x| !x.is_null()) else {
                continue;
            };
            let Some(current) = actual.pointer(&format!("/metrics/{key}")).filter(|x| !x.is_null()) else {
                changes.push(json!({
                    "kind":"skipped","code":"restore.metricUnavailable",
                    "params":{"alias":alias,"family":family},
                    "text":format!("{alias} no longer supports {family}; its Metric will be skipped.")
                }));
                continue;
            };
            let (after_auto, after_value) = metric_state(metric);
            let (before_auto, before_value) = metric_state(current);
            if before_auto == after_auto && (after_auto || before_value == after_value) {
                unchanged += 1;
            } else {
                changes.push(json!({
                    "kind":"metric","interfaceIndex":index,"alias":alias,"addressFamily":family,
                    "before":{"automatic":before_auto,"metric":before_value},
                    "after":{"automatic":after_auto,"metric":if after_auto {None} else {after_value}}
                }));
            }
        }
        if !field_bool(&wanted, "systemInterface") {
            let wanted_disabled = field_string(&wanted, "adminStatus").eq_ignore_ascii_case("down");
            let actual_disabled = field_string(actual, "adminStatus").eq_ignore_ascii_case("down");
            if wanted_disabled == actual_disabled {
                unchanged += 1;
            } else if let Some(owner) = owner_of(actual) {
                changes.push(json!({
                    "kind":"skipped","code":"restore.adapterProtected",
                    "params":{"alias":alias,"owner":owner},
                    "text":format!("{alias} is managed by {owner}; its enabled state will not be changed.")
                }));
            } else {
                changes.push(json!({
                    "kind":"adapter","interfaceIndex":index,"alias":alias,
                    "action":if wanted_disabled {"disable"} else {"enable"}
                }));
            }
        }
        if let Some(wifi) = wanted.get("wifi").filter(|x| x.is_object()) {
            let wanted_connected = field_bool(wifi, "connected");
            let wanted_profile = field_string(wifi, "currentProfile");
            let current_wifi = actual.get("wifi").cloned().unwrap_or(Value::Null);
            let current_connected = field_bool(&current_wifi, "connected");
            let current_profile = field_string(&current_wifi, "currentProfile");
            let before = if current_connected && !current_profile.is_empty() {
                Some(current_profile.clone())
            } else {
                None
            };
            if wanted_connected && !wanted_profile.is_empty() {
                if current_connected && current_profile == wanted_profile {
                    unchanged += 1;
                } else {
                    changes.push(json!({"kind":"wifi","interfaceName":alias,"action":"connect","before":before,"after":wanted_profile}));
                }
            } else if !wanted_connected {
                if current_connected {
                    changes.push(json!({"kind":"wifi","interfaceName":alias,"action":"disconnect","before":before,"after":Value::Null}));
                } else {
                    unchanged += 1;
                }
            }
        }
    }
    let current_items = arrays(current_routes.get("routes"));
    let target_items = arrays(target.pointer("/routes/routes"));
    for managed in managed_routes {
        let key = route_key(managed);
        let current = current_items.iter().find(|item| route_key(item) == key);
        let wanted = target_items.iter().find(|item| route_key(item) == key);
        match (current, wanted) {
            (Some(route), None) => changes.push(json!({"kind":"routeRemove","route":route_summary(route)})),
            (None, Some(route)) => changes.push(json!({"kind":"routeAdd","route":route_summary(route)})),
            (Some(_), Some(_)) => unchanged += 1,
            (None, None) => {}
        }
    }
    (changes, unchanged)
}

// ---------------------------------------------------------------------------
// Undo descriptions
// ---------------------------------------------------------------------------

fn alias_for(index: i64, baseline_state: &Value) -> String {
    arrays(baseline_state.pointer("/overview/interfaces"))
        .iter()
        .find(|item| field_i64(item, "interfaceIndex") == index)
        .map(|item| field_string(item, "alias"))
        .filter(|alias| !alias.is_empty())
        .unwrap_or_else(|| format!("#{index}"))
}

/// Human description of an undo scope. `profile_name` and `recent_name` resolve
/// stored ids to names; unknown ids fall back to the id itself.
pub fn scope_items(
    scope: &Value,
    baseline_state: &Value,
    profile_name: &dyn Fn(&str) -> Option<String>,
    recent_name: &dyn Fn(&str) -> Option<String>,
) -> Vec<Value> {
    let mut items = vec![];
    for metric in arrays(scope.get("metrics")) {
        let alias = alias_for(field_i64(&metric, "interfaceIndex"), baseline_state);
        let family = field_string(&metric, "addressFamily");
        let value = if field_bool(&metric, "automatic") {
            "automatic".to_string()
        } else {
            field_i64(&metric, "metric").to_string()
        };
        items.push(notice(
            "scope.metric",
            json!({"alias":alias,"family":family,"value":value}),
            format!("Changed {family} interface Metric on {alias} to {value}."),
        ));
    }
    if let Some(route) = scope.get("addRoute") {
        let (destination, next_hop) = (field_string(route, "destinationPrefix"), field_string(route, "nextHop"));
        items.push(notice(
            "scope.addRoute",
            json!({"destination":destination,"nextHop":next_hop}),
            format!("Added route {destination} via {next_hop}."),
        ));
    }
    if let Some(edit) = scope.get("editRoute") {
        let old = edit.get("oldRoute").cloned().unwrap_or(Value::Null);
        let (destination, next_hop) = (field_string(&old, "destinationPrefix"), field_string(&old, "nextHop"));
        let mode = {
            let mode = field_string(edit, "mode");
            if mode.is_empty() { "replace".to_string() } else { mode }
        };
        let text = if mode == "persistence" {
            format!("Made route {destination} via {next_hop} persistent.")
        } else {
            format!("Edited route {destination} via {next_hop}.")
        };
        items.push(notice(
            "scope.editRoute",
            json!({"destination":destination,"nextHop":next_hop,"mode":mode}),
            text,
        ));
    }
    if let Some(route) = scope.get("removeRoute") {
        let (destination, next_hop) = (field_string(route, "destinationPrefix"), field_string(route, "nextHop"));
        items.push(notice(
            "scope.removeRoute",
            json!({"destination":destination,"nextHop":next_hop}),
            format!("Removed route {destination} via {next_hop}."),
        ));
    }
    for wifi in arrays(scope.get("wifi")) {
        let name = field_string(&wifi, "interfaceName");
        items.push(notice(
            "scope.wifi",
            json!({"interfaceName":name}),
            format!("Changed the Wi-Fi connection on {name}."),
        ));
    }
    if let Some(control) = scope.get("adapterControl") {
        let alias = alias_for(field_i64(control, "interfaceIndex"), baseline_state);
        let action = field_string(control, "action");
        let verb = if action == "enable" { "Enabled" } else { "Disabled" };
        items.push(notice(
            "scope.adapterControl",
            json!({"alias":alias,"action":action}),
            format!("{verb} adapter {alias}."),
        ));
    }
    if let Some(id) = scope.get("restoreProfile").and_then(Value::as_str) {
        let name = profile_name(id).unwrap_or_else(|| id.to_string());
        items.push(notice(
            "scope.restoreProfile",
            json!({"name":name}),
            format!("Restored saved configuration \"{name}\"."),
        ));
    }
    if let Some(id) = scope.get("restoreRecent").and_then(Value::as_str) {
        let name = recent_name(id).unwrap_or_else(|| id.to_string());
        items.push(notice(
            "scope.restoreRecent",
            json!({"name":name}),
            format!("Restored history snapshot \"{name}\"."),
        ));
    }
    if let Some(defaults) = scope.get("windowsDefaults") {
        let count = arrays(defaults.get("interfaces")).len();
        let routes = arrays(defaults.get("managedRoutes")).len();
        items.push(notice(
            "scope.windowsDefaults",
            json!({"count":count.to_string(),"routes":routes.to_string()}),
            format!("Restored Windows defaults on {count} adapter(s) and removed {routes} app-managed route(s)."),
        ));
    }
    items
}

// ---------------------------------------------------------------------------
// Tray tooltip
// ---------------------------------------------------------------------------

/// The interface that carries the active default route, preferring a
/// connected Wi-Fi adapter when several are eligible.
pub fn active_interface(overview: &Value) -> Option<Value> {
    let eligible = arrays(overview.get("interfaces"))
        .into_iter()
        .filter(|item| {
            field_bool(item, "defaultRoute")
                && !field_bool(item, "systemInterface")
                && !field_string(item, "adminStatus").eq_ignore_ascii_case("down")
        })
        .collect::<Vec<_>>();
    eligible
        .iter()
        .find(|item| item.pointer("/wifi/connected").and_then(Value::as_bool).unwrap_or(false))
        .or_else(|| eligible.first())
        .cloned()
}

pub fn tray_tooltip_text(
    language: &str,
    overview: Option<&Value>,
    enabled_rules: usize,
    failed_rules: usize,
    development: bool,
) -> String {
    let zh = language == "zh-CN";
    let mut lines = vec!["Windows Network Manager".to_string()];
    if let Some(overview) = overview {
        lines.push(match active_interface(overview) {
            Some(item) => {
                let alias = field_string(&item, "alias");
                let ssid = item
                    .get("wifi")
                    .map(|wifi| field_string(wifi, "ssid"))
                    .filter(|ssid| !ssid.is_empty());
                match ssid {
                    Some(ssid) => format!("{alias} · {ssid}"),
                    None => alias,
                }
            }
            None => if zh { "未检测到活动连接".into() } else { "No active connection".into() },
        });
    }
    let mut auto = if enabled_rules == 0 {
        if zh { "自动连接：关闭".to_string() } else { "Auto-connect: off".to_string() }
    } else if zh {
        format!("自动连接：{enabled_rules} 条规则启用")
    } else {
        format!("Auto-connect: {enabled_rules} active")
    };
    if enabled_rules > 0 && development {
        auto.push_str(if zh { "（开发版本中未运行）" } else { " (inactive in development)" });
    }
    if failed_rules > 0 {
        auto.push_str(&if zh { format!(" · {failed_rules} 个错误") } else { format!(" · {failed_rules} error(s)") });
    }
    lines.push(auto);
    // NOTIFYICONDATA tooltips hold 127 characters plus the terminator.
    lines.join("\n").chars().take(127).collect()
}

/// Apply connectivity fields to one interface object.
pub fn apply_connectivity(target: &mut Map<String, Value>, info: Option<&Connectivity>) {
    target.insert("ipv4Connectivity".into(), json!(info.and_then(|x| x.ipv4.clone())));
    target.insert("ipv6Connectivity".into(), json!(info.and_then(|x| x.ipv6.clone())));
    target.insert("networkCategory".into(), json!(info.and_then(|x| x.category.clone())));
    target.insert("networkName".into(), json!(info.and_then(|x| x.name.clone())));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operational_status_maps_to_text_and_media_state() {
        let cases = [
            (1, "Up", "Connected"),
            (2, "Down", "Disconnected"),
            (3, "Testing", "Unknown"),
            (4, "Unknown", "Unknown"),
            (5, "Dormant", "Unknown"),
            (6, "NotPresent", "Disconnected"),
            (7, "LowerLayerDown", "Disconnected"),
            (99, "Unknown", "Unknown"),
        ];
        for (value, text, media) in cases {
            assert_eq!(oper_status_text(value), text);
            assert_eq!(media_state_text(text), media);
        }
    }

    #[test]
    fn guid_helpers_normalize_both_forms() {
        assert_eq!(braced_guid("{abc-123}"), "{ABC-123}");
        assert_eq!(braced_guid("abc-123"), "{ABC-123}");
        assert_eq!(braced_guid(""), "");
        assert_eq!(normalized_guid_key("{ABC-123}"), "abc-123");
    }

    #[test]
    fn connectivity_flags_choose_the_best_level_per_family() {
        assert_eq!(connectivity_level(0x40 | 0x20, false), "Internet");
        assert_eq!(connectivity_level(0x20, false), "LocalNetwork");
        assert_eq!(connectivity_level(0x10, false), "Subnet");
        assert_eq!(connectivity_level(0x1, false), "NoTraffic");
        assert_eq!(connectivity_level(0, false), "Disconnected");
        assert_eq!(connectivity_level(0x400, true), "Internet");
        assert_eq!(connectivity_level(0x40 | 0x2, true), "NoTraffic");
        assert_eq!(category_text(0), "Public");
        assert_eq!(category_text(1), "Private");
        assert_eq!(category_text(2), "DomainAuthenticated");
        let merged = merge_connectivity(
            Some(Connectivity { ipv4: Some("LocalNetwork".into()), ipv6: Some("Internet".into()), category: None, name: Some("Home".into()) }),
            Connectivity { ipv4: Some("Internet".into()), ipv6: Some("NoTraffic".into()), category: Some("Private".into()), name: Some("Other".into()) },
        );
        assert_eq!(merged.ipv4.as_deref(), Some("Internet"));
        assert_eq!(merged.ipv6.as_deref(), Some("Internet"));
        assert_eq!(merged.name.as_deref(), Some("Home"));
        assert_eq!(merged.category.as_deref(), Some("Private"));
    }

    #[test]
    fn notices_keep_code_params_and_text() {
        let items = vec![
            notice("route.add.defaultRoute", json!({}), "A"),
            notice("route.add.samePrefix", json!({"count":"2"}), "B"),
        ];
        assert_eq!(notice_texts(&items), vec!["A", "B"]);
        assert_eq!(items[1]["params"]["count"], "2");
        assert_eq!(notice("x", Value::Null, "y")["params"], json!({}));
    }

    fn interface(index: i64, alias: &str, auto: bool, metric: i64, admin: &str, wifi: Value) -> Value {
        json!({"interfaceIndex":index,"alias":alias,"adminStatus":admin,"systemInterface":false,
            "metrics":{"ipv4":{"automaticMetric":auto,"interfaceMetric":metric},"ipv6":null},"wifi":wifi})
    }

    #[test]
    fn restore_diff_reports_metric_adapter_wifi_and_route_changes() {
        let managed_kept = json!({"addressFamily":"IPv4","destinationPrefix":"10.1.0.0/16","nextHop":"192.168.1.1","interfaceIndex":7,"routeMetric":5,"persistent":true});
        let managed_add = json!({"addressFamily":"IPv4","destinationPrefix":"10.2.0.0/16","nextHop":"192.168.1.1","interfaceIndex":7,"routeMetric":5,"persistent":false});
        let managed_remove = json!({"addressFamily":"IPv4","destinationPrefix":"10.3.0.0/16","nextHop":"192.168.1.1","interfaceIndex":7,"routeMetric":9,"persistent":true});
        let target = json!({
            "overview":{"interfaces":[
                interface(7, "Wi-Fi", false, 10, "Up", json!({"connected":true,"currentProfile":"Home"})),
                interface(8, "Ethernet", true, 25, "Down", Value::Null),
                interface(9, "Gone", true, 25, "Up", Value::Null)
            ]},
            "routes":{"routes":[managed_kept.clone(), managed_add.clone()]}
        });
        let current_overview = json!({"interfaces":[
            interface(7, "Wi-Fi", true, 35, "Up", json!({"connected":true,"currentProfile":"Office"})),
            interface(8, "Ethernet", true, 40, "Up", Value::Null)
        ]});
        let current_routes = json!({"routes":[managed_kept.clone(), managed_remove.clone()]});
        let (changes, unchanged) = restore_changes(
            &target,
            &current_overview,
            &current_routes,
            &[managed_kept, managed_add, managed_remove],
        );
        let kinds = changes.iter().map(|c| field_string(c, "kind")).collect::<Vec<_>>();
        assert_eq!(kinds, vec!["metric", "wifi", "adapter", "skipped", "routeAdd", "routeRemove"]);
        assert_eq!(changes[0]["before"], json!({"automatic":true,"metric":35}));
        assert_eq!(changes[0]["after"], json!({"automatic":false,"metric":10}));
        assert_eq!(changes[1]["action"], "connect");
        assert_eq!(changes[1]["before"], "Office");
        assert_eq!(changes[1]["after"], "Home");
        assert_eq!(changes[2]["action"], "disable");
        assert_eq!(changes[3]["code"], "restore.interfaceMissing");
        assert_eq!(changes[4]["route"]["destinationPrefix"], "10.2.0.0/16");
        assert_eq!(changes[5]["route"]["routeMetric"], 9);
        // Wi-Fi adapter state (Up/Up), Ethernet automatic Metric, and the kept route.
        assert_eq!(unchanged, 3);
    }

    #[test]
    fn restore_diff_is_empty_when_state_matches() {
        let state = json!({
            "overview":{"interfaces":[interface(7, "Wi-Fi", true, 35, "Up", json!({"connected":false}))]},
            "routes":{"routes":[]}
        });
        let mut current = state["overview"].clone();
        // Automatic Metric values may differ; they are computed by Windows.
        current["interfaces"][0]["metrics"]["ipv4"]["interfaceMetric"] = json!(50);
        let (changes, unchanged) = restore_changes(&state, &current, &state["routes"], &[]);
        assert!(changes.is_empty());
        assert_eq!(unchanged, 3);
        let mut connected = current.clone();
        connected["interfaces"][0]["wifi"] = json!({"connected":true,"currentProfile":"Cafe"});
        let (changes, _) = restore_changes(&state, &connected, &state["routes"], &[]);
        assert_eq!(changes[0]["action"], "disconnect");
        assert_eq!(changes[0]["before"], "Cafe");
        assert!(changes[0]["after"].is_null());
    }

    #[test]
    fn restore_matches_recreated_tunnels_by_guid() {
        let stored = |index: i64, guid: &str| {
            let mut item = interface(index, "et_6_0qmn", false, 5, "Down", Value::Null);
            item["guid"] = json!(guid);
            item
        };
        let target = json!({"overview":{"interfaces":[stored(33, "{AAAA-1}")]},"routes":{"routes":[]}});
        // Same index, different GUID: EasyTier recreated the adapter.
        let mut replaced = stored(33, "bbbb-2");
        replaced["adminStatus"] = json!("Up");
        let (changes, _) = restore_changes(&target, &json!({"interfaces":[replaced]}), &json!({}), &[]);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["code"], "restore.interfaceRecreated");
        // Same GUID under a new index: changes use the current index; a
        // protected adapter's enabled state is left to its owner.
        let mut moved = stored(41, "aaaa-1");
        moved["adminStatus"] = json!("Up");
        moved["managedBy"] = json!("EasyTier");
        moved["metrics"]["ipv4"] = json!({"automaticMetric":true,"interfaceMetric":5});
        let (changes, _) = restore_changes(&target, &json!({"interfaces":[moved]}), &json!({}), &[]);
        let kinds = changes.iter().map(|c| field_string(c, "kind")).collect::<Vec<_>>();
        assert_eq!(kinds, vec!["metric", "skipped"]);
        assert_eq!(changes[0]["interfaceIndex"], 41);
        assert_eq!(changes[1]["code"], "restore.adapterProtected");
        assert_eq!(changes[1]["params"]["owner"], "EasyTier");
    }

    #[test]
    fn legacy_snapshots_without_guid_need_matching_alias() {
        let wanted = interface(7, "Wi-Fi", true, 35, "Up", Value::Null);
        let same = [interface(7, "Wi-Fi", true, 35, "Up", Value::Null)];
        let other = [interface(7, "vgate0", true, 35, "Up", Value::Null)];
        assert!(match_interface(&wanted, &same).is_some());
        assert!(match_interface(&wanted, &other).is_none());
    }

    #[test]
    fn scope_items_describe_every_scope_key() {
        let baseline = json!({"overview":{"interfaces":[{"interfaceIndex":7,"alias":"Wi-Fi"}]}});
        let profiles = |id: &str| (id == "p1").then(|| "Home".to_string());
        let recents = |id: &str| (id == "r1").then(|| "Before".to_string());
        let route = json!({"destinationPrefix":"10.0.0.0/8","nextHop":"192.168.1.1"});
        let cases = vec![
            (json!({"metrics":[{"interfaceIndex":7,"addressFamily":"IPv4","automatic":false,"metric":15}]}), "scope.metric", "value", "15"),
            (json!({"metrics":[{"interfaceIndex":3,"addressFamily":"IPv6","automatic":true,"metric":null}]}), "scope.metric", "alias", "#3"),
            (json!({"addRoute":route}), "scope.addRoute", "destination", "10.0.0.0/8"),
            (json!({"editRoute":{"oldRoute":route,"route":route,"mode":"persistence"}}), "scope.editRoute", "mode", "persistence"),
            (json!({"editRoute":{"oldRoute":route,"route":route}}), "scope.editRoute", "mode", "replace"),
            (json!({"removeRoute":route}), "scope.removeRoute", "nextHop", "192.168.1.1"),
            (json!({"wifi":[{"interfaceName":"Wi-Fi"}]}), "scope.wifi", "interfaceName", "Wi-Fi"),
            (json!({"adapterControl":{"interfaceIndex":7,"action":"disable"}}), "scope.adapterControl", "alias", "Wi-Fi"),
            (json!({"restoreProfile":"p1"}), "scope.restoreProfile", "name", "Home"),
            (json!({"restoreRecent":"r1"}), "scope.restoreRecent", "name", "Before"),
            (json!({"restoreRecent":"missing"}), "scope.restoreRecent", "name", "missing"),
            (json!({"windowsDefaults":{"interfaces":[{"interfaceIndex":7},{"interfaceIndex":8}],"managedRoutes":[]}}), "scope.windowsDefaults", "count", "2"),
        ];
        for (scope, code, param, value) in cases {
            let items = scope_items(&scope, &baseline, &profiles, &recents);
            assert_eq!(items.len(), 1, "{scope}");
            assert_eq!(items[0]["code"], code);
            assert_eq!(items[0]["params"][param], value, "{scope}");
            assert!(!field_string(&items[0], "text").is_empty());
        }
        assert!(scope_items(&json!({}), &baseline, &profiles, &recents).is_empty());
    }

    #[test]
    #[ignore = "manual, read-only check of local Network List Manager connectivity"]
    fn read_only_network_list_manager_query() {
        let result = query_network_list_manager().expect("Network List Manager should be available");
        for (key, info) in &result {
            println!("{key}: {info:?}");
        }
        assert!(result.keys().all(|key| key.len() == 36));
    }

    #[test]
    fn tray_tooltip_summarizes_connection_and_rules() {
        let overview = json!({"interfaces":[
            {"alias":"Ethernet","defaultRoute":true,"systemInterface":false,"adminStatus":"Up"},
            {"alias":"Wi-Fi","defaultRoute":true,"systemInterface":false,"adminStatus":"Up","wifi":{"connected":true,"ssid":"HomeWiFi"}}
        ]});
        assert_eq!(
            tray_tooltip_text("en", Some(&overview), 1, 0, false),
            "Windows Network Manager\nWi-Fi · HomeWiFi\nAuto-connect: 1 active"
        );
        let zh = tray_tooltip_text("zh-CN", Some(&json!({"interfaces":[]})), 0, 0, false);
        assert_eq!(zh, "Windows Network Manager\n未检测到活动连接\n自动连接：关闭");
        let dev = tray_tooltip_text("en", None, 2, 1, true);
        assert_eq!(dev, "Windows Network Manager\nAuto-connect: 2 active (inactive in development) · 1 error(s)");
        let long = json!({"interfaces":[{"alias":"x".repeat(300),"defaultRoute":true,"adminStatus":"Up"}]});
        assert!(tray_tooltip_text("en", Some(&long), 0, 0, false).chars().count() <= 127);
    }
}
