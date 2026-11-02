//! Background, read-only monitoring: traffic counters, Wi-Fi signal history
//! and a persisted, capped history of network events. Sampling never changes
//! the network and never contacts outside hosts.
//!
//! Coexistence: tunnel traffic (EasyTier, v2rayN TUN, ...) is already counted
//! on the physical adapter, so the total only sums hardware adapters. Tunnel
//! adapters appear and disappear whenever their owner toggles them, so their
//! events are marked `tunnel` and are not notified by default. A default
//! route captured by a TUN (split `/1` routes) is one event, not one per route.

use crate::{coexistence, network_insights::notice};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fs,
    net::Ipv4Addr,
    path::PathBuf,
    ptr,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;
use windows_sys::{
    core::GUID,
    Win32::{
        NetworkManagement::IpHelper::{
            FreeMibTable, GetIfTable2, GetIpForwardTable2, MIB_IF_TABLE2, MIB_IPFORWARD_TABLE2,
        },
        Networking::WinSock::AF_INET,
    },
};

pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);
/// Wi-Fi, default route and event checks run every fifth sample (10 s).
const SLOW_EVERY: u64 = 5;
const TRAFFIC_SAMPLES: usize = 150;
const SIGNAL_SAMPLES: usize = 360;
pub const MAX_EVENTS: usize = 500;
/// More events than this in one check are notified as a single summary.
const MAX_NOTIFICATIONS: usize = 3;

/// Opt-in tray notifications for network events. Off by default.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationSettings {
    pub enabled: bool,
    pub adapters: bool,
    pub wifi: bool,
    pub default_route: bool,
    pub tunnels: bool,
}
impl Default for NotificationSettings {
    fn default() -> Self {
        Self { enabled: false, adapters: true, wifi: true, default_route: true, tunnels: false }
    }
}

// ---------------------------------------------------------------------------
// Pure logic (unit tested)
// ---------------------------------------------------------------------------

/// Interfaces worth showing: no filter/loopback layers or WAN miniports.
pub fn monitored(alias: &str, description: &str, if_type: u32, filter: bool) -> bool {
    let text = format!("{alias} {description}").to_lowercase();
    !filter
        && if_type != 24
        && ![
            "wan miniport",
            "kernel debug",
            "teredo",
            "isatap",
            "ip-https",
            "6to4",
            "wi-fi direct virtual",
            "loopback",
        ]
        .iter()
        .any(|key| text.contains(key))
}

/// Bytes per second between two counter readings; a reset counter yields 0.
pub fn rate(previous: u64, current: u64, elapsed_ms: u64) -> u64 {
    if elapsed_ms == 0 || current < previous {
        return 0;
    }
    (current - previous).saturating_mul(1000) / elapsed_ms
}

fn push_capped<T>(items: &mut VecDeque<T>, item: T, cap: usize) {
    items.push_back(item);
    while items.len() > cap {
        items.pop_front();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdapterState {
    pub alias: String,
    pub owner: Option<String>,
    pub up: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WifiState {
    pub alias: String,
    pub ssid: Option<String>,
    pub profile: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DefaultRoute {
    pub alias: String,
    pub owner: Option<String>,
    pub split: bool,
}

/// One slow check: adapters by GUID key, Wi-Fi connections (None when Wi-Fi
/// details are unavailable) and the interface carrying default IPv4 traffic.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Observation {
    pub adapters: BTreeMap<String, AdapterState>,
    pub wifi: Option<BTreeMap<String, WifiState>>,
    pub default_route: Option<DefaultRoute>,
}

fn event(kind: &str, code: &str, params: Value, text: String, tunnel: bool) -> Value {
    let mut item = notice(code, params, text);
    item["kind"] = json!(kind);
    item["tunnel"] = json!(tunnel);
    item
}

fn owner_suffix(owner: &Option<String>) -> String {
    owner.as_ref().map(|owner| format!(" ({owner})")).unwrap_or_default()
}

/// Events between two observations. Wi-Fi adapters are reported by their
/// connection, not by adapter state, to avoid duplicates.
pub fn diff_events(previous: &Observation, next: &Observation) -> Vec<Value> {
    let mut events = vec![];
    let wifi_keys = |observation: &Observation| -> Vec<String> {
        observation.wifi.as_ref().map(|wifi| wifi.keys().cloned().collect()).unwrap_or_default()
    };
    let wifi_adapters = [wifi_keys(previous), wifi_keys(next)].concat();
    let mut keys = previous.adapters.keys().chain(next.adapters.keys()).cloned().collect::<Vec<_>>();
    keys.sort();
    keys.dedup();
    for key in keys.iter().filter(|key| !wifi_adapters.contains(key)) {
        let before = previous.adapters.get(key);
        let after = next.adapters.get(key);
        let was_up = before.map(|item| item.up).unwrap_or(false);
        let is_up = after.map(|item| item.up).unwrap_or(false);
        if was_up == is_up {
            continue;
        }
        let item = after.or(before).expect("adapter in one observation");
        let tunnel = item.owner.is_some();
        let params = json!({"alias":item.alias,"owner":item.owner});
        events.push(if is_up {
            event("adapter", "event.adapterUp", params, format!("{}{} is connected.", item.alias, owner_suffix(&item.owner)), tunnel)
        } else {
            event("adapter", "event.adapterDown", params, format!("{}{} is disconnected.", item.alias, owner_suffix(&item.owner)), tunnel)
        });
    }
    if let (Some(before), Some(after)) = (&previous.wifi, &next.wifi) {
        let mut keys = before.keys().chain(after.keys()).cloned().collect::<Vec<_>>();
        keys.sort();
        keys.dedup();
        for key in keys {
            let old = before.get(&key);
            let new = after.get(&key);
            let alias = new.or(old).map(|item| item.alias.clone()).unwrap_or_default();
            let old_ssid = old.and_then(|item| item.ssid.clone());
            let new_ssid = new.and_then(|item| item.ssid.clone());
            let profile = new.and_then(|item| item.profile.clone());
            match (old_ssid, new_ssid) {
                (None, Some(ssid)) => events.push(event(
                    "wifi",
                    "event.wifiConnected",
                    json!({"alias":alias,"ssid":ssid,"adapterGuid":key,"profile":profile}),
                    format!("{alias} connected to {ssid}."),
                    false,
                )),
                (Some(ssid), None) => events.push(event(
                    "wifi",
                    "event.wifiDisconnected",
                    json!({"alias":alias,"ssid":ssid}),
                    format!("{alias} disconnected from {ssid}."),
                    false,
                )),
                (Some(from), Some(to)) if from != to => events.push(event(
                    "wifi",
                    "event.wifiSwitched",
                    json!({"alias":alias,"from":from,"to":to,"adapterGuid":key,"profile":profile}),
                    format!("{alias} switched from {from} to {to}."),
                    false,
                )),
                _ => {}
            }
        }
    }
    match (&previous.default_route, &next.default_route) {
        (Some(from), Some(to)) if from.alias != to.alias || from.split != to.split => {
            let params = json!({"from":from.alias,"to":to.alias,"owner":to.owner});
            events.push(match &to.owner {
                Some(owner) => event(
                    "defaultRoute",
                    "event.defaultRouteTunnel",
                    params,
                    format!("Default traffic now goes through {} ({owner}).", to.alias),
                    false,
                ),
                None => event(
                    "defaultRoute",
                    "event.defaultRouteChanged",
                    params,
                    format!("Default traffic moved from {} to {}.", from.alias, to.alias),
                    false,
                ),
            });
        }
        (Some(from), None) => events.push(event(
            "defaultRoute",
            "event.defaultRouteLost",
            json!({"from":from.alias}),
            format!("No default route: {} no longer carries default traffic.", from.alias),
            false,
        )),
        (None, Some(to)) => events.push(event(
            "defaultRoute",
            "event.defaultRouteRestored",
            json!({"to":to.alias,"owner":to.owner}),
            format!("Default traffic goes through {}{}.", to.alias, owner_suffix(&to.owner)),
            false,
        )),
        _ => {}
    }
    events
}

/// The interface index carrying default IPv4 traffic: split `/1` routes on an
/// up interface always win; otherwise the lowest effective `/0` Metric.
pub fn pick_default(
    routes: &[(u32, Ipv4Addr, u8, u32)],
    interface_metric: impl Fn(u32) -> u32,
    up: impl Fn(u32) -> bool,
) -> Option<(u32, bool)> {
    let split = routes.iter().find(|(index, destination, length, _)| {
        *length == 1
            && (*destination == Ipv4Addr::new(0, 0, 0, 0) || *destination == Ipv4Addr::new(128, 0, 0, 0))
            && up(*index)
    });
    if let Some((index, ..)) = split {
        return Some((*index, true));
    }
    routes
        .iter()
        .filter(|(index, destination, length, _)| *length == 0 && destination.is_unspecified() && up(*index))
        .min_by_key(|(index, _, _, metric)| metric.saturating_add(interface_metric(*index)))
        .map(|(index, ..)| (*index, false))
}

/// Whether an event should produce a tray notification under `settings`.
pub fn should_notify(settings: &NotificationSettings, event: &Value) -> bool {
    if !settings.enabled {
        return false;
    }
    if event.get("tunnel").and_then(Value::as_bool).unwrap_or(false) && !settings.tunnels {
        return false;
    }
    match event.get("kind").and_then(Value::as_str).unwrap_or("") {
        "adapter" => settings.adapters,
        "wifi" => settings.wifi,
        "defaultRoute" => settings.default_route,
        "trigger" => true,
        _ => false,
    }
}

/// Notification bodies, at most `MAX_NOTIFICATIONS`; more become a summary.
pub fn notification_bodies(language: &str, events: &[&Value]) -> Vec<String> {
    if events.len() > MAX_NOTIFICATIONS {
        return vec![if language == "zh-CN" {
            format!("{} 项网络变化，请在“监控”页查看。", events.len())
        } else {
            format!("{} network changes. Open Monitor for details.", events.len())
        }];
    }
    events.iter().map(|item| event_text(language, item)).collect()
}

/// Localized event text for notifications (the UI translates codes itself).
pub fn event_text(language: &str, event: &Value) -> String {
    let text = event.get("text").and_then(Value::as_str).unwrap_or_default().to_string();
    if language != "zh-CN" {
        return text;
    }
    let param = |name: &str| event.pointer(&format!("/params/{name}")).and_then(Value::as_str).unwrap_or_default().to_string();
    let owner = || {
        let owner = param("owner");
        if owner.is_empty() { String::new() } else { format!("（{owner}）") }
    };
    match event.get("code").and_then(Value::as_str).unwrap_or_default() {
        "event.adapterUp" => format!("{}{} 已连接。", param("alias"), owner()),
        "event.adapterDown" => format!("{}{} 已断开。", param("alias"), owner()),
        "event.wifiConnected" => format!("{} 已连接到 {}。", param("alias"), param("ssid")),
        "event.wifiDisconnected" => format!("{} 已断开 {}。", param("alias"), param("ssid")),
        "event.wifiSwitched" => format!("{} 已从 {} 切换到 {}。", param("alias"), param("from"), param("to")),
        "event.defaultRouteTunnel" => format!("默认流量现在经由 {}{}。", param("to"), owner()),
        "event.defaultRouteChanged" => format!("默认流量已从 {} 切换到 {}。", param("from"), param("to")),
        "event.defaultRouteLost" => format!("没有默认路由：{} 不再承载默认流量。", param("from")),
        "event.defaultRouteRestored" => format!("默认流量经由 {}{}。", param("to"), owner()),
        _ => text,
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

struct Series {
    index: u32,
    alias: String,
    owner: Option<String>,
    hardware: bool,
    up: bool,
    last: Option<(u64, u64, u64)>,
    samples: VecDeque<[u64; 3]>,
}

struct SignalSeries {
    alias: String,
    ssid: Option<String>,
    samples: VecDeque<(u64, Option<u32>, Option<String>)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Counter {
    pub index: u32,
    pub key: String,
    pub alias: String,
    pub owner: Option<String>,
    pub hardware: bool,
    pub up: bool,
    pub rx: u64,
    pub tx: u64,
}

#[derive(Default)]
struct MonitorState {
    traffic: HashMap<String, Series>,
    total: VecDeque<[u64; 3]>,
    signal: HashMap<String, SignalSeries>,
    events: Vec<Value>,
    observation: Option<Observation>,
    next_id: u64,
}

impl MonitorState {
    /// Record one counter reading. `mono_ms` is monotonic (for rates) and
    /// `wall_ms` is the displayed time.
    fn push_traffic(&mut self, mono_ms: u64, wall_ms: u64, counters: &[Counter]) {
        let (mut total_rx, mut total_tx, mut measured) = (0u64, 0u64, false);
        for counter in counters {
            let series = self.traffic.entry(counter.key.clone()).or_insert_with(|| Series {
                index: counter.index,
                alias: counter.alias.clone(),
                owner: counter.owner.clone(),
                hardware: counter.hardware,
                up: counter.up,
                last: None,
                samples: VecDeque::new(),
            });
            series.index = counter.index;
            series.alias = counter.alias.clone();
            series.owner = counter.owner.clone();
            series.up = counter.up;
            if let Some((last_ms, last_rx, last_tx)) = series.last {
                let elapsed = mono_ms.saturating_sub(last_ms);
                let (rx, tx) = (rate(last_rx, counter.rx, elapsed), rate(last_tx, counter.tx, elapsed));
                push_capped(&mut series.samples, [wall_ms, rx, tx], TRAFFIC_SAMPLES);
                if series.hardware && series.owner.is_none() {
                    total_rx = total_rx.saturating_add(rx);
                    total_tx = total_tx.saturating_add(tx);
                    measured = true;
                }
            }
            series.last = Some((mono_ms, counter.rx, counter.tx));
        }
        let present = counters.iter().map(|counter| counter.key.as_str()).collect::<Vec<_>>();
        self.traffic.retain(|key, _| present.contains(&key.as_str()));
        if measured {
            push_capped(&mut self.total, [wall_ms, total_rx, total_tx], TRAFFIC_SAMPLES);
        }
    }

    fn push_signal(&mut self, wall_ms: u64, wifi: &BTreeMap<String, (WifiState, Option<u32>)>) {
        for (key, (state, signal)) in wifi {
            let series = self.signal.entry(key.clone()).or_insert_with(|| SignalSeries {
                alias: state.alias.clone(),
                ssid: None,
                samples: VecDeque::new(),
            });
            series.alias = state.alias.clone();
            series.ssid = state.ssid.clone();
            push_capped(&mut series.samples, (wall_ms, *signal, state.ssid.clone()), SIGNAL_SAMPLES);
        }
    }

    fn append_events(&mut self, events: Vec<Value>) -> Vec<Value> {
        let now = Utc::now().to_rfc3339();
        let stamped = events
            .into_iter()
            .map(|mut item| {
                self.next_id += 1;
                item["id"] = json!(format!("{}-{}", Utc::now().timestamp_millis(), self.next_id));
                item["time"] = json!(now);
                item
            })
            .collect::<Vec<_>>();
        self.events.extend(stamped.iter().cloned());
        let excess = self.events.len().saturating_sub(MAX_EVENTS);
        self.events.drain(..excess);
        stamped
    }
}

pub struct Monitor {
    events_path: PathBuf,
    state: Mutex<MonitorState>,
    stop: Mutex<bool>,
    wake: Condvar,
}

impl Monitor {
    pub fn new(events_path: PathBuf) -> Arc<Self> {
        let events = fs::read_to_string(&events_path)
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<Value>>(&text).ok())
            .unwrap_or_default();
        let excess = events.len().saturating_sub(MAX_EVENTS);
        Arc::new(Self {
            events_path,
            state: Mutex::new(MonitorState { events: events[excess..].to_vec(), ..Default::default() }),
            stop: Mutex::new(false),
            wake: Condvar::new(),
        })
    }

    /// Start sampling. `on_events` receives newly recorded events.
    pub fn spawn<F>(self: &Arc<Self>, on_events: F) -> std::io::Result<()>
    where
        F: Fn(&[Value]) + Send + 'static,
    {
        let monitor = Arc::clone(self);
        thread::Builder::new().name("network-monitor".into()).spawn(move || {
            let started = Instant::now();
            let mut tick = 0u64;
            loop {
                let proxy = if tick % SLOW_EVERY == 0 {
                    let processes = coexistence::running_processes();
                    Some(coexistence::proxy_product(&coexistence::detect_software(processes.values().map(String::as_str))))
                } else {
                    None
                };
                let events = monitor.sample(started.elapsed().as_millis() as u64, proxy.as_deref());
                if !events.is_empty() {
                    on_events(&events);
                }
                tick += 1;
                let Ok(stop) = monitor.stop.lock() else { return };
                if *stop {
                    return;
                }
                let Ok((stop, _)) = monitor.wake.wait_timeout(stop, SAMPLE_INTERVAL) else { return };
                if *stop {
                    return;
                }
            }
        })?;
        Ok(())
    }

    pub fn stop(&self) {
        if let Ok(mut stop) = self.stop.lock() {
            *stop = true;
            self.wake.notify_all();
        }
    }

    /// One sampling pass. With `proxy` set (every slow tick) this also checks
    /// Wi-Fi and the default route and records events.
    fn sample(&self, mono_ms: u64, proxy: Option<&str>) -> Vec<Value> {
        let proxy_product = proxy.unwrap_or("Proxy TUN");
        let counters = read_counters(proxy_product, &self.known_owners());
        let wall_ms = Utc::now().timestamp_millis().max(0) as u64;
        let slow = proxy.map(|_| {
            let wifi = crate::wifi_windows::connection_samples().ok().map(|items| {
                items
                    .into_iter()
                    .map(|item| {
                        let state = WifiState { alias: item.alias, ssid: item.ssid, profile: item.profile };
                        (crate::network_insights::normalized_guid_key(&item.adapter_guid), (state, item.signal))
                    })
                    .collect::<BTreeMap<_, _>>()
            });
            (wifi, read_default_route(&counters))
        });
        let Ok(mut state) = self.state.lock() else { return vec![] };
        state.push_traffic(mono_ms, wall_ms, &counters);
        let Some((wifi, default_route)) = slow else { return vec![] };
        if let Some(wifi) = &wifi {
            state.push_signal(wall_ms, wifi);
        }
        let observation = Observation {
            adapters: counters
                .iter()
                .map(|counter| {
                    (counter.key.clone(), AdapterState { alias: counter.alias.clone(), owner: counter.owner.clone(), up: counter.up })
                })
                .collect(),
            wifi: wifi.map(|items| items.into_iter().map(|(key, (state, _))| (key, state)).collect()),
            default_route,
        };
        let events = match &state.observation {
            Some(previous) => diff_events(previous, &observation),
            None => vec![],
        };
        state.observation = Some(observation);
        if events.is_empty() {
            return vec![];
        }
        let stamped = state.append_events(events);
        self.save(&state.events);
        stamped
    }

    fn known_owners(&self) -> HashMap<String, Option<String>> {
        self.state
            .lock()
            .map(|state| state.traffic.iter().map(|(key, series)| (key.clone(), series.owner.clone())).collect())
            .unwrap_or_default()
    }

    /// Record events produced elsewhere in the app (network triggers).
    pub fn record(&self, events: Vec<Value>) -> Vec<Value> {
        let Ok(mut state) = self.state.lock() else { return vec![] };
        let stamped = state.append_events(events);
        self.save(&state.events);
        stamped
    }

    fn save(&self, events: &[Value]) {
        let temp = self.events_path.with_extension("tmp");
        if let Ok(text) = serde_json::to_string(events) {
            if fs::write(&temp, text).is_ok() {
                let _ = fs::rename(&temp, &self.events_path);
            }
        }
    }

    pub fn events(&self) -> Vec<Value> {
        self.state.lock().map(|state| state.events.iter().rev().cloned().collect()).unwrap_or_default()
    }

    pub fn clear_events(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.events.clear();
            self.save(&state.events);
        }
    }

    pub fn snapshot(&self) -> Value {
        let Ok(state) = self.state.lock() else { return json!({}) };
        let mut interfaces = state
            .traffic
            .iter()
            .filter(|(_, series)| {
                series.hardware || series.owner.is_some() || series.samples.iter().any(|[_, rx, tx]| rx + tx > 0)
            })
            .map(|(key, series)| {
                let last = series.samples.back();
                json!({
                    "key":key,
                    "interfaceIndex":series.index,
                    "alias":series.alias,
                    "owner":series.owner,
                    "hardware":series.hardware,
                    "up":series.up,
                    "rxBps":last.map(|sample| sample[1]).unwrap_or(0),
                    "txBps":last.map(|sample| sample[2]).unwrap_or(0),
                    "samples":series.samples
                })
            })
            .collect::<Vec<_>>();
        interfaces.sort_by_key(|item| {
            (
                !item["hardware"].as_bool().unwrap_or(false),
                !item["up"].as_bool().unwrap_or(false),
                item["alias"].as_str().unwrap_or_default().to_lowercase(),
            )
        });
        let signal = state
            .signal
            .iter()
            .map(|(key, series)| {
                json!({
                    "adapterGuid":key,
                    "alias":series.alias,
                    "ssid":series.ssid,
                    "signalPercent":series.samples.back().and_then(|sample| sample.1),
                    "samples":series.samples.iter().map(|(time, percent, ssid)| json!([time, percent, ssid])).collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        json!({
            "intervalMs":SAMPLE_INTERVAL.as_millis() as u64,
            "traffic":{"total":state.total,"interfaces":interfaces},
            "signal":signal,
            "eventCount":state.events.len()
        })
    }
}

// ---------------------------------------------------------------------------
// Windows reads
// ---------------------------------------------------------------------------

fn utf16(value: &[u16]) -> String {
    let end = value.iter().position(|c| *c == 0).unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end])
}

fn guid_key(guid: &GUID) -> String {
    Uuid::from_fields(guid.data1, guid.data2, guid.data3, &guid.data4).to_string()
}

/// Interface byte counters. Owners are resolved once per interface and then
/// kept, so the process list is not needed on every fast sample.
fn read_counters(proxy_product: &str, known: &HashMap<String, Option<String>>) -> Vec<Counter> {
    let mut table: *mut MIB_IF_TABLE2 = ptr::null_mut();
    if unsafe { GetIfTable2(&mut table) } != 0 || table.is_null() {
        return vec![];
    }
    let count = unsafe { (*table).NumEntries as usize }.min(4096);
    let rows = unsafe { std::slice::from_raw_parts((*table).Table.as_ptr(), count) };
    let mut result: Vec<Counter> = vec![];
    for row in rows {
        let flags = row.InterfaceAndOperStatusFlags._bitfield;
        let alias = utf16(&row.Alias);
        let description = utf16(&row.Description);
        if !monitored(&alias, &description, row.Type, flags & 0b10 != 0) {
            continue;
        }
        let key = guid_key(&row.InterfaceGuid);
        if result.iter().any(|item| item.key == key) {
            continue;
        }
        let owner = match known.get(&key) {
            Some(owner) if owner.as_deref() != Some("Proxy TUN") => owner.clone(),
            _ => coexistence::resolve_owner(&alias, &description, row.Type as i64, proxy_product),
        };
        result.push(Counter {
            index: row.InterfaceIndex,
            key,
            alias,
            owner,
            hardware: flags & 0b1 != 0,
            up: row.OperStatus == 1,
            rx: row.InOctets,
            tx: row.OutOctets,
        });
    }
    unsafe { FreeMibTable(table.cast()) };
    result
}

fn read_default_route(counters: &[Counter]) -> Option<DefaultRoute> {
    let mut table: *mut MIB_IPFORWARD_TABLE2 = ptr::null_mut();
    if unsafe { GetIpForwardTable2(AF_INET, &mut table) } != 0 || table.is_null() {
        return None;
    }
    let count = unsafe { (*table).NumEntries as usize }.min(65_536);
    let rows = unsafe { std::slice::from_raw_parts((*table).Table.as_ptr(), count) };
    let routes = rows
        .iter()
        .filter(|row| row.DestinationPrefix.PrefixLength <= 1)
        .map(|row| {
            let bytes = unsafe { row.DestinationPrefix.Prefix.Ipv4.sin_addr.S_un.S_un_b };
            (
                row.InterfaceIndex,
                Ipv4Addr::new(bytes.s_b1, bytes.s_b2, bytes.s_b3, bytes.s_b4),
                row.DestinationPrefix.PrefixLength,
                row.Metric,
            )
        })
        .collect::<Vec<_>>();
    unsafe { FreeMibTable(table.cast()) };
    let by_index = |index: u32| counters.iter().find(|counter| counter.index == index);
    let (index, split) = pick_default(
        &routes,
        |index| {
            crate::native_metric(index, AF_INET)
                .get("interfaceMetric")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32
        },
        |index| by_index(index).map(|counter| counter.up).unwrap_or(false),
    )?;
    let counter = by_index(index)?;
    Some(DefaultRoute { alias: counter.alias.clone(), owner: counter.owner.clone(), split })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(alias: &str, owner: Option<&str>, up: bool) -> AdapterState {
        AdapterState { alias: alias.into(), owner: owner.map(Into::into), up }
    }

    fn wifi(alias: &str, ssid: Option<&str>) -> WifiState {
        WifiState { alias: alias.into(), ssid: ssid.map(Into::into), profile: ssid.map(Into::into) }
    }

    #[test]
    fn filters_and_rates() {
        assert!(monitored("WLAN", "Intel(R) Wi-Fi 6 AX201 160MHz", 71, false));
        assert!(monitored("et_6_0qmn", "et_6_0qmn Tunnel", 53, false));
        assert!(!monitored("WLAN", "Intel(R) Wi-Fi 6 AX201-WFP Native MAC Layer LightWeight Filter-0000", 71, true));
        assert!(!monitored("本地连接* 2", "WAN Miniport (SSTP)", 131, false));
        assert!(!monitored("Loopback Pseudo-Interface 1", "Software Loopback Interface 1", 24, false));
        assert!(!monitored("本地连接* 1", "Microsoft Wi-Fi Direct Virtual Adapter", 71, false));
        assert_eq!(rate(1_000, 3_000, 2_000), 1_000);
        assert_eq!(rate(5_000, 1_000, 2_000), 0, "counter reset");
        assert_eq!(rate(1_000, 3_000, 0), 0);
    }

    #[test]
    fn total_traffic_excludes_tunnels_and_virtual_adapters() {
        let counter = |key: &str, owner: Option<&str>, hardware: bool, rx: u64| Counter {
            index: 1,
            key: key.into(),
            alias: key.into(),
            owner: owner.map(Into::into),
            hardware,
            up: true,
            rx,
            tx: rx / 10,
        };
        let mut state = MonitorState::default();
        state.push_traffic(0, 100, &[counter("wlan", None, true, 0), counter("et", Some("EasyTier"), false, 0), counter("veth", None, false, 0)]);
        assert!(state.total.is_empty(), "the first reading only sets the baseline");
        state.push_traffic(2_000, 102, &[counter("wlan", None, true, 4_000), counter("et", Some("EasyTier"), false, 2_000), counter("veth", None, false, 1_000)]);
        assert_eq!(state.total.back(), Some(&[102, 2_000, 200]));
        assert_eq!(state.traffic["et"].samples.back(), Some(&[102, 1_000, 100]));
        state.push_traffic(4_000, 104, &[counter("wlan", None, true, 6_000)]);
        assert!(!state.traffic.contains_key("et"), "removed adapters are dropped");
        for index in 0..200 {
            state.push_traffic(6_000 + index * 2_000, 106 + index, &[counter("wlan", None, true, 6_000 + index * 10)]);
        }
        assert_eq!(state.total.len(), TRAFFIC_SAMPLES);
    }

    #[test]
    fn events_for_adapters_wifi_and_tunnel_default_route() {
        let mut before = Observation::default();
        before.adapters.insert("eth".into(), adapter("以太网", None, true));
        before.adapters.insert("tun".into(), adapter("singbox_tun", Some("v2rayN"), false));
        before.adapters.insert("wlan".into(), adapter("WLAN", None, true));
        before.wifi = Some(BTreeMap::from([("wlan".to_string(), wifi("WLAN", Some("Home")))]));
        before.default_route = Some(DefaultRoute { alias: "以太网".into(), owner: None, split: false });
        let mut after = before.clone();
        after.adapters.insert("tun".into(), adapter("singbox_tun", Some("v2rayN"), true));
        after.wifi = Some(BTreeMap::from([("wlan".to_string(), wifi("WLAN", Some("Office")))]));
        after.default_route = Some(DefaultRoute { alias: "singbox_tun".into(), owner: Some("v2rayN".into()), split: true });
        let events = diff_events(&before, &after);
        let codes = events.iter().map(|item| item["code"].as_str().unwrap()).collect::<Vec<_>>();
        assert_eq!(codes, vec!["event.adapterUp", "event.wifiSwitched", "event.defaultRouteTunnel"]);
        assert_eq!(events[0]["tunnel"], true);
        assert_eq!(events[2]["tunnel"], false, "a captured default route is one ordinary event");
        assert!(diff_events(&after, &after).is_empty());
        let mut lost = after.clone();
        lost.wifi = None;
        assert!(diff_events(&after, &lost).iter().all(|item| item["kind"] != "wifi"), "unavailable Wi-Fi details are not a disconnect");
        lost.default_route = None;
        assert_eq!(diff_events(&after, &lost).last().unwrap()["code"], "event.defaultRouteLost");
    }

    #[test]
    fn default_route_prefers_split_tunnel_routes() {
        let zero = Ipv4Addr::new(0, 0, 0, 0);
        let half = Ipv4Addr::new(128, 0, 0, 0);
        let routes = vec![(23, zero, 0, 0), (12, zero, 0, 0), (40, zero, 1, 0), (40, half, 1, 0)];
        let metric = |index: u32| if index == 23 { 25 } else { 35 };
        assert_eq!(pick_default(&routes, metric, |_| true), Some((40, true)));
        assert_eq!(pick_default(&routes, metric, |index| index != 40), Some((23, false)));
        assert_eq!(pick_default(&routes, metric, |index| index == 12), Some((12, false)));
        assert_eq!(pick_default(&routes, metric, |_| false), None);
    }

    #[test]
    fn notifications_are_opt_in_and_skip_tunnels() {
        let mut settings = NotificationSettings::default();
        let adapter_event = json!({"kind":"adapter","tunnel":false});
        let tunnel_event = json!({"kind":"adapter","tunnel":true});
        assert!(!should_notify(&settings, &adapter_event));
        settings.enabled = true;
        assert!(should_notify(&settings, &adapter_event));
        assert!(!should_notify(&settings, &tunnel_event));
        settings.tunnels = true;
        assert!(should_notify(&settings, &tunnel_event));
        settings.wifi = false;
        assert!(!should_notify(&settings, &json!({"kind":"wifi"})));
        let events = (0..5).map(|_| json!({"text":"x"})).collect::<Vec<_>>();
        let refs = events.iter().collect::<Vec<_>>();
        assert_eq!(notification_bodies("en", &refs), vec!["5 network changes. Open Monitor for details."]);
        let event = json!({"code":"event.wifiSwitched","params":{"alias":"WLAN","from":"A","to":"B"},"text":"WLAN switched from A to B."});
        assert_eq!(event_text("zh-CN", &event), "WLAN 已从 A 切换到 B。");
        assert_eq!(event_text("en", &event), "WLAN switched from A to B.");
    }

    #[test]
    fn events_are_capped_and_persisted() {
        let path = std::env::temp_dir().join(format!("wnm-events-{}.json", Uuid::new_v4()));
        let monitor = Monitor::new(path.clone());
        let stamped = {
            let mut state = monitor.state.lock().unwrap();
            let stamped = state.append_events((0..MAX_EVENTS + 20).map(|index| json!({"kind":"adapter","text":index.to_string()})).collect());
            monitor.save(&state.events);
            stamped
        };
        assert_eq!(stamped.len(), MAX_EVENTS + 20);
        let events = monitor.events();
        assert_eq!(events.len(), MAX_EVENTS);
        assert_eq!(events[0]["text"], (MAX_EVENTS + 19).to_string(), "newest first");
        assert_eq!(Monitor::new(path.clone()).events().len(), MAX_EVENTS);
        monitor.clear_events();
        assert!(Monitor::new(path.clone()).events().is_empty());
        let _ = fs::remove_file(path);
    }

    #[test]
    #[ignore = "manual, read-only check of local counters, routes and Wi-Fi"]
    fn read_only_local_monitor_reads() {
        let counters = read_counters("Proxy TUN", &HashMap::new());
        assert!(!counters.is_empty());
        for counter in &counters {
            println!("{} #{} owner={:?} hw={} up={} rx={} tx={}", counter.alias, counter.index, counter.owner, counter.hardware, counter.up, counter.rx, counter.tx);
        }
        println!("default: {:?}", read_default_route(&counters));
        if let Ok(wifi) = crate::wifi_windows::connection_samples() {
            for item in wifi {
                println!("wifi {} ssid={:?} signal={:?}", item.alias, item.ssid, item.signal);
            }
        }
    }
}
