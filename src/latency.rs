//! Latency sampling for Monitor: one small ICMP echo per interval to the
//! default gateway and the DNS servers of the physical adapter carrying
//! default traffic. Read-only. Adapters owned by tunnel/proxy software are
//! never used, and DNS servers outside the local network are pinged only
//! when the user opts in, so by default nothing leaves the local network.

use crate::{
    arrays, coexistence, field_bool, field_string,
    net_tools::{echo, IcmpHandle, IP_SUCCESS},
};
use serde_json::Value;
use std::net::Ipv4Addr;

#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    /// "gateway" or "dns".
    pub kind: &'static str,
    pub address: Ipv4Addr,
    pub source: Option<Ipv4Addr>,
    pub alias: String,
}

/// Private, link-local or carrier-grade NAT addresses stay on the local
/// network (or the ISP's), so pinging them never goes through a proxy.
pub fn is_local_network(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_private() || ip.is_link_local() || (a == 100 && (64..128).contains(&b))
}

fn usable(ip: Ipv4Addr) -> bool {
    !(ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() || coexistence::is_fake_ip(&ip.to_string()))
}

fn ipv4_list(value: Option<&Value>) -> Vec<Ipv4Addr> {
    arrays(value).iter().filter_map(Value::as_str).filter_map(|text| text.parse().ok()).collect()
}

/// Targets from the network overview: the up, non-system, non-tunnel adapter
/// with an IPv4 gateway and the lowest IPv4 interface Metric. A TUN that
/// captured default traffic with split routes does not change this choice.
pub fn select_targets(interfaces: &[Value], public_dns: bool) -> Vec<Target> {
    let Some(adapter) = interfaces
        .iter()
        .filter(|item| {
            field_string(item, "operationalStatus").eq_ignore_ascii_case("up")
                && !field_bool(item, "systemInterface")
                && coexistence::owner_of(item).is_none()
                && !ipv4_list(item.get("ipv4Gateways")).is_empty()
        })
        .min_by_key(|item| item.pointer("/metrics/ipv4/interfaceMetric").and_then(Value::as_i64).unwrap_or(i64::MAX))
    else {
        return vec![];
    };
    let alias = field_string(adapter, "alias");
    let source = arrays(adapter.get("ipv4Addresses"))
        .iter()
        .filter_map(|item| field_string(item, "ipAddress").parse::<Ipv4Addr>().ok())
        .find(|ip| !ip.is_link_local());
    let mut targets = vec![];
    if let Some(gateway) = ipv4_list(adapter.get("ipv4Gateways")).into_iter().find(|ip| usable(*ip)) {
        targets.push(Target { kind: "gateway", address: gateway, source, alias: alias.clone() });
    }
    for dns in ipv4_list(adapter.get("dnsServers")).into_iter().filter(|ip| usable(*ip) && (public_dns || is_local_network(*ip))) {
        if !targets.iter().any(|target| target.address == dns) && targets.len() < 4 {
            targets.push(Target { kind: "dns", address: dns, source, alias: alias.clone() });
        }
    }
    targets
}

/// One echo with a 1-second timeout; None when there is no reply.
pub fn ping_once(handle: &IcmpHandle, target: &Target) -> Option<u32> {
    let reply = echo(handle, target.source, target.address, 64, 1000);
    (reply.status == IP_SUCCESS).then(|| reply.rtt.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn adapter(alias: &str, metric: i64, gateway: &str, dns: &[&str], extra: Value) -> Value {
        let mut item = json!({
            "alias":alias,"operationalStatus":"Up","systemInterface":false,
            "ipv4Gateways":[gateway],"dnsServers":dns,
            "ipv4Addresses":[{"ipAddress":Ipv4Addr::new(192, 168, 1, 20).to_string()}],
            "metrics":{"ipv4":{"interfaceMetric":metric}},
        });
        for (key, value) in extra.as_object().unwrap() {
            item[key] = value.clone();
        }
        item
    }
    fn ip(a: u8, b: u8, c: u8, d: u8) -> String {
        Ipv4Addr::new(a, b, c, d).to_string()
    }

    #[test]
    fn picks_the_physical_adapter_with_the_lowest_metric() {
        let interfaces = vec![
            adapter("Ethernet", 25, &ip(192, 168, 1, 1), &[&ip(192, 168, 1, 1)], json!({})),
            adapter("Wi-Fi", 50, &ip(10, 0, 0, 1), &[], json!({})),
            adapter("EasyTier", 1, &ip(10, 126, 126, 1), &[], json!({"managedBy":"EasyTier"})),
        ];
        let targets = select_targets(&interfaces, false);
        assert_eq!(targets.len(), 1, "the gateway doubles as DNS and is pinged once");
        assert_eq!(targets[0].alias, "Ethernet");
        assert_eq!(targets[0].kind, "gateway");
        assert_eq!(targets[0].source, Some(Ipv4Addr::new(192, 168, 1, 20)));
    }

    #[test]
    fn public_dns_servers_need_an_opt_in() {
        let interfaces = vec![adapter("Ethernet", 25, &ip(192, 168, 1, 1), &[&ip(1, 1, 1, 1), &ip(192, 168, 1, 53), &ip(127, 0, 0, 1)], json!({}))];
        let local = select_targets(&interfaces, false);
        assert_eq!(local.iter().map(|t| t.address.to_string()).collect::<Vec<_>>(), vec![ip(192, 168, 1, 1), ip(192, 168, 1, 53)]);
        let all = select_targets(&interfaces, true);
        assert!(all.iter().any(|t| t.address.to_string() == ip(1, 1, 1, 1)));
        assert!(all.iter().all(|t| !t.address.is_loopback()));
    }

    #[test]
    fn fake_ip_dns_and_down_or_system_adapters_are_ignored() {
        let interfaces = vec![
            adapter("Ethernet", 25, &ip(192, 168, 1, 1), &[&ip(198, 18, 0, 2)], json!({})),
            adapter("Down", 1, &ip(10, 0, 0, 1), &[], json!({"operationalStatus":"Down"})),
            adapter("System", 1, &ip(10, 0, 0, 1), &[], json!({"systemInterface":true})),
        ];
        let targets = select_targets(&interfaces, true);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].alias, "Ethernet");
        assert!(select_targets(&[], false).is_empty());
    }

    #[test]
    fn local_network_ranges() {
        assert!(is_local_network(Ipv4Addr::new(10, 1, 2, 3)));
        assert!(is_local_network(Ipv4Addr::new(169, 254, 1, 1)));
        assert!(is_local_network(Ipv4Addr::new(100, 64, 0, 1)));
        assert!(!is_local_network(Ipv4Addr::new(8, 8, 8, 8)));
    }
}
