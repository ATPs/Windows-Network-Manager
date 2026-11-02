//! Coexistence with tunnel and proxy software such as EasyTier and v2rayN
//! (xray, sing-box, mihomo). Those programs create, recreate and remove their
//! own adapters and routes, so this app labels them and refuses changes that
//! would fight the owning program. Everything here is read-only.

use crate::{arrays, field_bool, field_i64, field_string};
use serde_json::{json, Value};
use std::{collections::HashMap, mem::size_of, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ},
    },
};

/// Owner placeholder for a proxy TUN adapter; replaced by the detected product.
const PROXY_OWNER: &str = "proxy";

/// Process names (lowercase, without `.exe`) of proxy cores that v2rayN and
/// similar clients launch.
const PROXY_CORES: &[&str] = &["xray", "v2ray", "sing-box", "mihomo", "clash", "clash-meta"];

/// Other programs that own network adapters, keyed by lowercase process name.
const TUNNEL_PROGRAMS: &[(&str, &str)] = &[
    ("easytier-gui", "EasyTier"),
    ("easytier-core", "EasyTier"),
    ("tailscaled", "Tailscale"),
    ("tailscale-ipn", "Tailscale"),
    ("zerotier-one_x64", "ZeroTier"),
    ("zerotier_desktop_ui", "ZeroTier"),
    ("wireguard", "WireGuard"),
];

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Which program owns an adapter, judged from its name, description and
/// interface type. Wintun adapters describe themselves as "<name> Tunnel".
pub fn tunnel_owner(alias: &str, description: &str, if_type: i64) -> Option<&'static str> {
    let alias = alias.trim().to_lowercase();
    let description = description.trim().to_lowercase();
    let text = format!("{alias} {description}");
    if text.contains("easytier") || (alias.starts_with("et_") && if_type == 53) {
        return Some("EasyTier");
    }
    if ["sing-box", "singbox", "sing-tun", "mihomo", "clash", "xray", "v2ray"]
        .iter()
        .any(|key| text.contains(key))
        || alias == "meta"
    {
        return Some(PROXY_OWNER);
    }
    for (key, owner) in [("tailscale", "Tailscale"), ("zerotier", "ZeroTier"), ("wireguard", "WireGuard")] {
        if text.contains(key) {
            return Some(owner);
        }
    }
    if description.contains("wintun")
        || description.contains("tap-windows")
        || description.contains("tap-win32")
        || (if_type == 53 && description.ends_with(" tunnel"))
    {
        return Some("tunnel");
    }
    None
}

fn process_key(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

/// Group running proxy/VPN processes by product. Proxy cores are attributed
/// to v2rayN when v2rayN itself is running.
pub fn detect_software<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<Value> {
    let mut keys = names.into_iter().map(process_key).collect::<Vec<_>>();
    keys.sort();
    keys.dedup();
    let v2rayn = keys.iter().any(|key| key == "v2rayn");
    let mut products: Vec<(String, Vec<String>)> = vec![];
    let mut add = |product: &str, process: &str| {
        match products.iter_mut().find(|(name, _)| name == product) {
            Some((_, list)) => list.push(process.to_string()),
            None => products.push((product.to_string(), vec![process.to_string()])),
        }
    };
    for key in &keys {
        if key == "v2rayn" {
            add("v2rayN", "v2rayN");
        } else if PROXY_CORES.contains(&key.as_str()) {
            add(if v2rayn { "v2rayN" } else { key }, key);
        } else if let Some((_, product)) = TUNNEL_PROGRAMS.iter().find(|(name, _)| name == key) {
            add(product, key);
        }
    }
    products
        .into_iter()
        .map(|(product, processes)| json!({"product":product,"processes":processes}))
        .collect()
}

/// The name shown for proxy TUN adapters: the running proxy product, or a
/// generic label when none is detected.
pub fn proxy_product(software: &[Value]) -> String {
    software
        .iter()
        .map(|item| field_string(item, "product"))
        .find(|product| product == "v2rayN" || PROXY_CORES.contains(&product.as_str()))
        .unwrap_or_else(|| "Proxy TUN".into())
}

/// Set `managedBy` on every interface (null when the app may manage it).
pub fn annotate_overview(overview: &mut Value, proxy_product: &str) {
    let Some(items) = overview.get_mut("interfaces").and_then(Value::as_array_mut) else {
        return;
    };
    for item in items {
        let owner = if field_bool(item, "systemInterface") {
            None
        } else {
            tunnel_owner(
                &field_string(item, "alias"),
                &field_string(item, "description"),
                field_i64(item, "interfaceType"),
            )
        };
        let owner = owner.map(|name| if name == PROXY_OWNER { proxy_product } else { name });
        if let Some(map) = item.as_object_mut() {
            map.insert("managedBy".into(), owner.map(Value::from).unwrap_or(Value::Null));
        }
    }
}

/// Routes that proxy TUN software uses to capture all traffic or fake-IP DNS.
pub fn is_capture_prefix(destination: &str) -> bool {
    matches!(
        destination.trim().to_ascii_lowercase().as_str(),
        "0.0.0.0/1" | "128.0.0.0/1" | "198.18.0.0/15" | "::/1" | "8000::/1"
    )
}

/// True for answers from proxy fake-IP DNS (198.18.0.0/15).
pub fn is_fake_ip(address: &str) -> bool {
    address
        .parse::<std::net::Ipv4Addr>()
        .map(|ip| {
            let [a, b, ..] = ip.octets();
            a == 198 && (b == 18 || b == 19)
        })
        .unwrap_or(false)
}

/// The owner and alias of a protected interface, by index.
pub fn protected_owner(overview: &Value, index: i64) -> Option<(String, String)> {
    arrays(overview.get("interfaces"))
        .into_iter()
        .find(|item| field_i64(item, "interfaceIndex") == index)
        .and_then(|item| owner_of(&item).map(|owner| (field_string(&item, "alias"), owner)))
}

pub fn owner_of(adapter: &Value) -> Option<String> {
    adapter
        .get("managedBy")
        .and_then(Value::as_str)
        .filter(|owner| !owner.is_empty())
        .map(str::to_owned)
}

/// Mark routes that belong to protected interfaces or capture traffic.
pub fn annotate_routes(routes: &mut Value, overview: &Value) {
    let owners: HashMap<i64, String> = arrays(overview.get("interfaces"))
        .into_iter()
        .filter_map(|item| owner_of(&item).map(|owner| (field_i64(&item, "interfaceIndex"), owner)))
        .collect();
    let Some(items) = routes.get_mut("routes").and_then(Value::as_array_mut) else {
        return;
    };
    for route in items {
        let owner = owners.get(&field_i64(route, "interfaceIndex")).cloned();
        let capture = is_capture_prefix(&field_string(route, "destinationPrefix"));
        if let Some(map) = route.as_object_mut() {
            map.insert("managedBy".into(), owner.map(Value::from).unwrap_or(Value::Null));
            map.insert("captureRoute".into(), json!(capture));
        }
    }
}

/// The protected adapter that currently carries default IPv4 traffic, if any:
/// split `/1` routes always win; otherwise the lowest effective `/0` Metric.
pub fn default_via(overview: &Value, routes: &Value) -> Option<Value> {
    let items = arrays(routes.get("routes"));
    let ipv4 = |route: &&Value| field_string(route, "addressFamily") == "IPv4";
    let split = items
        .iter()
        .filter(ipv4)
        .find(|route| matches!(field_string(route, "destinationPrefix").as_str(), "0.0.0.0/1" | "128.0.0.0/1"));
    let winner = split.or_else(|| {
        items
            .iter()
            .filter(ipv4)
            .filter(|route| field_string(route, "destinationPrefix") == "0.0.0.0/0")
            .min_by_key(|route| {
                route
                    .get("effectiveMetric")
                    .and_then(Value::as_i64)
                    .unwrap_or_else(|| field_i64(route, "routeMetric"))
            })
    })?;
    let index = field_i64(winner, "interfaceIndex");
    let (alias, owner) = protected_owner(overview, index)?;
    Some(json!({"interfaceIndex":index,"alias":alias,"owner":owner,"splitRoutes":split.is_some()}))
}

/// Summary for the "Proxy & VPN software" card.
pub fn status(overview: &Value, routes: &Value, software: Vec<Value>, system_proxy: Value) -> Value {
    let tunnels = arrays(overview.get("interfaces"))
        .into_iter()
        .filter_map(|item| {
            owner_of(&item).map(|owner| {
                let ipv4 = arrays(item.get("ipv4Addresses"))
                    .first()
                    .map(|address| field_string(address, "ipAddress"));
                json!({
                    "interfaceIndex":field_i64(&item,"interfaceIndex"),
                    "alias":field_string(&item,"alias"),
                    "owner":owner,
                    "up":field_string(&item,"operationalStatus").eq_ignore_ascii_case("up"),
                    "ipv4":ipv4
                })
            })
        })
        .collect::<Vec<_>>();
    json!({
        "software":software,
        "systemProxy":system_proxy,
        "tunnels":tunnels,
        "defaultVia":default_via(overview, routes)
    })
}

// ---------------------------------------------------------------------------
// Windows reads
// ---------------------------------------------------------------------------

/// Running process names by PID (`xray.exe`), from a ToolHelp snapshot.
pub fn running_processes() -> HashMap<u32, String> {
    let mut result = HashMap::new();
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
        return result;
    }
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while ok {
        let end = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
        result.insert(entry.th32ProcessID, String::from_utf16_lossy(&entry.szExeFile[..end]));
        ok = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe {
        CloseHandle(snapshot);
    }
    result
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

const INTERNET_SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

fn registry_dword(name: &str) -> Option<u32> {
    let (key, value) = (wide(INTERNET_SETTINGS), wide(name));
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            (&mut data as *mut u32).cast(),
            &mut size,
        )
    };
    (code == 0).then_some(data)
}

fn registry_string(name: &str) -> Option<String> {
    let (key, value) = (wide(INTERNET_SETTINGS), wide(name));
    let mut buffer = vec![0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
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
    Some(String::from_utf16_lossy(&buffer[..end]).chars().take(256).collect())
}

/// The current user's Windows system proxy (read-only).
pub fn system_proxy() -> Value {
    json!({
        "enabled":registry_dword("ProxyEnable").unwrap_or(0) != 0,
        "server":registry_string("ProxyServer").unwrap_or_default(),
        "autoConfigUrl":registry_string("AutoConfigURL").unwrap_or_default()
    })
}

/// Live summary: running software, system proxy and protected adapters.
pub fn live_status(overview: &Value, routes: &Value) -> Value {
    let processes = running_processes();
    let software = detect_software(processes.values().map(String::as_str));
    status(overview, routes, software, system_proxy())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owners_are_detected_from_real_adapter_strings() {
        assert_eq!(tunnel_owner("et_6_0qmn", "et_6_0qmn Tunnel", 53), Some("EasyTier"));
        assert_eq!(tunnel_owner("vgate0", "Rust Wintun Tunnel Tunnel", 53), Some("tunnel"));
        assert_eq!(tunnel_owner("本地连接", "Wintun Userspace Tunnel", 53), Some("tunnel"));
        assert_eq!(tunnel_owner("singbox_tun", "sing-box Tunnel", 53), Some(PROXY_OWNER));
        assert_eq!(tunnel_owner("Meta", "Meta Tunnel", 53), Some(PROXY_OWNER));
        assert_eq!(tunnel_owner("Tailscale", "Tailscale Tunnel", 53), Some("Tailscale"));
        assert_eq!(tunnel_owner("以太网", "Realtek PCIe GbE Family Controller", 6), None);
        assert_eq!(tunnel_owner("WLAN", "Intel(R) Wi-Fi 6 AX201 160MHz", 71), None);
        assert_eq!(tunnel_owner("vEthernet (FSE HostVnic)", "Hyper-V Virtual Ethernet Container Adapter", 6), None);
        assert_eq!(tunnel_owner("以太网 4", "VirtualBox Host-Only Ethernet Adapter", 6), None);
        // An "et_" name alone is not enough without the Wintun interface type.
        assert_eq!(tunnel_owner("et_office", "Realtek USB GbE", 6), None);
    }

    #[test]
    fn software_is_grouped_by_product() {
        let found = detect_software(["v2rayN.exe", "xray.exe", "easytier-gui.exe", "explorer.exe", "xray.exe"]);
        assert_eq!(
            found,
            vec![
                json!({"product":"EasyTier","processes":["easytier-gui"]}),
                json!({"product":"v2rayN","processes":["v2rayN","xray"]}),
            ]
        );
        assert_eq!(proxy_product(&found), "v2rayN");
        let core_only = detect_software(["sing-box.exe"]);
        assert_eq!(core_only, vec![json!({"product":"sing-box","processes":["sing-box"]})]);
        assert_eq!(proxy_product(&core_only), "sing-box");
        assert_eq!(proxy_product(&[]), "Proxy TUN");
    }

    fn overview() -> Value {
        json!({"interfaces":[
            {"interfaceIndex":23,"alias":"以太网","description":"Realtek PCIe GbE Family Controller","interfaceType":6,"systemInterface":false,"operationalStatus":"Up"},
            {"interfaceIndex":33,"alias":"et_6_0qmn","description":"et_6_0qmn Tunnel","interfaceType":53,"systemInterface":false,"operationalStatus":"Up","ipv4Addresses":[{"ipAddress":"10.126.126.5"}]},
            {"interfaceIndex":40,"alias":"singbox_tun","description":"sing-box Tunnel","interfaceType":53,"systemInterface":false,"operationalStatus":"Up"},
            {"interfaceIndex":2,"alias":"本地连接* 2","description":"WAN Miniport (SSTP)","interfaceType":131,"systemInterface":true}
        ]})
    }

    #[test]
    fn annotation_marks_protected_interfaces_and_routes() {
        let mut value = overview();
        annotate_overview(&mut value, "v2rayN");
        let owners = arrays(value.get("interfaces")).iter().map(|item| item["managedBy"].clone()).collect::<Vec<_>>();
        assert_eq!(owners, vec![Value::Null, json!("EasyTier"), json!("v2rayN"), Value::Null]);
        assert_eq!(protected_owner(&value, 33), Some(("et_6_0qmn".into(), "EasyTier".into())));
        assert_eq!(protected_owner(&value, 23), None);
        let mut routes = json!({"routes":[
            {"addressFamily":"IPv4","destinationPrefix":"10.126.126.0/24","interfaceIndex":33},
            {"addressFamily":"IPv4","destinationPrefix":"0.0.0.0/1","interfaceIndex":40},
            {"addressFamily":"IPv4","destinationPrefix":"10.0.0.0/8","interfaceIndex":23}
        ]});
        annotate_routes(&mut routes, &value);
        assert_eq!(routes["routes"][0]["managedBy"], "EasyTier");
        assert_eq!(routes["routes"][1]["captureRoute"], true);
        assert!(routes["routes"][2]["managedBy"].is_null());
        assert_eq!(routes["routes"][2]["captureRoute"], false);
    }

    #[test]
    fn default_path_through_a_tunnel_is_reported() {
        let mut value = overview();
        annotate_overview(&mut value, "v2rayN");
        let physical = json!({"addressFamily":"IPv4","destinationPrefix":"0.0.0.0/0","interfaceIndex":23,"effectiveMetric":25});
        let plain = json!({"routes":[physical.clone()]});
        assert!(default_via(&value, &plain).is_none());
        let tun = json!({"routes":[physical.clone(),
            {"addressFamily":"IPv4","destinationPrefix":"0.0.0.0/1","interfaceIndex":40,"effectiveMetric":9000}]});
        let via = default_via(&value, &tun).unwrap();
        assert_eq!(via["owner"], "v2rayN");
        assert_eq!(via["splitRoutes"], true);
        let exit_node = json!({"routes":[physical,
            {"addressFamily":"IPv4","destinationPrefix":"0.0.0.0/0","interfaceIndex":33,"effectiveMetric":5}]});
        assert_eq!(default_via(&value, &exit_node).unwrap()["alias"], "et_6_0qmn");
        let summary = status(&value, &tun, vec![], json!({"enabled":true,"server":"127.0.0.1:8889"}));
        assert_eq!(summary["tunnels"].as_array().unwrap().len(), 2);
        assert_eq!(summary["tunnels"][0]["ipv4"], "10.126.126.5");
        assert_eq!(summary["systemProxy"]["server"], "127.0.0.1:8889");
    }

    #[test]
    fn capture_and_fake_ip_ranges() {
        for prefix in ["0.0.0.0/1", "128.0.0.0/1", "198.18.0.0/15", "::/1"] {
            assert!(is_capture_prefix(prefix), "{prefix}");
        }
        assert!(!is_capture_prefix("0.0.0.0/0"));
        assert!(!is_capture_prefix("10.0.0.0/8"));
        assert!(is_fake_ip("198.18.0.12"));
        assert!(is_fake_ip("198.19.255.1"));
        assert!(!is_fake_ip("198.20.0.1"));
        assert!(!is_fake_ip("not-an-ip"));
    }

    #[test]
    #[ignore = "manual, read-only check of local processes and proxy settings"]
    fn read_only_local_coexistence_reads() {
        let processes = running_processes();
        assert!(!processes.is_empty());
        println!("{:?}", detect_software(processes.values().map(String::as_str)));
        println!("{}", system_proxy());
    }
}
