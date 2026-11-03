//! Read-only network tools for the Tools page: connectivity test, route
//! lookup, traceroute and active connections. Nothing here changes network
//! configuration; ICMP echoes and DNS lookups run only when the user asks.

use crate::{arrays, coexistence, field_i64, field_string, route_key};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    ffi::c_void,
    mem::size_of,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs},
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::{GetLastError, HANDLE, INVALID_HANDLE_VALUE},
    NetworkManagement::IpHelper::{
        GetExtendedTcpTable, GetExtendedUdpTable, IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho2Ex,
        ICMP_ECHO_REPLY, IP_OPTION_INFORMATION, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID,
        MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID, MIB_UDP6ROW_OWNER_PID, MIB_UDP6TABLE_OWNER_PID,
        MIB_UDPROW_OWNER_PID, MIB_UDPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID,
    },
    Networking::WinSock::{AF_INET, AF_INET6},
};

/// Public hosts contacted only when the user runs the connectivity test.
pub const INTERNET_TARGET: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);
pub const DNS_TEST_HOST: &str = "www.msftconnecttest.com";

const ECHO_COUNT: usize = 4;
const ECHO_TIMEOUT_MS: u32 = 1000;
const TRACE_TIMEOUT_MS: u32 = 1000;
const TRACE_PROBES: usize = 2;
const MAX_CONNECTIONS: usize = 4000;

// IP_STATUS values returned by the ICMP API.
pub(crate) const IP_SUCCESS: u32 = 0;
const IP_TTL_EXPIRED_TRANSIT: u32 = 11013;

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn valid_hostname(value: &str) -> bool {
    value.len() <= 253
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

/// Accept an IPv4/IPv6 literal or a DNS host name; nothing else reaches
/// Windows APIs or PowerShell.
pub fn valid_target(value: &str) -> Result<String, String> {
    let value = value.trim().trim_end_matches('.');
    let unbracketed = value.trim_start_matches('[').trim_end_matches(']');
    if unbracketed.parse::<IpAddr>().is_ok() {
        return Ok(unbracketed.to_string());
    }
    if !value.is_empty() && valid_hostname(value) {
        return Ok(value.to_ascii_lowercase());
    }
    Err("Enter an IPv4/IPv6 address or a host name such as example.com.".into())
}

/// Resolve with the Windows resolver. IP literals return themselves.
pub fn resolve(target: &str) -> Result<Vec<IpAddr>, String> {
    if let Ok(ip) = target.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }
    let mut addresses = (target, 0)
        .to_socket_addrs()
        .map_err(|e| format!("Cannot resolve {target}: {e}"))?
        .map(|address| address.ip())
        .collect::<Vec<_>>();
    addresses.dedup();
    if addresses.is_empty() {
        return Err(format!("{target} has no addresses."));
    }
    Ok(addresses)
}

fn preferred_ipv4(addresses: &[IpAddr]) -> Option<Ipv4Addr> {
    addresses.iter().find_map(|ip| match ip {
        IpAddr::V4(v4) => Some(*v4),
        IpAddr::V6(_) => None,
    })
}

// ---------------------------------------------------------------------------
// ICMP
// ---------------------------------------------------------------------------

pub(crate) struct IcmpHandle(HANDLE);
impl IcmpHandle {
    pub(crate) fn open() -> Result<Self, String> {
        let handle = unsafe { IcmpCreateFile() };
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return Err("Windows ICMP service is unavailable.".into());
        }
        Ok(Self(handle))
    }
}
impl Drop for IcmpHandle {
    fn drop(&mut self) {
        unsafe {
            IcmpCloseHandle(self.0);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Echo {
    pub status: u32,
    pub rtt: Option<u32>,
    pub from: Option<Ipv4Addr>,
}

pub(crate) fn echo(handle: &IcmpHandle, source: Option<Ipv4Addr>, destination: Ipv4Addr, ttl: u8, timeout: u32) -> Echo {
    let data = [0x57u8; 32];
    let options = IP_OPTION_INFORMATION { Ttl: ttl, Tos: 0, Flags: 0, OptionsSize: 0, OptionsData: ptr::null_mut() };
    // Room for one reply, the echoed data, an ICMP error and padding.
    let mut buffer = vec![0u64; (size_of::<ICMP_ECHO_REPLY>() + data.len() + 8 + 128) / 8];
    let count = unsafe {
        IcmpSendEcho2Ex(
            handle.0,
            ptr::null_mut(),
            None,
            ptr::null(),
            source.map(|ip| u32::from_ne_bytes(ip.octets())).unwrap_or(0),
            u32::from_ne_bytes(destination.octets()),
            data.as_ptr().cast::<c_void>(),
            data.len() as u16,
            &options,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * 8) as u32,
            timeout,
        )
    };
    if count == 0 {
        return Echo { status: unsafe { GetLastError() }, rtt: None, from: None };
    }
    let reply: ICMP_ECHO_REPLY = unsafe { ptr::read_unaligned(buffer.as_ptr().cast()) };
    Echo {
        status: reply.Status,
        rtt: Some(reply.RoundTripTime),
        from: Some(Ipv4Addr::from(reply.Address.to_ne_bytes())),
    }
}

/// Loss and latency for a series of echoes (`None` = no reply).
pub fn summarize(samples: &[Option<u32>]) -> Value {
    let received = samples.iter().flatten().copied().collect::<Vec<_>>();
    let sent = samples.len();
    let loss = if sent == 0 { 100 } else { (sent - received.len()) * 100 / sent };
    let avg = (!received.is_empty()).then(|| received.iter().sum::<u32>() / received.len() as u32);
    json!({
        "sent":sent,
        "received":received.len(),
        "lossPercent":loss,
        "minMs":received.iter().min(),
        "avgMs":avg,
        "maxMs":received.iter().max(),
        "ok":!received.is_empty()
    })
}

fn ping(source: Option<Ipv4Addr>, destination: Ipv4Addr) -> Value {
    let samples = match IcmpHandle::open() {
        Ok(handle) => (0..ECHO_COUNT)
            .map(|_| {
                let reply = echo(&handle, source, destination, 128, ECHO_TIMEOUT_MS);
                (reply.status == IP_SUCCESS).then(|| reply.rtt.unwrap_or(0))
            })
            .collect::<Vec<_>>(),
        Err(_) => vec![None; ECHO_COUNT],
    };
    let mut result = summarize(&samples);
    result["target"] = json!(destination.to_string());
    result
}

// ---------------------------------------------------------------------------
// Connectivity test
// ---------------------------------------------------------------------------

fn first_ipv4(values: Option<&Value>, key: Option<&str>) -> Option<Ipv4Addr> {
    arrays(values).iter().find_map(|item| {
        let text = match key {
            Some(key) => field_string(item, key),
            None => item.as_str().unwrap_or_default().to_string(),
        };
        text.split('/').next().and_then(|ip| ip.parse::<Ipv4Addr>().ok())
    })
}

/// Which checks failed for one adapter, worst first.
pub fn verdict(checks: &[Value]) -> &'static str {
    let ok = |kind: &str| -> Option<bool> {
        let list = checks.iter().filter(|check| field_string(check, "kind") == kind).collect::<Vec<_>>();
        (!list.is_empty()).then(|| list.iter().any(|check| check.get("ok").and_then(Value::as_bool).unwrap_or(false)))
    };
    if ok("gateway") == Some(false) {
        "gateway"
    } else if ok("internet") == Some(false) {
        "internet"
    } else if ok("dns") == Some(false) {
        "dns"
    } else {
        "ok"
    }
}

fn dns_check() -> Value {
    match resolve(DNS_TEST_HOST) {
        Ok(addresses) => {
            let texts = addresses.iter().map(IpAddr::to_string).collect::<Vec<_>>();
            let fake = texts.iter().any(|ip| coexistence::is_fake_ip(ip));
            json!({"host":DNS_TEST_HOST,"ok":true,"addresses":texts,"fakeIp":fake})
        }
        Err(error) => json!({"host":DNS_TEST_HOST,"ok":false,"addresses":[],"fakeIp":false,"error":error}),
    }
}

/// Ping the gateway, DNS servers and an Internet host from each connected
/// adapter (bound to its address), plus the default path Windows chooses.
pub fn connectivity_test(overview: &Value, routes: &Value) -> Value {
    let candidates = arrays(overview.get("interfaces"))
        .into_iter()
        .filter(|item| {
            !item.get("systemInterface").and_then(Value::as_bool).unwrap_or(false)
                && field_string(item, "operationalStatus").eq_ignore_ascii_case("up")
                && first_ipv4(item.get("ipv4Gateways"), None).is_some()
                && first_ipv4(item.get("ipv4Addresses"), Some("ipAddress")).is_some()
        })
        .collect::<Vec<_>>();
    let mut jobs: Vec<(usize, &'static str, Option<Ipv4Addr>, Ipv4Addr)> = vec![];
    for (slot, item) in candidates.iter().enumerate() {
        let source = first_ipv4(item.get("ipv4Addresses"), Some("ipAddress"));
        if let Some(gateway) = first_ipv4(item.get("ipv4Gateways"), None) {
            jobs.push((slot, "gateway", source, gateway));
        }
        for dns in arrays(item.get("dnsServers"))
            .iter()
            .filter_map(|value| value.as_str().and_then(|ip| ip.parse::<Ipv4Addr>().ok()))
            .filter(|ip| !ip.is_loopback() && !coexistence::is_fake_ip(&ip.to_string()))
            .take(2)
        {
            jobs.push((slot, "dns", source, dns));
        }
        jobs.push((slot, "internet", source, INTERNET_TARGET));
    }
    let default_slot = candidates.len();
    jobs.push((default_slot, "internet", None, INTERNET_TARGET));
    let (results, dns) = thread::scope(|scope| {
        let dns = scope.spawn(dns_check);
        let handles = jobs
            .iter()
            .map(|(slot, kind, source, target)| {
                let (slot, kind, source, target) = (*slot, *kind, *source, *target);
                scope.spawn(move || {
                    let mut result = ping(source, target);
                    result["kind"] = json!(kind);
                    (slot, result)
                })
            })
            .collect::<Vec<_>>();
        let results = handles.into_iter().filter_map(|handle| handle.join().ok()).collect::<Vec<_>>();
        (results, dns.join().unwrap_or(Value::Null))
    });
    let mut by_slot: HashMap<usize, Vec<Value>> = HashMap::new();
    for (slot, result) in results {
        by_slot.entry(slot).or_default().push(result);
    }
    let adapters = candidates
        .iter()
        .enumerate()
        .map(|(slot, item)| {
            let checks = by_slot.remove(&slot).unwrap_or_default();
            json!({
                "interfaceIndex":field_i64(item,"interfaceIndex"),
                "alias":field_string(item,"alias"),
                "managedBy":item.get("managedBy").cloned().unwrap_or(Value::Null),
                "source":first_ipv4(item.get("ipv4Addresses"), Some("ipAddress")).map(|ip| ip.to_string()),
                "verdict":verdict(&checks),
                "checks":checks
            })
        })
        .collect::<Vec<_>>();
    let internet = by_slot.remove(&default_slot).and_then(|mut list| list.pop()).unwrap_or(Value::Null);
    json!({
        "testedAt":chrono::Utc::now().to_rfc3339(),
        "defaultPath":{
            "internet":internet,
            "dns":dns,
            "via":coexistence::default_via(overview, routes)
        },
        "adapters":adapters
    })
}

// ---------------------------------------------------------------------------
// Route lookup
// ---------------------------------------------------------------------------

const FIND_ROUTE_SCRIPT: &str = r#"$r=@(Find-NetRoute -RemoteIPAddress ([string]$InputData.address) -ErrorAction Stop);$a=$r|Where-Object{$_.CimClass.CimClassName -eq 'MSFT_NetIPAddress'}|Select-Object -First 1;$n=$r|Where-Object{$_.CimClass.CimClassName -eq 'MSFT_NetRoute'}|Select-Object -First 1;[PSCustomObject]@{sourceAddress=[string]$a.IPAddress;addressFamily=[string]$n.AddressFamily;destinationPrefix=[string]$n.DestinationPrefix;nextHop=[string]$n.NextHop;interfaceIndex=[int]$n.InterfaceIndex;interfaceAlias=[string]$n.InterfaceAlias;routeMetric=[long]$n.RouteMetric}|ConvertTo-Json -Compress"#;

/// Which route, adapter and source address Windows would use for a host.
pub fn lookup_route(target: &str, overview: &Value, routes: &Value) -> Result<Value, crate::PublicError> {
    let target = valid_target(target).map_err(|message| crate::err("VALIDATION_ERROR", message))?;
    let addresses = resolve(&target).map_err(|message| crate::err("RESOLVE_FAILED", message))?;
    let address = preferred_ipv4(&addresses).map(IpAddr::V4).unwrap_or(addresses[0]);
    let mut route = crate::run_powershell_json(FIND_ROUTE_SCRIPT, Some(&json!({"address":address.to_string()})))?;
    let index = field_i64(&route, "interfaceIndex");
    let known = arrays(routes.get("routes")).into_iter().find(|item| route_key(item) == route_key(&route));
    if let Some(known) = &known {
        route["effectiveMetric"] = known.get("effectiveMetric").cloned().unwrap_or(Value::Null);
        route["interfaceAlias"] = json!(field_string(known, "interfaceAlias"));
    }
    let owner = coexistence::protected_owner(overview, index).map(|(_, owner)| owner);
    let texts = addresses.iter().map(IpAddr::to_string).collect::<Vec<_>>();
    Ok(json!({
        "target":target,
        "address":address.to_string(),
        "addresses":texts,
        "fakeIp":coexistence::is_fake_ip(&address.to_string()),
        "route":route,
        "managedBy":owner
    }))
}

// ---------------------------------------------------------------------------
// Traceroute
// ---------------------------------------------------------------------------

static TRACE_ID: AtomicU64 = AtomicU64::new(0);
static TRACE_CANCEL: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

/// Stop the running trace, if any.
pub fn cancel_trace() {
    if let Ok(mut current) = TRACE_CANCEL.lock() {
        if let Some(flag) = current.take() {
            flag.store(true, Ordering::SeqCst);
        }
    }
}

/// Prepare a trace and return `(id, destination, cancel flag)`; a new trace
/// cancels the previous one.
pub fn begin_trace(target: &str) -> Result<(u64, String, Ipv4Addr, Arc<AtomicBool>), String> {
    let target = valid_target(target)?;
    let addresses = resolve(&target)?;
    let destination = preferred_ipv4(&addresses).ok_or("Traceroute supports IPv4 destinations only.")?;
    cancel_trace();
    let flag = Arc::new(AtomicBool::new(false));
    if let Ok(mut current) = TRACE_CANCEL.lock() {
        *current = Some(flag.clone());
    }
    Ok((TRACE_ID.fetch_add(1, Ordering::SeqCst) + 1, target, destination, flag))
}

/// Run a hop-by-hop trace, calling `emit` for each hop. Returns
/// `(reached, cancelled)`.
pub fn run_trace(
    destination: Ipv4Addr,
    max_hops: u8,
    cancel: &AtomicBool,
    mut emit: impl FnMut(Value),
) -> Result<(bool, bool), String> {
    let handle = IcmpHandle::open()?;
    for ttl in 1..=max_hops.clamp(1, 30) {
        let mut rtts = vec![];
        let mut address = None;
        let mut reached = false;
        for _ in 0..TRACE_PROBES {
            if cancel.load(Ordering::SeqCst) {
                return Ok((false, true));
            }
            let reply = echo(&handle, None, destination, ttl, TRACE_TIMEOUT_MS);
            match reply.status {
                IP_SUCCESS | IP_TTL_EXPIRED_TRANSIT => {
                    rtts.push(reply.rtt.map(Value::from).unwrap_or(Value::Null));
                    address = address.or(reply.from);
                    reached |= reply.status == IP_SUCCESS;
                }
                _ => rtts.push(Value::Null),
            }
        }
        emit(json!({
            "hop":ttl,
            "address":address.map(|ip| ip.to_string()),
            "rttsMs":rtts,
            "status":if reached { "reached" } else if address.is_some() { "transit" } else { "timeout" }
        }));
        if reached {
            return Ok((true, false));
        }
    }
    Ok((false, cancel.load(Ordering::SeqCst)))
}

// ---------------------------------------------------------------------------
// Active connections
// ---------------------------------------------------------------------------

pub fn tcp_state_text(state: u32) -> &'static str {
    match state {
        1 => "Closed",
        2 => "Listen",
        3 => "SynSent",
        4 => "SynReceived",
        5 => "Established",
        6 => "FinWait1",
        7 => "FinWait2",
        8 => "CloseWait",
        9 => "Closing",
        10 => "LastAck",
        11 => "TimeWait",
        12 => "DeleteTcb",
        _ => "Unknown",
    }
}

fn port(value: u32) -> u16 {
    u16::from_be((value & 0xffff) as u16)
}

fn read_table(fetch: impl Fn(*mut c_void, &mut u32) -> u32) -> Option<Vec<u64>> {
    let mut size = 0u32;
    fetch(ptr::null_mut(), &mut size);
    for _ in 0..4 {
        let mut buffer = vec![0u64; size as usize / 8 + 64];
        let mut available = (buffer.len() * 8) as u32;
        match fetch(buffer.as_mut_ptr().cast(), &mut available) {
            0 => return Some(buffer),
            122 => size = available,
            _ => return None,
        }
    }
    None
}

unsafe fn rows<T: Copy, Row: Copy>(buffer: &[u64], first_row: impl Fn(*const T) -> *const Row, count: impl Fn(*const T) -> u32) -> Vec<Row> {
    let table = buffer.as_ptr().cast::<T>();
    let total = (count(table) as usize).min(MAX_CONNECTIONS * 2);
    std::slice::from_raw_parts(first_row(table), total).to_vec()
}

/// Maps local addresses to the adapter alias that owns them.
pub fn adapter_for_address(addresses: &HashMap<String, String>, ip: &str) -> String {
    let ip = ip.split('%').next().unwrap_or(ip);
    match ip.parse::<IpAddr>() {
        Ok(address) if address.is_unspecified() => "*".into(),
        Ok(address) if address.is_loopback() => "Loopback".into(),
        _ => addresses.get(ip).cloned().unwrap_or_default(),
    }
}

/// TCP and UDP endpoints with their owning process and adapter.
pub fn active_connections(overview: &Value) -> Value {
    let processes = coexistence::running_processes();
    let software = coexistence::detect_software(processes.values().map(String::as_str));
    let products: HashMap<String, String> = software
        .iter()
        .flat_map(|item| {
            let product = field_string(item, "product");
            arrays(item.get("processes"))
                .into_iter()
                .map(move |process| (process.as_str().unwrap_or_default().to_lowercase(), product.clone()))
        })
        .collect();
    let mut addresses = HashMap::new();
    for item in arrays(overview.get("interfaces")) {
        for address in arrays(item.get("ipv4Addresses")).into_iter().chain(arrays(item.get("ipv6Addresses"))) {
            addresses.insert(field_string(&address, "ipAddress"), field_string(&item, "alias"));
        }
    }
    let mut list = vec![];
    let mut push = |protocol: &str, family: &str, local: String, local_port: u16, remote: Option<(String, u16)>, state: &str, pid: u32| {
        let process = match pid {
            0 => "System Idle Process".to_string(),
            4 => "System".to_string(),
            _ => processes.get(&pid).cloned().unwrap_or_default(),
        };
        let key = process.to_lowercase();
        let product = products.get(key.strip_suffix(".exe").unwrap_or(&key)).cloned();
        list.push(json!({
            "protocol":protocol,
            "family":family,
            "localAddress":local,
            "localPort":local_port,
            "remoteAddress":remote.as_ref().map(|(ip, _)| ip.clone()),
            "remotePort":remote.as_ref().map(|(_, port)| *port),
            "state":state,
            "pid":pid,
            "process":process,
            "product":product,
            "adapter":adapter_for_address(&addresses, &local)
        }));
    };
    let tcp4 = read_table(|buffer, size| unsafe { GetExtendedTcpTable(buffer, size, 1, AF_INET as u32, TCP_TABLE_OWNER_PID_ALL, 0) });
    if let Some(buffer) = tcp4 {
        for row in unsafe { rows::<MIB_TCPTABLE_OWNER_PID, MIB_TCPROW_OWNER_PID>(&buffer, |t| (*t).table.as_ptr(), |t| (*t).dwNumEntries) } {
            let remote = (row.dwState != 2).then(|| (Ipv4Addr::from(row.dwRemoteAddr.to_ne_bytes()).to_string(), port(row.dwRemotePort)));
            push("TCP", "IPv4", Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes()).to_string(), port(row.dwLocalPort), remote, tcp_state_text(row.dwState), row.dwOwningPid);
        }
    }
    let tcp6 = read_table(|buffer, size| unsafe { GetExtendedTcpTable(buffer, size, 1, AF_INET6 as u32, TCP_TABLE_OWNER_PID_ALL, 0) });
    if let Some(buffer) = tcp6 {
        for row in unsafe { rows::<MIB_TCP6TABLE_OWNER_PID, MIB_TCP6ROW_OWNER_PID>(&buffer, |t| (*t).table.as_ptr(), |t| (*t).dwNumEntries) } {
            let remote = (row.dwState != 2).then(|| (Ipv6Addr::from(row.ucRemoteAddr).to_string(), port(row.dwRemotePort)));
            push("TCP", "IPv6", Ipv6Addr::from(row.ucLocalAddr).to_string(), port(row.dwLocalPort), remote, tcp_state_text(row.dwState), row.dwOwningPid);
        }
    }
    let udp4 = read_table(|buffer, size| unsafe { GetExtendedUdpTable(buffer, size, 1, AF_INET as u32, UDP_TABLE_OWNER_PID, 0) });
    if let Some(buffer) = udp4 {
        for row in unsafe { rows::<MIB_UDPTABLE_OWNER_PID, MIB_UDPROW_OWNER_PID>(&buffer, |t| (*t).table.as_ptr(), |t| (*t).dwNumEntries) } {
            push("UDP", "IPv4", Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes()).to_string(), port(row.dwLocalPort), None, "", row.dwOwningPid);
        }
    }
    let udp6 = read_table(|buffer, size| unsafe { GetExtendedUdpTable(buffer, size, 1, AF_INET6 as u32, UDP_TABLE_OWNER_PID, 0) });
    if let Some(buffer) = udp6 {
        for row in unsafe { rows::<MIB_UDP6TABLE_OWNER_PID, MIB_UDP6ROW_OWNER_PID>(&buffer, |t| (*t).table.as_ptr(), |t| (*t).dwNumEntries) } {
            push("UDP", "IPv6", Ipv6Addr::from(row.ucLocalAddr).to_string(), port(row.dwLocalPort), None, "", row.dwOwningPid);
        }
    }
    let total = list.len();
    list.truncate(MAX_CONNECTIONS);
    json!({"connections":list,"total":total,"truncated":total > MAX_CONNECTIONS,"readAt":chrono::Utc::now().to_rfc3339()})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_accept_ips_and_host_names_only() {
        assert_eq!(valid_target(" 1.1.1.1 ").unwrap(), "1.1.1.1");
        assert_eq!(valid_target("[2606:4700::1111]").unwrap(), "2606:4700::1111");
        assert_eq!(valid_target("Example.COM.").unwrap(), "example.com");
        assert_eq!(valid_target("intranet").unwrap(), "intranet");
        for bad in ["", "a b", "x;calc", "$(whoami)", "-bad.com", "a..b", "host/path", "'quoted'", &"a".repeat(64)] {
            assert!(valid_target(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn echo_summaries_report_loss_and_latency() {
        let all = summarize(&[Some(10), Some(20), None, Some(30)]);
        assert_eq!(all["sent"], 4);
        assert_eq!(all["received"], 3);
        assert_eq!(all["lossPercent"], 25);
        assert_eq!(all["minMs"], 10);
        assert_eq!(all["avgMs"], 20);
        assert_eq!(all["maxMs"], 30);
        assert_eq!(all["ok"], true);
        let none = summarize(&[None, None]);
        assert_eq!(none["lossPercent"], 100);
        assert!(none["avgMs"].is_null());
        assert_eq!(none["ok"], false);
    }

    #[test]
    fn verdict_names_the_first_failing_layer() {
        let check = |kind: &str, ok: bool| json!({"kind":kind,"ok":ok});
        assert_eq!(verdict(&[check("gateway", true), check("dns", true), check("internet", true)]), "ok");
        assert_eq!(verdict(&[check("gateway", false), check("internet", false)]), "gateway");
        assert_eq!(verdict(&[check("gateway", true), check("internet", false)]), "internet");
        assert_eq!(verdict(&[check("gateway", true), check("dns", false), check("dns", true), check("internet", true)]), "ok");
        assert_eq!(verdict(&[check("gateway", true), check("dns", false), check("internet", true)]), "dns");
    }

    #[test]
    fn connection_helpers_map_states_ports_and_adapters() {
        assert_eq!(tcp_state_text(2), "Listen");
        assert_eq!(tcp_state_text(5), "Established");
        assert_eq!(tcp_state_text(99), "Unknown");
        assert_eq!(port(u32::from(0x1f90u16.to_be())), 8080);
        let addresses = HashMap::from([("192.168.1.20".to_string(), "WLAN".to_string())]);
        assert_eq!(adapter_for_address(&addresses, "192.168.1.20"), "WLAN");
        assert_eq!(adapter_for_address(&addresses, "0.0.0.0"), "*");
        assert_eq!(adapter_for_address(&addresses, "::"), "*");
        assert_eq!(adapter_for_address(&addresses, "127.0.0.1"), "Loopback");
        assert_eq!(adapter_for_address(&addresses, "10.0.0.9"), "");
    }

    #[test]
    #[ignore = "manual, read-only check of local connection tables"]
    fn read_only_connection_tables() {
        let result = active_connections(&json!({"interfaces":[]}));
        assert!(result["total"].as_u64().unwrap() > 0);
    }
}
