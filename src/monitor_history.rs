//! Saved monitor history: one JSON-lines file per local day with one line per
//! minute (traffic per interface, latency to the local gateway and DNS
//! servers, Wi-Fi signal and network event counts). Files live in
//! `monitor-data` next to the program by default, fall back to the app data
//! folder when that is not writable, and are pruned after the retention
//! period. Only files named `YYYY-MM-DD.jsonl` in that folder are ever
//! deleted. Recording is read-only; nothing here changes the network.

use crate::monitor::{rate, Counter};
use chrono::{Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub const DATA_DIR: &str = "monitor-data";
pub const MAX_POINTS: i64 = 600;
const BUCKETS: [i64; 7] = [60, 300, 900, 1800, 3600, 7200, 14400];

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct MonitorSettings {
    pub history_enabled: bool,
    pub retention_days: u32,
    pub latency_enabled: bool,
    pub latency_interval_seconds: u32,
    /// Also ping DNS servers outside the local network (off by default).
    pub latency_public_dns: bool,
    /// Custom parent folder for `monitor-data`. Set only by the folder
    /// picker command; values sent by the UI or an import are ignored.
    pub folder: Option<String>,
}
impl Default for MonitorSettings {
    fn default() -> Self {
        Self {
            history_enabled: true,
            retention_days: 30,
            latency_enabled: true,
            latency_interval_seconds: 10,
            latency_public_dns: false,
            folder: None,
        }
    }
}

pub fn validate_settings(settings: &MonitorSettings) -> Result<(), String> {
    if !(1..=365).contains(&settings.retention_days) {
        return Err("History can be kept from 1 to 365 days.".into());
    }
    if ![10, 30, 60].contains(&settings.latency_interval_seconds) {
        return Err("The latency interval must be 10, 30 or 60 seconds.".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Storage location
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Storage {
    pub path: PathBuf,
    /// "custom", "install" or "appData".
    pub source: &'static str,
    pub fallback_reason: Option<&'static str>,
}

/// Where history is written: a custom folder, else next to the program,
/// else the app data folder. The program folder is skipped under Program
/// Files or Windows, where only elevated sessions could write and history
/// would split between two places.
pub fn resolve_storage(
    exe_dir: Option<&Path>,
    config_dir: &Path,
    custom: Option<&Path>,
    protected: impl Fn(&Path) -> bool,
    probe: impl Fn(&Path) -> bool,
) -> Storage {
    let mut reason = None;
    if let Some(custom) = custom {
        let path = custom.join(DATA_DIR);
        if probe(&path) {
            return Storage { path, source: "custom", fallback_reason: None };
        }
        reason = Some("customUnwritable");
    }
    if let Some(exe_dir) = exe_dir {
        let path = exe_dir.join(DATA_DIR);
        if protected(exe_dir) {
            reason = reason.or(Some("installProtected"));
        } else if probe(&path) {
            return Storage { path, source: "install", fallback_reason: reason };
        } else {
            reason = reason.or(Some("installUnwritable"));
        }
    }
    Storage { path: config_dir.join(DATA_DIR), source: "appData", fallback_reason: reason }
}

/// Folders only administrators can write to.
pub fn is_protected_dir(path: &Path) -> bool {
    let text = path.to_string_lossy().to_lowercase();
    ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "SystemRoot", "windir"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .filter(|root| !root.is_empty())
        .any(|root| {
            let root = root.trim_end_matches('\\').to_lowercase();
            text == root || text.starts_with(&format!("{root}\\"))
        })
}

/// Create the folder and check that a file can be written and removed.
pub fn probe_writable(dir: &Path) -> bool {
    let test = dir.join(format!(".wnm-write-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(dir).is_ok() && fs::write(&test, b"ok").is_ok() && fs::remove_file(&test).is_ok()
}

pub fn day_file_name(day: NaiveDate) -> String {
    format!("{}.jsonl", day.format("%Y-%m-%d"))
}

pub fn parse_day_file(name: &str) -> Option<NaiveDate> {
    let stem = name.strip_suffix(".jsonl")?;
    if stem.len() != 10 || !stem.chars().enumerate().all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() }) {
        return None;
    }
    NaiveDate::parse_from_str(stem, "%Y-%m-%d").ok()
}

/// History files older than `keep_days` (today counts as the first day).
pub fn prune_candidates(names: &[String], today: NaiveDate, keep_days: u32) -> Vec<String> {
    let oldest = today - chrono::Duration::days(i64::from(keep_days.max(1)) - 1);
    names.iter().filter(|name| parse_day_file(name).is_some_and(|day| day < oldest)).cloned().collect()
}

fn local_day(t: i64) -> NaiveDate {
    Local.timestamp_opt(t, 0).single().map(|time| time.date_naive()).unwrap_or_else(|| Utc::now().date_naive())
}

fn day_files(dir: &Path) -> Vec<(NaiveDate, PathBuf, u64)> {
    let mut files = fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let day = parse_day_file(&name)?;
                    let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
                    Some((day, entry.path(), size))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    files.sort_by_key(|(day, ..)| *day);
    files
}

pub fn storage_info(storage: &Storage, default_path: &Path, retention_days: u32) -> Value {
    let files = day_files(&storage.path);
    json!({
        "path":storage.path.display().to_string(),
        "source":storage.source,
        "fallbackReason":storage.fallback_reason,
        "defaultPath":default_path.display().to_string(),
        "sizeBytes":files.iter().map(|(.., size)| size).sum::<u64>(),
        "days":files.len(),
        "oldestDay":files.first().map(|(day, ..)| day.to_string()),
        "retentionDays":retention_days,
    })
}

/// Delete history files older than the retention period. Returns how many.
pub fn prune(dir: &Path, today: NaiveDate, keep_days: u32) -> usize {
    let names = day_files(dir).into_iter().map(|(day, ..)| day_file_name(day)).collect::<Vec<_>>();
    prune_candidates(&names, today, keep_days).iter().filter(|name| fs::remove_file(dir.join(name)).is_ok()).count()
}

/// Delete every history file in the folder (Clear history).
pub fn clear(dir: &Path) -> usize {
    day_files(dir).iter().filter(|(_, path, _)| fs::remove_file(path).is_ok()).count()
}

/// Copy day files that do not exist yet in `to` (changing the folder).
pub fn copy_days(from: &Path, to: &Path) -> usize {
    day_files(from)
        .iter()
        .filter(|(day, path, _)| {
            let target = to.join(day_file_name(*day));
            !target.exists() && fs::copy(path, target).is_ok()
        })
        .count()
}

// ---------------------------------------------------------------------------
// One minute of history
// ---------------------------------------------------------------------------

/// `[key, alias, owner, hardware, rxBytes, txBytes, peakRxBps, peakTxBps, upSamples, samples]`
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct IfMin(pub String, pub String, pub Option<String>, pub bool, pub u64, pub u64, pub u64, pub u64, pub u32, pub u32);

/// `[kind, address, alias, round-trip times (null = no reply)]`
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct LatMin(pub String, pub String, pub String, pub Vec<Option<u32>>);

/// `[adapterGuid, alias, averageSignal, minimumSignal, ssid]`
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct WifiMin(pub String, pub String, pub u32, pub u32, pub Option<String>);

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct EventCounts {
    pub down: u32,
    pub up: u32,
    pub wifi_disc: u32,
    pub wifi_switch: u32,
    pub route: u32,
    pub tunnel: u32,
}
impl EventCounts {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
    fn add(&mut self, other: &Self) {
        self.down += other.down;
        self.up += other.up;
        self.wifi_disc += other.wifi_disc;
        self.wifi_switch += other.wifi_switch;
        self.route += other.route;
        self.tunnel += other.tunnel;
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(default)]
pub struct Minute {
    /// Minute start, UTC epoch seconds.
    pub t: i64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub i: Vec<IfMin>,
    /// Total of physical adapters: `[rxBytes, txBytes, peakRxBps, peakTxBps]`.
    pub tot: [u64; 4],
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lat: Vec<LatMin>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub w: Vec<WifiMin>,
    #[serde(skip_serializing_if = "EventCounts::is_empty")]
    pub e: EventCounts,
    /// Alias of the interface carrying default traffic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dr: Option<String>,
}

#[derive(Default)]
struct WifiAcc {
    alias: String,
    sum: u64,
    count: u32,
    min: u32,
    ssid: Option<String>,
}

/// Builds minutes from 2-second samples. Every `record_*` call returns the
/// previous minute once a new one has started.
#[derive(Default)]
pub struct Accumulator {
    current: Option<Minute>,
    wifi: BTreeMap<String, WifiAcc>,
    last: HashMap<String, (u64, u64, u64)>,
}

impl Accumulator {
    fn roll(&mut self, wall_ms: u64) -> Option<Minute> {
        let start = (wall_ms / 1000 / 60 * 60) as i64;
        match &self.current {
            Some(minute) if minute.t == start => None,
            Some(minute) if minute.t > start => None, // clock moved back: keep filling the current minute
            _ => {
                let done = self.take();
                self.current = Some(Minute { t: start, ..Default::default() });
                done
            }
        }
    }

    fn minute(&mut self) -> &mut Minute {
        self.current.get_or_insert_with(Minute::default)
    }

    /// Bytes from counter differences (a reset counter counts 0) and the
    /// highest 2-second rate in the minute.
    pub fn record_traffic(&mut self, mono_ms: u64, wall_ms: u64, counters: &[Counter]) -> Option<Minute> {
        let done = self.roll(wall_ms);
        let mut deltas = vec![];
        for counter in counters {
            if let Some((last_ms, last_rx, last_tx)) = self.last.get(&counter.key).copied() {
                let elapsed = mono_ms.saturating_sub(last_ms);
                let rx = counter.rx.checked_sub(last_rx).unwrap_or(0);
                let tx = counter.tx.checked_sub(last_tx).unwrap_or(0);
                deltas.push((counter, rx, tx, rate(last_rx, counter.rx, elapsed), rate(last_tx, counter.tx, elapsed)));
            }
            self.last.insert(counter.key.clone(), (mono_ms, counter.rx, counter.tx));
        }
        self.last.retain(|key, _| counters.iter().any(|counter| &counter.key == key));
        let minute = self.current.as_mut().expect("rolled");
        let (mut rx_rate, mut tx_rate) = (0u64, 0u64);
        for (counter, rx, tx, rx_bps, tx_bps) in deltas {
            if counter.hardware && counter.owner.is_none() {
                minute.tot[0] += rx;
                minute.tot[1] += tx;
                rx_rate += rx_bps;
                tx_rate += tx_bps;
            }
            let keep = counter.hardware || counter.owner.is_some() || rx + tx > 0;
            let existing = minute.i.iter().position(|item| item.0 == counter.key);
            if !keep && existing.is_none() {
                continue;
            }
            let item = match existing {
                Some(index) => &mut minute.i[index],
                None => {
                    minute.i.push(IfMin(counter.key.clone(), counter.alias.clone(), counter.owner.clone(), counter.hardware, 0, 0, 0, 0, 0, 0));
                    minute.i.last_mut().expect("pushed")
                }
            };
            item.1 = counter.alias.clone();
            item.2 = counter.owner.clone();
            item.4 += rx;
            item.5 += tx;
            item.6 = item.6.max(rx_bps);
            item.7 = item.7.max(tx_bps);
            item.8 += u32::from(counter.up);
            item.9 += 1;
        }
        minute.tot[2] = minute.tot[2].max(rx_rate);
        minute.tot[3] = minute.tot[3].max(tx_rate);
        done
    }

    pub fn record_latency(&mut self, wall_ms: u64, kind: &str, address: &str, alias: &str, rtt: Option<u32>) -> Option<Minute> {
        let done = self.roll(wall_ms);
        let minute = self.minute();
        match minute.lat.iter_mut().find(|item| item.0 == kind && item.1 == address) {
            Some(item) => item.3.push(rtt),
            None => minute.lat.push(LatMin(kind.into(), address.into(), alias.into(), vec![rtt])),
        }
        done
    }

    pub fn record_wifi(&mut self, wall_ms: u64, guid: &str, alias: &str, signal: Option<u32>, ssid: Option<String>) -> Option<Minute> {
        let done = self.roll(wall_ms);
        let entry = self.wifi.entry(guid.to_string()).or_default();
        entry.alias = alias.to_string();
        if let Some(signal) = signal {
            entry.min = if entry.count == 0 { signal } else { entry.min.min(signal) };
            entry.sum += u64::from(signal);
            entry.count += 1;
            entry.ssid = ssid;
        }
        done
    }

    /// Count network events (from `monitor::diff_events`).
    pub fn record_events(&mut self, wall_ms: u64, events: &[Value]) -> Option<Minute> {
        let done = self.roll(wall_ms);
        let counts = &mut self.minute().e;
        for event in events {
            let tunnel = event.get("tunnel").and_then(Value::as_bool).unwrap_or(false);
            match event.get("code").and_then(Value::as_str).unwrap_or_default() {
                _ if tunnel => counts.tunnel += 1,
                "event.adapterDown" => counts.down += 1,
                "event.adapterUp" => counts.up += 1,
                "event.wifiDisconnected" => counts.wifi_disc += 1,
                "event.wifiSwitched" => counts.wifi_switch += 1,
                code if code.starts_with("event.defaultRoute") => counts.route += 1,
                _ => {}
            }
        }
        done
    }

    pub fn record_default_route(&mut self, wall_ms: u64, alias: Option<String>) -> Option<Minute> {
        let done = self.roll(wall_ms);
        self.minute().dr = alias;
        done
    }

    /// The minute in progress (written when the app exits).
    pub fn take(&mut self) -> Option<Minute> {
        let mut minute = self.current.take()?;
        minute.w = std::mem::take(&mut self.wifi)
            .into_iter()
            .filter(|(_, item)| item.count > 0)
            .map(|(guid, item)| WifiMin(guid, item.alias, (item.sum / u64::from(item.count)) as u32, item.min, item.ssid))
            .collect();
        let empty = minute.i.is_empty() && minute.lat.is_empty() && minute.w.is_empty() && minute.e.is_empty() && minute.tot == [0; 4];
        (!empty).then_some(minute)
    }
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

pub struct History {
    pub storage: Storage,
    pub settings: MonitorSettings,
    config_dir: PathBuf,
    exe_dir: Option<PathBuf>,
    pub acc: Accumulator,
    pruned_on: Option<NaiveDate>,
}

impl History {
    pub fn new(config_dir: PathBuf, exe_dir: Option<PathBuf>, settings: MonitorSettings) -> Self {
        let mut history = Self {
            storage: Storage { path: config_dir.join(DATA_DIR), source: "appData", fallback_reason: None },
            settings: MonitorSettings::default(),
            config_dir,
            exe_dir,
            acc: Accumulator::default(),
            pruned_on: None,
        };
        history.apply_settings(settings);
        history
    }

    pub fn default_path(&self) -> PathBuf {
        resolve_storage(self.exe_dir.as_deref(), &self.config_dir, None, is_protected_dir, |path| path.exists() || probe_writable(path)).path
    }

    /// Re-resolve the folder and prune with the (possibly shorter) retention.
    pub fn apply_settings(&mut self, settings: MonitorSettings) {
        let custom = settings.folder.as_ref().map(PathBuf::from);
        self.storage = resolve_storage(self.exe_dir.as_deref(), &self.config_dir, custom.as_deref(), is_protected_dir, probe_writable);
        self.settings = settings;
        self.pruned_on = None;
        self.prune_if_due();
    }

    fn prune_if_due(&mut self) {
        let today = Local::now().date_naive();
        if self.pruned_on != Some(today) {
            prune(&self.storage.path, today, self.settings.retention_days);
            self.pruned_on = Some(today);
        }
    }

    fn append(path: &Path, minute: &Minute) -> std::io::Result<()> {
        fs::create_dir_all(path)?;
        let readme = path.join("README.txt");
        if !readme.exists() {
            let _ = fs::write(
                &readme,
                "Windows Network Manager monitor history.\r\nOne file per day (YYYY-MM-DD.jsonl), one line per minute. Files older than the retention period set in Settings are deleted automatically. You can delete this folder at any time.\r\n",
            );
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path.join(day_file_name(local_day(minute.t))))?;
        let mut line = serde_json::to_string(minute)?;
        line.push('\n');
        file.write_all(line.as_bytes())
    }

    /// Append a finished minute. A failing folder falls back to app data.
    pub fn write(&mut self, minute: Option<Minute>) {
        let Some(minute) = minute else { return };
        if !self.settings.history_enabled {
            return;
        }
        self.prune_if_due();
        if Self::append(&self.storage.path, &minute).is_err() && self.storage.source != "appData" {
            self.storage = Storage { path: self.config_dir.join(DATA_DIR), source: "appData", fallback_reason: Some("writeFailed") };
            let _ = Self::append(&self.storage.path, &minute);
        }
    }
}

// ---------------------------------------------------------------------------
// Queries (pure over minutes; unit tested)
// ---------------------------------------------------------------------------

/// The smallest bucket that keeps a range at or under `MAX_POINTS` points.
pub fn choose_bucket(range_seconds: i64) -> i64 {
    BUCKETS.iter().copied().find(|bucket| range_seconds / bucket <= MAX_POINTS).unwrap_or(BUCKETS[BUCKETS.len() - 1])
}

pub fn range_seconds(range: &str) -> Option<i64> {
    match range {
        "1h" => Some(3600),
        "24h" => Some(86_400),
        "7d" => Some(7 * 86_400),
        "30d" => Some(30 * 86_400),
        _ => None,
    }
}

/// Read the minutes in `[from, to)` from the day files that can contain them.
/// Unreadable lines (for example a cut-off last line) are skipped.
pub fn read_minutes(dir: &Path, from: i64, to: i64) -> Vec<Minute> {
    let first = local_day(from) - chrono::Duration::days(1);
    let last = local_day(to) + chrono::Duration::days(1);
    let mut minutes = day_files(dir)
        .into_iter()
        .filter(|(day, ..)| *day >= first && *day <= last)
        .flat_map(|(_, path, _)| fs::read_to_string(path).unwrap_or_default().lines().filter_map(|line| serde_json::from_str::<Minute>(line).ok()).collect::<Vec<_>>())
        .filter(|minute| minute.t >= from && minute.t < to)
        .collect::<Vec<_>>();
    minutes.sort_by_key(|minute| minute.t);
    minutes
}

#[derive(Default, Clone)]
struct Traffic {
    rx: u64,
    tx: u64,
    peak_rx: u64,
    peak_tx: u64,
    minutes: u64,
    up: u64,
    samples: u64,
}
impl Traffic {
    fn add(&mut self, rx: u64, tx: u64, peak_rx: u64, peak_tx: u64) {
        self.rx += rx;
        self.tx += tx;
        self.peak_rx = self.peak_rx.max(peak_rx);
        self.peak_tx = self.peak_tx.max(peak_tx);
        self.minutes += 1;
    }
    fn stats(&self) -> Value {
        let seconds = (self.minutes * 60).max(1);
        json!({
            "rxBytes":self.rx,"txBytes":self.tx,
            "rxAvg":self.rx / seconds,"txAvg":self.tx / seconds,
            "rxPeak":self.peak_rx,"txPeak":self.peak_tx,
            "uptimePct":(self.samples > 0).then(|| self.up * 100 / self.samples),
        })
    }
    fn point(&self, t_ms: i64) -> Value {
        if self.minutes == 0 {
            return json!([t_ms, null, null, null, null]);
        }
        let seconds = self.minutes * 60;
        json!([t_ms, self.rx / seconds, self.tx / seconds, self.peak_rx, self.peak_tx])
    }
}

fn percentile(sorted: &[u32], share: f64) -> Option<u32> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((sorted.len() as f64) * share).ceil() as usize;
    sorted.get(rank.clamp(1, sorted.len()) - 1).copied()
}

fn latency_stats(rtts: &[Option<u32>]) -> Value {
    let mut received = rtts.iter().flatten().copied().collect::<Vec<_>>();
    received.sort_unstable();
    let sent = rtts.len();
    json!({
        "sent":sent,
        "received":received.len(),
        "lossPct":if sent == 0 { 0 } else { (sent - received.len()) * 100 / sent },
        "min":received.first(),
        "avg":(!received.is_empty()).then(|| received.iter().map(|v| u64::from(*v)).sum::<u64>() / received.len() as u64),
        "p95":percentile(&received, 0.95),
        "max":received.last(),
    })
}

/// Downsampled series and statistics for `[from, to)`. Per-interface points
/// are returned only for `selected` keys; statistics for every interface.
pub fn summarize(minutes: &[Minute], from: i64, to: i64, selected: &[String]) -> Value {
    let bucket = choose_bucket(to - from);
    let start = from - from.rem_euclid(bucket);
    let slots = ((to - start + bucket - 1) / bucket).max(1) as usize;
    let slot = |t: i64| (((t - start) / bucket).max(0) as usize).min(slots - 1);

    let mut total = Traffic::default();
    let mut total_slots = vec![Traffic::default(); slots];
    let mut interfaces: BTreeMap<String, (IfMin, Traffic, Vec<Traffic>)> = BTreeMap::new();
    let mut latency: BTreeMap<(String, String), (String, Vec<Option<u32>>, Vec<Vec<Option<u32>>>)> = BTreeMap::new();
    let mut wifi: BTreeMap<String, (String, Vec<String>, Vec<(u64, u32, u32)>)> = BTreeMap::new();
    let mut events = EventCounts::default();
    let mut daily: BTreeMap<NaiveDate, (u64, u64, EventCounts)> = BTreeMap::new();

    for minute in minutes {
        let index = slot(minute.t);
        total.add(minute.tot[0], minute.tot[1], minute.tot[2], minute.tot[3]);
        total_slots[index].add(minute.tot[0], minute.tot[1], minute.tot[2], minute.tot[3]);
        for item in &minute.i {
            let entry = interfaces.entry(item.0.clone()).or_insert_with(|| (item.clone(), Traffic::default(), vec![]));
            entry.0 = item.clone();
            entry.1.add(item.4, item.5, item.6, item.7);
            entry.1.up += u64::from(item.8);
            entry.1.samples += u64::from(item.9);
            if selected.contains(&item.0) {
                if entry.2.is_empty() {
                    entry.2 = vec![Traffic::default(); slots];
                }
                entry.2[index].add(item.4, item.5, item.6, item.7);
            }
        }
        for item in &minute.lat {
            let entry = latency.entry((item.0.clone(), item.1.clone())).or_insert_with(|| (item.2.clone(), vec![], vec![vec![]; slots]));
            entry.0 = item.2.clone();
            entry.1.extend(item.3.iter().copied());
            entry.2[index].extend(item.3.iter().copied());
        }
        for item in &minute.w {
            let entry = wifi.entry(item.0.clone()).or_insert_with(|| (item.1.clone(), vec![], vec![(0, 0, u32::MAX); slots]));
            entry.0 = item.1.clone();
            if let Some(ssid) = item.4.as_ref().filter(|ssid| !entry.1.contains(ssid)) {
                entry.1.push(ssid.clone());
            }
            let cell = &mut entry.2[index];
            cell.0 += u64::from(item.2);
            cell.1 += 1;
            cell.2 = cell.2.min(item.3);
        }
        events.add(&minute.e);
        let day = daily.entry(local_day(minute.t)).or_default();
        day.0 += minute.tot[0];
        day.1 += minute.tot[1];
        day.2.add(&minute.e);
    }

    let time = |index: usize| (start + index as i64 * bucket) * 1000;
    let available = interfaces
        .values()
        .map(|(item, ..)| json!({"key":item.0,"alias":item.1,"owner":item.2,"hardware":item.3}))
        .collect::<Vec<_>>();
    let interface_stats = interfaces
        .values()
        .map(|(item, traffic, points)| {
            let mut value = traffic.stats();
            value["key"] = json!(item.0);
            value["alias"] = json!(item.1);
            value["owner"] = json!(item.2);
            value["hardware"] = json!(item.3);
            value["points"] = json!(points.iter().enumerate().map(|(index, cell)| cell.point(time(index))).collect::<Vec<_>>());
            value
        })
        .collect::<Vec<_>>();
    let mut total_value = total.stats();
    total_value["points"] = json!(total_slots.iter().enumerate().map(|(index, cell)| cell.point(time(index))).collect::<Vec<_>>());
    json!({
        "from":from * 1000,
        "to":to * 1000,
        "bucketSeconds":bucket,
        "minutes":minutes.len(),
        "available":available,
        "total":total_value,
        "interfaces":interface_stats,
        "latency":latency.into_iter().map(|((kind, address), (alias, all, cells))| json!({
            "kind":kind,"address":address,"alias":alias,
            "stats":latency_stats(&all),
            "points":cells.iter().enumerate().map(|(index, cell)| {
                let stats = latency_stats(cell);
                if cell.is_empty() { json!([time(index), null, null, null]) } else { json!([time(index), stats["avg"], stats["max"], stats["lossPct"]]) }
            }).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "wifi":wifi.into_iter().map(|(guid, (alias, ssids, cells))| json!({
            "adapterGuid":guid,"alias":alias,"ssids":ssids,
            "points":cells.iter().enumerate().map(|(index, (sum, count, min))| {
                if *count == 0 { json!([time(index), null, null]) } else { json!([time(index), sum / u64::from(*count), min]) }
            }).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "events":events,
        "daily":daily.into_iter().map(|(day, (rx, tx, counts))| json!({
            "day":day.to_string(),"rxBytes":rx,"txBytes":tx,
            "disconnects":counts.down,"reconnects":counts.up,"routeChanges":counts.route,
            "wifiChanges":counts.wifi_disc + counts.wifi_switch,"tunnelEvents":counts.tunnel,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counter(key: &str, hardware: bool, owner: Option<&str>, rx: u64, tx: u64) -> Counter {
        Counter { index: 1, key: key.into(), alias: key.into(), owner: owner.map(Into::into), hardware, up: true, rx, tx }
    }

    #[test]
    fn settings_are_validated() {
        assert!(validate_settings(&MonitorSettings::default()).is_ok());
        assert_eq!(MonitorSettings::default().retention_days, 30);
        assert!(validate_settings(&MonitorSettings { retention_days: 0, ..Default::default() }).is_err());
        assert!(validate_settings(&MonitorSettings { retention_days: 366, ..Default::default() }).is_err());
        assert!(validate_settings(&MonitorSettings { latency_interval_seconds: 5, ..Default::default() }).is_err());
        let old: MonitorSettings = serde_json::from_value(json!({"retentionDays":7})).unwrap();
        assert!(old.history_enabled && old.latency_enabled && old.retention_days == 7);
    }

    #[test]
    fn storage_prefers_custom_then_install_then_app_data() {
        let config = Path::new(r"C:\Users\me\AppData\Roaming\wnm");
        let exe = Path::new(r"C:\Users\me\AppData\Local\Windows Network Manager");
        let custom = Path::new(r"D:\Logs");
        let never = |_: &Path| false;
        let s = resolve_storage(Some(exe), config, Some(custom), never, |_| true);
        assert_eq!((s.source, s.path.clone()), ("custom", custom.join(DATA_DIR)));
        let s = resolve_storage(Some(exe), config, None, never, |_| true);
        assert_eq!((s.source, s.path.clone(), s.fallback_reason), ("install", exe.join(DATA_DIR), None));
        let s = resolve_storage(Some(exe), config, Some(custom), never, |path| !path.starts_with(custom));
        assert_eq!((s.source, s.fallback_reason), ("install", Some("customUnwritable")));
        let s = resolve_storage(Some(exe), config, None, never, |_| false);
        assert_eq!((s.source, s.path.clone(), s.fallback_reason), ("appData", config.join(DATA_DIR), Some("installUnwritable")));
        let s = resolve_storage(Some(exe), config, None, |_| true, |_| true);
        assert_eq!((s.source, s.fallback_reason), ("appData", Some("installProtected")));
    }

    #[test]
    fn only_strict_day_files_older_than_retention_are_pruned() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        let names = ["2026-11-03.jsonl", "2026-10-05.jsonl", "2026-10-04.jsonl", "2025-01-01.jsonl", "notes.jsonl", "2026-1-01.jsonl", "README.txt", "2026-10-01.jsonl.bak"]
            .map(String::from)
            .to_vec();
        let mut pruned = prune_candidates(&names, today, 30);
        pruned.sort();
        assert_eq!(pruned, vec!["2025-01-01.jsonl", "2026-10-04.jsonl"]);
        assert!(prune_candidates(&names, today, 365).contains(&"2025-01-01.jsonl".to_string()));
        assert_eq!(parse_day_file("2026-11-03.jsonl"), Some(today));
        assert_eq!(day_file_name(today), "2026-11-03.jsonl");
    }

    #[test]
    fn minutes_accumulate_bytes_peaks_and_counter_resets() {
        let mut acc = Accumulator::default();
        let base = 1_800_000_000_000u64 / 60_000 * 60_000;
        assert!(acc.record_traffic(0, base, &[counter("eth", true, None, 1000, 100)]).is_none());
        acc.record_traffic(2000, base + 2000, &[counter("eth", true, None, 5000, 300), counter("tun", false, Some("EasyTier"), 0, 0)]);
        // A counter reset counts zero bytes, not a huge negative jump.
        acc.record_traffic(4000, base + 4000, &[counter("eth", true, None, 10, 400), counter("tun", false, Some("EasyTier"), 50, 0)]);
        let minute = acc.record_traffic(6000, base + 60_000, &[counter("eth", true, None, 20, 410)]).expect("minute rolled");
        assert_eq!(minute.t, (base / 1000) as i64);
        assert_eq!(minute.tot[0], 4000);
        assert_eq!(minute.tot[1], 300);
        assert_eq!(minute.tot[2], 2000, "peak rate is 4000 bytes in 2 s");
        let eth = minute.i.iter().find(|item| item.0 == "eth").unwrap();
        assert_eq!((eth.4, eth.9), (4000, 2));
        // Tunnel traffic is listed per interface but not added to the total.
        let tun = minute.i.iter().find(|item| item.0 == "tun").unwrap();
        assert_eq!(tun.4, 50);
    }

    #[test]
    fn events_latency_and_wifi_are_counted() {
        let mut acc = Accumulator::default();
        let t = 1_800_000_000_000u64;
        acc.record_events(t, &[json!({"code":"event.adapterDown","tunnel":false}), json!({"code":"event.adapterDown","tunnel":true}), json!({"code":"event.defaultRouteChanged"})]);
        acc.record_latency(t, "gw", "192.168.1.1", "Wi-Fi", Some(3));
        acc.record_latency(t, "gw", "192.168.1.1", "Wi-Fi", None);
        acc.record_wifi(t, "g", "Wi-Fi", Some(80), Some("Home".into()));
        acc.record_wifi(t, "g", "Wi-Fi", Some(60), Some("Home".into()));
        let minute = acc.take().unwrap();
        assert_eq!((minute.e.down, minute.e.tunnel, minute.e.route), (1, 1, 1));
        assert_eq!(minute.lat[0].3, vec![Some(3), None]);
        assert_eq!((minute.w[0].2, minute.w[0].3), (70, 60));
    }

    #[test]
    fn buckets_keep_ranges_under_the_point_limit() {
        for range in ["1h", "24h", "7d", "30d"] {
            let seconds = range_seconds(range).unwrap();
            assert!(seconds / choose_bucket(seconds) <= MAX_POINTS, "{range}");
        }
        assert_eq!(choose_bucket(3600), 60);
        assert_eq!(choose_bucket(86_400), 300);
        assert!(range_seconds("1y").is_none());
    }

    #[test]
    fn summaries_average_over_measured_time_and_keep_gaps() {
        let minute = |t: i64, rx: u64, peak: u64| Minute {
            t,
            i: vec![IfMin("eth".into(), "Ethernet".into(), None, true, rx, 0, peak, 0, 30, 30)],
            tot: [rx, 0, peak, 0],
            lat: vec![LatMin("gw".into(), "g".into(), "Ethernet".into(), vec![Some(2), Some(4), None, Some(10)])],
            ..Default::default()
        };
        let from = 1_800_000_000 / 3600 * 3600;
        let minutes = vec![minute(from, 6000, 500), minute(from + 60, 12_000, 900)];
        let result = summarize(&minutes, from, from + 3600, &["eth".into()]);
        assert_eq!(result["bucketSeconds"], 60);
        assert_eq!(result["total"]["rxBytes"], 18_000);
        assert_eq!(result["total"]["rxAvg"], 150, "18000 bytes over the 2 measured minutes");
        assert_eq!(result["total"]["rxPeak"], 900);
        let points = result["total"]["points"].as_array().unwrap();
        assert_eq!(points.len(), 60);
        assert!(points[5][1].is_null(), "minutes without data are gaps");
        assert_eq!(result["interfaces"][0]["uptimePct"], 100);
        assert_eq!(result["interfaces"][0]["points"].as_array().unwrap().len(), 60);
        let stats = &result["latency"][0]["stats"];
        assert_eq!((stats["sent"].as_u64(), stats["lossPct"].as_u64(), stats["max"].as_u64()), (Some(8), Some(25), Some(10)));
        assert_eq!(stats["p95"], 10);
        assert_eq!(result["daily"][0]["rxBytes"], 18_000);
    }

    #[test]
    fn day_files_round_trip_and_skip_broken_lines() {
        let dir = std::env::temp_dir().join(format!("wnm-history-test-{}", uuid::Uuid::new_v4()));
        let t = Local::now().timestamp() / 60 * 60;
        let minute = Minute { t, tot: [1, 2, 3, 4], ..Default::default() };
        History::append(&dir, &minute).unwrap();
        let file = dir.join(day_file_name(local_day(t)));
        let mut text = fs::read_to_string(&file).unwrap();
        text.push_str("{\"t\":12"); // cut-off last line
        fs::write(&file, text).unwrap();
        let read = read_minutes(&dir, t - 60, t + 60);
        assert_eq!(read, vec![minute]);
        assert!(dir.join("README.txt").exists());
        assert_eq!(clear(&dir), 1);
        assert!(dir.join("README.txt").exists(), "clear only removes day files");
        let _ = fs::remove_dir_all(&dir);
    }
}
