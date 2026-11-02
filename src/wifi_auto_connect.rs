//! Opt-in, per-adapter Wi-Fi reconnection supervision.
//!
//! This module owns scheduling and state transitions only. The application
//! supplies a [`WifiBackend`] so all Windows-specific command execution stays
//! in the existing, validated Rust networking layer. Tests can therefore use
//! an in-memory backend and never touch a real adapter.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const MIN_INTERVAL_SECONDS: u64 = 5;
pub const MAX_INTERVAL_SECONDS: u64 = 86_400;
pub const VERIFY_TIMEOUT_SECONDS: u64 = 30;

/// One persisted auto-connect rule. `adapter_guid` is the stable Windows
/// interface identity; the alias is deliberately resolved at runtime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutoConnectRule {
    pub adapter_guid: String,
    pub profile_name: String,
    pub interval_seconds: u64,
    #[serde(default)]
    pub enabled: bool,
}

/// Data returned by a read-only adapter/profile discovery pass.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredAdapter {
    pub adapter_guid: String,
    pub interface_name: String,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub current_profile: Option<String>,
    #[serde(default)]
    pub profiles: Vec<String>,
}

/// The state observed by a worker for its target adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiObservation {
    pub adapter_guid: String,
    pub connected: bool,
    pub current_profile: Option<String>,
}

impl WifiObservation {
    pub fn target_connected(&self, rule: &AutoConnectRule) -> bool {
        normalize_guid(&self.adapter_guid) == normalize_guid(&rule.adapter_guid)
            && self.connected
            && self.current_profile.as_deref() == Some(rule.profile_name.as_str())
    }
}

/// Implemented by the application using its fixed, hidden Windows commands.
/// `connect` should only submit a connection request. The supervisor verifies
/// the resulting state through `observe` before reporting success.
pub trait WifiBackend: Send + Sync + 'static {
    fn observe(&self, rule: &AutoConnectRule) -> Result<WifiObservation, String>;
    fn connect(&self, rule: &AutoConnectRule) -> Result<(), String>;
}

/// Status shape consumed by the frontend and tray status surfaces.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutoConnectStatus {
    pub adapter_guid: String,
    pub state: String,
    pub last_check: Option<String>,
    pub last_attempt: Option<String>,
    pub last_result: Option<String>,
    pub next_check: Option<String>,
    pub error: Option<String>,
}

impl AutoConnectStatus {
    fn disabled(rule: &AutoConnectRule) -> Self {
        Self {
            adapter_guid: rule.adapter_guid.clone(),
            state: "disabled".into(),
            last_check: None,
            last_attempt: None,
            last_result: None,
            next_check: None,
            error: None,
        }
    }

    fn waiting(rule: &AutoConnectRule, next_check: Option<String>) -> Self {
        Self {
            adapter_guid: rule.adapter_guid.clone(),
            state: "waiting".into(),
            last_check: None,
            last_attempt: None,
            last_result: None,
            next_check,
            error: None,
        }
    }
}

/// Validate a persisted or UI-created rule before it reaches a worker.
pub fn validate_rule(rule: &AutoConnectRule) -> Result<(), String> {
    if !valid_guid(&rule.adapter_guid) {
        return Err("Invalid Wi-Fi adapter GUID.".into());
    }
    if rule.profile_name.trim().is_empty()
        // Windows WLAN profile names are limited to 255 UTF-16 code units.
        || rule.profile_name.encode_utf16().count() > 255
        || rule.profile_name
            .chars()
            .any(|c| c == '\0' || c == '\r' || c == '\n' || c.is_control())
    {
        return Err("Invalid Wi-Fi profile name.".into());
    }
    if !(MIN_INTERVAL_SECONDS..=MAX_INTERVAL_SECONDS).contains(&rule.interval_seconds) {
        return Err(format!(
            "Interval must be between {MIN_INTERVAL_SECONDS} and {MAX_INTERVAL_SECONDS} seconds."
        ));
    }
    Ok(())
}

pub fn valid_guid(value: &str) -> bool {
    let trimmed = value.trim();
    let bare = trimmed
        .strip_prefix('{')
        .and_then(|v| v.strip_suffix('}'))
        .unwrap_or(trimmed);
    let parts: Vec<_> = bare.split('-').collect();
    parts.len() == 5
        && [8usize, 4, 4, 4, 12]
            .iter()
            .zip(parts.iter())
            .all(|(length, part)| {
                part.len() == *length && part.chars().all(|c| c.is_ascii_hexdigit())
            })
}

#[derive(Debug)]
struct WorkerControl {
    state: Mutex<ControlState>,
    wake: Condvar,
}

#[derive(Debug, Clone)]
struct ControlState {
    rule: AutoConnectRule,
    stop: bool,
    generation: u64,
}

struct WorkerHandle {
    control: Arc<WorkerControl>,
    thread: Option<JoinHandle<()>>,
    configured_rule: AutoConnectRule,
}

/// Cancellable supervisor with one independent worker per enabled adapter.
pub struct AutoConnectSupervisor {
    backend: Arc<dyn WifiBackend>,
    statuses: Arc<Mutex<HashMap<String, AutoConnectStatus>>>,
    workers: Mutex<HashMap<String, WorkerHandle>>,
    lifecycle: Mutex<()>,
    stopped: AtomicBool,
    on_status: Arc<dyn Fn(AutoConnectStatus) + Send + Sync>,
}

impl AutoConnectSupervisor {
    pub fn new<B, F>(backend: Arc<B>, on_status: F) -> Arc<Self>
    where
        B: WifiBackend,
        F: Fn(AutoConnectStatus) + Send + Sync + 'static,
    {
        Arc::new(Self {
            backend,
            statuses: Arc::new(Mutex::new(HashMap::new())),
            workers: Mutex::new(HashMap::new()),
            lifecycle: Mutex::new(()),
            stopped: AtomicBool::new(false),
            on_status: Arc::new(on_status),
        })
    }

    /// Replace all rules. Disabled rules remain in the status snapshot but do
    /// not own a worker. Duplicate adapter GUIDs are rejected.
    pub fn apply_rules(self: &Arc<Self>, rules: Vec<AutoConnectRule>) -> Result<(), String> {
        let _lifecycle = self
            .lifecycle
            .lock()
            .map_err(|_| "Auto-connect worker state is unavailable.")?;
        if self.stopped.load(Ordering::Acquire) {
            return Err("Auto-connect supervisor has been stopped.".into());
        }
        let mut seen = HashSet::new();
        for rule in &rules {
            validate_rule(rule)?;
            let key = normalize_guid(&rule.adapter_guid);
            if !seen.insert(key) {
                return Err("Only one auto-connect rule is allowed per adapter.".into());
            }
        }
        let wanted: HashMap<_, _> = rules
            .iter()
            .cloned()
            .map(|rule| (normalize_guid(&rule.adapter_guid), rule))
            .collect();

        let old_keys: Vec<String> = self
            .workers
            .lock()
            .map_err(|_| "Auto-connect worker state is unavailable.")?
            .keys()
            .cloned()
            .collect();
        for key in old_keys {
            if !wanted.get(&key).map(|rule| rule.enabled).unwrap_or(false) {
                self.stop_worker(&key);
            }
        }
        let desired_keys: HashSet<String> = rules
            .iter()
            .map(|rule| normalize_guid(&rule.adapter_guid))
            .collect();
        if let Ok(mut statuses) = self.statuses.lock() {
            // Remove deleted rules from the snapshot. Disabled rules below
            // deliberately retain a disabled status for the UI.
            statuses.retain(|key, _| desired_keys.contains(key));
        }
        for rule in rules {
            if rule.enabled {
                self.start_or_update(rule)?;
            } else {
                self.emit_disabled(&rule);
            }
        }
        Ok(())
    }

    pub fn status_snapshot(&self) -> Vec<AutoConnectStatus> {
        self.statuses
            .lock()
            .map(|statuses| statuses.values().cloned().collect())
            .unwrap_or_default()
    }

    pub fn stop_all(&self) {
        let _lifecycle = self.lifecycle.lock().ok();
        self.stopped.store(true, Ordering::Release);
        let keys: Vec<String> = self
            .workers
            .lock()
            .map(|workers| workers.keys().cloned().collect())
            .unwrap_or_default();
        for key in keys {
            self.stop_worker(&key);
        }
    }

    fn start_or_update(self: &Arc<Self>, rule: AutoConnectRule) -> Result<(), String> {
        let key = normalize_guid(&rule.adapter_guid);
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| "Auto-connect worker state is unavailable.")?;
        if let Some(handle) = workers.get_mut(&key) {
            // Do not wait for an unrelated in-flight connection when another
            // adapter's rule changes and this rule is unchanged.
            if handle.configured_rule == rule {
                return Ok(());
            }
            let mut control = handle
                .control
                .state
                .lock()
                .map_err(|_| "Auto-connect worker state is unavailable.")?;
            control.rule = rule.clone();
            control.generation = control.generation.wrapping_add(1);
            handle.configured_rule = rule;
            handle.control.wake.notify_all();
            return Ok(());
        }

        let control = Arc::new(WorkerControl {
            state: Mutex::new(ControlState {
                rule: rule.clone(),
                stop: false,
                generation: 0,
            }),
            wake: Condvar::new(),
        });
        self.emit(AutoConnectStatus::waiting(
            &rule,
            Some(now_after(rule.interval_seconds)),
        ));
        let thread_control = Arc::clone(&control);
        let supervisor = Arc::clone(self);
        let thread = thread::Builder::new()
            .name(format!("wifi-auto-{}", &key[..8.min(key.len())]))
            .spawn(move || supervisor.worker_loop(thread_control))
            .map_err(|e| format!("Could not start auto-connect worker: {e}"))?;
        workers.insert(
            key,
            WorkerHandle {
                control,
                thread: Some(thread),
                configured_rule: rule,
            },
        );
        Ok(())
    }

    fn stop_worker(&self, key: &str) {
        let handle = self
            .workers
            .lock()
            .ok()
            .and_then(|mut workers| workers.remove(key));
        if let Some(mut handle) = handle {
            let disabled_rule = if let Ok(mut control) = handle.control.state.lock() {
                control.stop = true;
                handle.control.wake.notify_all();
                Some(control.rule.clone())
            } else {
                None
            };
            // Do not allow a replacement worker for this adapter to start
            // while the old worker may still be inside connect().
            if let Some(thread) = handle.thread.take() {
                let _ = thread.join();
            }
            if let Some(rule) = disabled_rule {
                self.emit_disabled(&rule);
            }
        }
    }

    fn worker_loop(self: Arc<Self>, control: Arc<WorkerControl>) {
        loop {
            let (rule, generation) = match control.state.lock() {
                Ok(state) if !state.stop => (state.rule.clone(), state.generation),
                _ => return,
            };
            self.run_cycle(
                &rule,
                &control,
                generation,
                Duration::from_secs(VERIFY_TIMEOUT_SECONDS),
            );
            let interval = Duration::from_secs(rule.interval_seconds);
            let guard = match control.state.lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };
            if guard.stop {
                return;
            }
            if guard.generation != generation {
                // The update already notified the condition variable. Start
                // the new rule immediately instead of waiting the old rule's
                // interval and losing that wake-up.
                continue;
            }
            let _ = control.wake.wait_timeout(guard, interval);
        }
    }

    fn run_cycle(
        &self,
        rule: &AutoConnectRule,
        control: &WorkerControl,
        generation: u64,
        verify_timeout: Duration,
    ) {
        if self.stopped.load(Ordering::Acquire) || should_stop_or_change(control, generation) {
            return;
        }
        let checked_at = now();
        let mut status = self
            .status_for(rule)
            .unwrap_or_else(|| AutoConnectStatus::waiting(rule, None));
        status.last_check = Some(checked_at);
        status.state = "checking".into();
        status.error = None;
        if !self.emit_if_current(control, generation, status.clone()) {
            return;
        }

        if should_stop_or_change(control, generation) {
            return;
        }
        let observation = match self.backend.observe(rule) {
            Ok(observation) => observation,
            Err(error) => {
                if should_stop_or_change(control, generation) {
                    return;
                }
                status.state = "failed".into();
                status.last_result = Some("observe-failed".into());
                status.error = Some(error);
                status.next_check = Some(now_after(rule.interval_seconds));
                self.emit_if_current(control, generation, status);
                return;
            }
        };
        if should_stop_or_change(control, generation) {
            return;
        }
        if observation.target_connected(rule) {
            status.state = "connected".into();
            status.last_result = Some("already-connected".into());
            status.next_check = Some(now_after(rule.interval_seconds));
            self.emit_if_current(control, generation, status);
            return;
        }

        // A rule edit or disable can arrive while observe() is running. Do
        // not submit the old target after that update.
        if should_stop_or_change(control, generation) {
            return;
        }
        status.state = "connecting".into();
        status.last_attempt = Some(now());
        status.last_result = Some("request-sent".into());
        if !self.emit_if_current(control, generation, status.clone()) {
            return;
        }
        match self.connect_if_current(rule, control, generation) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                if should_stop_or_change(control, generation) {
                    return;
                }
                status.state = "failed".into();
                status.last_result = Some("connect-request-failed".into());
                status.error = Some(error);
                status.next_check = Some(now_after(rule.interval_seconds));
                self.emit_if_current(control, generation, status);
                return;
            }
        }

        let deadline = Instant::now() + verify_timeout;
        loop {
            if should_stop_or_change(control, generation) {
                return;
            }
            let observation = self.backend.observe(rule);
            if should_stop_or_change(control, generation) {
                return;
            }
            match observation {
                Ok(observation) if observation.target_connected(rule) => {
                    status.state = "connected".into();
                    status.last_result = Some("connected".into());
                    status.error = None;
                    status.next_check = Some(now_after(rule.interval_seconds));
                    self.emit_if_current(control, generation, status);
                    return;
                }
                Ok(_) => {}
                Err(error) => status.error = Some(error),
            }
            if Instant::now() >= deadline {
                if should_stop_or_change(control, generation) {
                    return;
                }
                status.state = "failed".into();
                status.last_result = Some("verification-timeout".into());
                status.next_check = Some(now_after(rule.interval_seconds));
                self.emit_if_current(control, generation, status);
                return;
            }
            let guard = match control.state.lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };
            if guard.stop || guard.generation != generation {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let _ = control
                .wake
                .wait_timeout(guard, remaining.min(Duration::from_secs(1)));
        }
    }

    fn status_for(&self, rule: &AutoConnectRule) -> Option<AutoConnectStatus> {
        self.statuses
            .lock()
            .ok()
            .and_then(|statuses| statuses.get(&normalize_guid(&rule.adapter_guid)).cloned())
    }

    fn emit_disabled(&self, rule: &AutoConnectRule) {
        let mut status = self
            .status_for(rule)
            .unwrap_or_else(|| AutoConnectStatus::disabled(rule));
        status.state = "disabled".into();
        status.next_check = None;
        status.error = None;
        self.emit(status);
    }

    fn emit_if_current(
        &self,
        control: &WorkerControl,
        generation: u64,
        status: AutoConnectStatus,
    ) -> bool {
        let Ok(guard) = control.state.lock() else {
            return false;
        };
        if self.stopped.load(Ordering::Acquire) || guard.stop || guard.generation != generation {
            return false;
        }
        self.emit(status);
        true
    }

    /// Validate the rule and generation while holding the same control lock
    /// across the backend write. An update or disable therefore cannot pass
    /// the final check and race the connection request.
    fn connect_if_current(
        &self,
        rule: &AutoConnectRule,
        control: &WorkerControl,
        generation: u64,
    ) -> Result<bool, String> {
        let guard = control
            .state
            .lock()
            .map_err(|_| "Auto-connect worker state is unavailable.".to_string())?;
        if self.stopped.load(Ordering::Acquire) || guard.stop || guard.generation != generation {
            return Ok(false);
        }
        self.backend.connect(rule)?;
        Ok(true)
    }

    fn emit(&self, status: AutoConnectStatus) {
        if let Ok(mut statuses) = self.statuses.lock() {
            statuses.insert(normalize_guid(&status.adapter_guid), status.clone());
        }
        (self.on_status)(status);
    }
}

fn should_stop_or_change(control: &WorkerControl, generation: u64) -> bool {
    control
        .state
        .lock()
        .map(|state| state.stop || state.generation != generation)
        .unwrap_or(true)
}

fn normalize_guid(value: &str) -> String {
    value.trim().trim_matches(['{', '}']).to_ascii_lowercase()
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn now_after(seconds: u64) -> String {
    (Utc::now() + chrono::Duration::seconds(seconds as i64)).to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    const TEST_GUID: &str = "255bc7a7-a80c-47e6-902b-386aa6ee90db";

    fn test_rule(enabled: bool) -> AutoConnectRule {
        AutoConnectRule {
            adapter_guid: TEST_GUID.into(),
            profile_name: "Office".into(),
            interval_seconds: MIN_INTERVAL_SECONDS,
            enabled,
        }
    }

    #[test]
    fn validation_rejects_bad_guid_profile_and_interval() {
        let mut rule = AutoConnectRule {
            adapter_guid: "{255bc7a7-a80c-47e6-902b-386aa6ee90db}".into(),
            profile_name: "办公 Wi-Fi".into(),
            interval_seconds: 60,
            enabled: true,
        };
        assert!(validate_rule(&rule).is_ok());
        rule.adapter_guid = "not-a-guid".into();
        assert!(validate_rule(&rule).is_err());
        rule.adapter_guid = "255bc7a7-a80c-47e6-902b-386aa6ee90db".into();
        rule.profile_name = "bad\nprofile".into();
        assert!(validate_rule(&rule).is_err());
        rule.profile_name = "办公 Wi-Fi".into();
        rule.interval_seconds = MIN_INTERVAL_SECONDS - 1;
        assert!(validate_rule(&rule).is_err());

        rule.interval_seconds = MIN_INTERVAL_SECONDS;
        rule.profile_name = "你".repeat(255);
        assert!(validate_rule(&rule).is_ok());
        rule.profile_name.push('你');
        assert!(validate_rule(&rule).is_err());
    }

    #[test]
    fn observation_matches_exact_target_profile_and_adapter() {
        let rule = AutoConnectRule {
            adapter_guid: "255BC7A7-A80C-47E6-902B-386AA6EE90DB".into(),
            profile_name: "Office".into(),
            interval_seconds: 60,
            enabled: true,
        };
        let good = WifiObservation {
            adapter_guid: "{255bc7a7-a80c-47e6-902b-386aa6ee90db}".into(),
            connected: true,
            current_profile: Some("Office".into()),
        };
        assert!(good.target_connected(&rule));
        assert!(!WifiObservation {
            current_profile: Some("Office-Guest".into()),
            ..good.clone()
        }
        .target_connected(&rule));
        assert!(!WifiObservation {
            adapter_guid: "other".into(),
            ..good
        }
        .target_connected(&rule));
    }

    struct RecordingBackend {
        observation: Mutex<Result<WifiObservation, String>>,
        requests: AtomicUsize,
        connect_succeeds: bool,
    }

    impl WifiBackend for RecordingBackend {
        fn observe(&self, _: &AutoConnectRule) -> Result<WifiObservation, String> {
            self.observation.lock().unwrap().clone()
        }
        fn connect(&self, rule: &AutoConnectRule) -> Result<(), String> {
            self.requests.fetch_add(1, Ordering::SeqCst);
            if self.connect_succeeds {
                *self.observation.lock().unwrap() = Ok(WifiObservation {
                    adapter_guid: rule.adapter_guid.clone(),
                    connected: true,
                    current_profile: Some(rule.profile_name.clone()),
                });
            }
            Ok(())
        }
    }

    fn cycle_with_fake_backend(
        observation: Result<WifiObservation, String>,
        connect_succeeds: bool,
    ) -> (usize, AutoConnectStatus) {
        let rule = test_rule(true);
        let backend = Arc::new(RecordingBackend {
            observation: Mutex::new(observation),
            requests: AtomicUsize::new(0),
            connect_succeeds,
        });
        let supervisor = AutoConnectSupervisor::new(Arc::clone(&backend), |_| {});
        let control = WorkerControl {
            state: Mutex::new(ControlState {
                rule: rule.clone(),
                stop: false,
                generation: 0,
            }),
            wake: Condvar::new(),
        };
        supervisor.run_cycle(&rule, &control, 0, Duration::from_millis(20));
        (
            backend.requests.load(Ordering::SeqCst),
            supervisor.status_snapshot().remove(0),
        )
    }

    #[test]
    fn cycles_request_only_when_needed_and_verify_the_connection() {
        let observation = |profile: Option<&str>, connected| {
            Ok(WifiObservation {
                adapter_guid: TEST_GUID.into(),
                connected,
                current_profile: profile.map(str::to_owned),
            })
        };
        let (requests, status) = cycle_with_fake_backend(observation(Some("Office"), true), true);
        assert_eq!(requests, 0);
        assert_eq!(status.last_result.as_deref(), Some("already-connected"));

        let (requests, status) = cycle_with_fake_backend(observation(Some("Other"), true), true);
        assert_eq!(requests, 1);
        assert_eq!(status.last_result.as_deref(), Some("connected"));

        let (requests, status) = cycle_with_fake_backend(Err("Profile unavailable".into()), true);
        assert_eq!(requests, 0);
        assert_eq!(status.error.as_deref(), Some("Profile unavailable"));

        let (requests, status) = cycle_with_fake_backend(observation(None, false), false);
        assert_eq!(requests, 1);
        assert_eq!(status.last_result.as_deref(), Some("verification-timeout"));
    }

    #[test]
    fn a_replaced_rule_cannot_publish_an_old_status() {
        let backend = Arc::new(RecordingBackend {
            observation: Mutex::new(Err("unused".into())),
            requests: AtomicUsize::new(0),
            connect_succeeds: false,
        });
        let supervisor = AutoConnectSupervisor::new(backend, |_| {});
        let rule = test_rule(true);
        let control = WorkerControl {
            state: Mutex::new(ControlState {
                rule: rule.clone(),
                stop: false,
                generation: 1,
            }),
            wake: Condvar::new(),
        };
        let status = AutoConnectStatus::waiting(&rule, None);
        assert!(!supervisor.emit_if_current(&control, 0, status.clone()));
        assert!(supervisor.status_snapshot().is_empty());
        assert!(supervisor.emit_if_current(&control, 1, status));
        assert_eq!(supervisor.status_snapshot().len(), 1);
    }

    #[derive(Default)]
    struct BlockingBackend {
        connect_started: Mutex<usize>,
        connect_profiles: Mutex<Vec<String>>,
        wake: Condvar,
        release: AtomicBool,
        active: AtomicUsize,
        max_active: AtomicUsize,
    }

    impl BlockingBackend {
        fn wait_for_connect(&self) {
            self.wait_for_connect_count(1);
        }

        fn wait_for_connect_count(&self, expected: usize) {
            let mut started = self.connect_started.lock().unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while *started < expected {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !remaining.is_zero(),
                    "timed out waiting for a fake connection request"
                );
                let (next, result) = self.wake.wait_timeout(started, remaining).unwrap();
                started = next;
                assert!(
                    !result.timed_out() || *started >= expected,
                    "timed out waiting for a fake connection request"
                );
            }
        }

        fn allow_connect(&self) {
            self.release.store(true, Ordering::Release);
            self.wake.notify_all();
        }
    }

    impl WifiBackend for BlockingBackend {
        fn observe(&self, rule: &AutoConnectRule) -> Result<WifiObservation, String> {
            Ok(WifiObservation {
                adapter_guid: rule.adapter_guid.clone(),
                connected: false,
                current_profile: None,
            })
        }

        fn connect(&self, rule: &AutoConnectRule) -> Result<(), String> {
            let active = self.active.fetch_add(1, Ordering::AcqRel) + 1;
            self.max_active.fetch_max(active, Ordering::AcqRel);
            {
                let mut started = self.connect_started.lock().unwrap();
                *started += 1;
                self.connect_profiles
                    .lock()
                    .unwrap()
                    .push(rule.profile_name.clone());
                self.wake.notify_all();
            }
            while !self.release.load(Ordering::Acquire) {
                let started = self.connect_started.lock().unwrap();
                let _ = self.wake.wait_timeout(started, Duration::from_millis(10));
            }
            self.active.fetch_sub(1, Ordering::AcqRel);
            Ok(())
        }
    }

    struct SeparateAdaptersBackend {
        first: Arc<BlockingBackend>,
        second_started: std::sync::mpsc::Sender<()>,
    }

    impl WifiBackend for SeparateAdaptersBackend {
        fn observe(&self, rule: &AutoConnectRule) -> Result<WifiObservation, String> {
            self.first.observe(rule)
        }
        fn connect(&self, rule: &AutoConnectRule) -> Result<(), String> {
            if normalize_guid(&rule.adapter_guid) == TEST_GUID {
                self.first.connect(rule)
            } else {
                let _ = self.second_started.send(());
                Ok(())
            }
        }
    }

    #[test]
    fn different_adapters_progress_while_one_request_is_in_flight() {
        let first = Arc::new(BlockingBackend::default());
        let (second_started, receiver) = std::sync::mpsc::channel();
        let backend = Arc::new(SeparateAdaptersBackend {
            first: Arc::clone(&first),
            second_started,
        });
        let supervisor = AutoConnectSupervisor::new(backend, |_| {});
        supervisor.apply_rules(vec![test_rule(true)]).unwrap();
        first.wait_for_connect();

        let mut second = test_rule(true);
        second.adapter_guid = "355bc7a7-a80c-47e6-902b-386aa6ee90db".into();
        let changing = Arc::clone(&supervisor);
        let update = thread::spawn(move || changing.apply_rules(vec![test_rule(true), second]));
        let second_progressed = receiver.recv_timeout(Duration::from_secs(2)).is_ok();
        // Always release the blocked request before joining, including when the assertion fails.
        first.allow_connect();
        update.join().unwrap().unwrap();
        supervisor.stop_all();
        assert!(
            second_progressed,
            "the second adapter was blocked by the first"
        );
    }

    #[test]
    fn disabling_waits_for_in_flight_worker_before_reenable() {
        let backend = Arc::new(BlockingBackend::default());
        let statuses = Arc::new(Mutex::new(Vec::<AutoConnectStatus>::new()));
        let status_sink = Arc::clone(&statuses);
        let supervisor = AutoConnectSupervisor::new(Arc::clone(&backend), move |status| {
            status_sink.lock().unwrap().push(status);
        });

        supervisor.apply_rules(vec![test_rule(true)]).unwrap();
        backend.wait_for_connect();

        let stopping = Arc::clone(&supervisor);
        let disable = thread::spawn(move || stopping.apply_rules(vec![test_rule(false)]));
        // apply_rules must join the old worker instead of returning while its
        // backend call is still active.
        thread::sleep(Duration::from_millis(40));
        assert!(!disable.is_finished());
        backend.allow_connect();
        disable.join().unwrap().unwrap();

        supervisor.apply_rules(vec![test_rule(true)]).unwrap();
        backend.wait_for_connect_count(2);
        supervisor.stop_all();
        assert_eq!(backend.max_active.load(Ordering::Acquire), 1);
        let snapshot = supervisor.status_snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].adapter_guid, TEST_GUID);
    }

    #[test]
    fn rule_update_cannot_overtake_an_in_flight_connect() {
        let backend = Arc::new(BlockingBackend::default());
        let supervisor = AutoConnectSupervisor::new(Arc::clone(&backend), |_| {});
        supervisor.apply_rules(vec![test_rule(true)]).unwrap();
        backend.wait_for_connect();

        let mut replacement = test_rule(true);
        replacement.profile_name = "Backup".into();
        let updating = Arc::clone(&supervisor);
        let update = thread::spawn(move || updating.apply_rules(vec![replacement]));
        thread::sleep(Duration::from_millis(40));
        assert!(!update.is_finished());
        assert_eq!(*backend.connect_started.lock().unwrap(), 1);

        backend.allow_connect();
        update.join().unwrap().unwrap();
        backend.wait_for_connect_count(2);
        supervisor.stop_all();
        let profiles = backend.connect_profiles.lock().unwrap().clone();
        assert_eq!(profiles, vec!["Office", "Backup"]);
        assert_eq!(backend.max_active.load(Ordering::Acquire), 1);
    }
}
