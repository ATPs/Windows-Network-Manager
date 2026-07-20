const { getRawNetworkOverview } = require('../windows/adapters');
const { getWifiOverview } = require('../windows/wifi');
const { optionalString, uniqueStrings } = require('../utils/normalize');

async function getNetworkOverview() {
  const [raw, wifiResult] = await Promise.all([getRawNetworkOverview(), getWifiOverview().catch((error) => ({ interfaces: [], warnings: ['Wi-Fi 信息读取失败。'], error }))]);
  return normalizeNetworkOverview(raw, wifiResult);
}

function normalizeNetworkOverview(raw, wifiResult = { interfaces: [], warnings: [] }) {
  const records = new Map();
  const ensure = (index) => {
    const key = Number(index);
    if (!records.has(key)) records.set(key, { interfaceIndex: key });
    return records.get(key);
  };

  for (const adapter of raw.adapters || []) Object.assign(ensure(adapter.interfaceIndex), adapter, { adapterPresent: true });
  for (const item of raw.ipInterfaces || []) {
    const record = ensure(item.interfaceIndex);
    record.alias ||= item.alias;
    record.ipInterfaces ||= {};
    record.ipInterfaces[item.addressFamily] = item;
  }
  for (const config of raw.ipConfigurations || []) {
    const record = ensure(config.interfaceIndex);
    record.alias ||= config.alias;
    record.ipv4Gateways = uniqueStrings(config.ipv4Gateways);
    record.ipv6Gateways = uniqueStrings(config.ipv6Gateways);
  }
  for (const address of raw.addresses || []) {
    const record = ensure(address.interfaceIndex);
    const key = address.addressFamily === 'IPv6' ? 'ipv6Addresses' : 'ipv4Addresses';
    record[key] ||= [];
    record[key].push({ address: address.ipAddress, prefixLength: address.prefixLength, state: address.addressState, type: address.type });
  }
  for (const dns of raw.dns || []) {
    const record = ensure(dns.interfaceIndex);
    record.dnsServers ||= [];
    record.dnsServers.push(...uniqueStrings(dns.servers));
  }
  for (const profile of raw.connectionProfiles || []) {
    const record = ensure(profile.interfaceIndex);
    record.networkProfile = profile;
  }

  const wifiByAlias = new Map((wifiResult.interfaces || []).map((item) => [item.name, item]));
  const interfaces = [...records.values()].filter((record) => Number.isInteger(record.interfaceIndex) && record.interfaceIndex > 0).map((record) => {
    const wifi = wifiByAlias.get(record.alias);
    const type = classifyInterface(record, wifi);
    const systemInterface = isSystemInterface(record, type);
    const ipv4Metric = record.ipInterfaces?.IPv4;
    const ipv6Metric = record.ipInterfaces?.IPv6;
    return {
      interfaceIndex: record.interfaceIndex,
      alias: optionalString(record.alias) || `Interface ${record.interfaceIndex}`,
      description: optionalString(record.description),
      guid: optionalString(record.guid)?.replace(/[{}]/g, '') || null,
      macAddress: optionalString(record.macAddress),
      type,
      hardwareInterface: record.hardwareInterface === true,
      virtual: record.virtual === true || ['Virtual', 'VPN', 'Tunnel'].includes(type),
      systemInterface,
      adminStatus: optionalString(record.adminStatus),
      operationalStatus: optionalString(record.operationalStatus) || optionalString(ipv4Metric?.connectionState),
      mediaConnectionState: optionalString(record.mediaConnectionState),
      linkSpeed: optionalString(record.linkSpeed),
      ipv4Addresses: record.ipv4Addresses || [],
      ipv6Addresses: record.ipv6Addresses || [],
      ipv4Gateways: record.ipv4Gateways || [],
      ipv6Gateways: record.ipv6Gateways || [],
      dnsServers: uniqueStrings(record.dnsServers),
      dhcpEnabled: ipv4Metric?.dhcp ?? null,
      metrics: {
        ipv4: ipv4Metric ? { automatic: ipv4Metric.automaticMetric === true, value: ipv4Metric.interfaceMetric } : null,
        ipv6: ipv6Metric ? { automatic: ipv6Metric.automaticMetric === true, value: ipv6Metric.interfaceMetric } : null,
      },
      networkCategory: optionalString(record.networkProfile?.category),
      defaultRoute: (record.ipv4Gateways?.length || 0) > 0 || (record.ipv6Gateways?.length || 0) > 0,
      wifi: wifi ? {
        connected: wifi.connected,
        state: wifi.state,
        ssid: wifi.ssid,
        signalPercent: parseSignal(wifi.signal),
        radioType: wifi.radioType,
        currentProfile: wifi.profile,
        profiles: wifi.profiles || [],
        profilesError: wifi.profilesError === true,
      } : null,
      capabilities: {
        wifi: Boolean(wifi),
        metricIPv4: Boolean(ipv4Metric) && !systemInterface,
        metricIPv6: Boolean(ipv6Metric) && !systemInterface,
        routes: Boolean(ipv4Metric || ipv6Metric),
        adapterControl: record.adapterPresent === true && !systemInterface && type !== 'Loopback',
      },
    };
  }).sort(interfaceSort);

  return { interfaces, warnings: wifiResult.warnings || [], refreshedAt: new Date().toISOString() };
}

function classifyInterface(record, wifi) {
  const text = `${record.alias || ''} ${record.description || ''}`.toLowerCase();
  if (Number(record.interfaceType) === 24 || text.includes('loopback')) return 'Loopback';
  if (wifi || Number(record.interfaceType) === 71) return 'Wi-Fi';
  if (/tailscale|wireguard|vpn|zerotier/.test(text)) return 'VPN';
  if ([53, 131].includes(Number(record.interfaceType)) || /tunnel|\btun\b|\btap\b|sing-tun/.test(text)) return 'Tunnel';
  if (record.virtual === true || /hyper-v|vmware|virtualbox|virtual adapter|easytier/.test(text)) return 'Virtual';
  if (Number(record.interfaceType) === 6 || record.hardwareInterface === true) return 'Ethernet';
  return 'Other';
}

function isSystemInterface(record, type) {
  const text = `${record.alias || ''} ${record.description || ''}`.toLowerCase();
  if (type === 'Loopback') return true;
  if (/wan miniport|kernel debug|isatap|teredo|6to4|ip-https platform|microsoft wi-fi direct virtual adapter|本地连接\*/.test(text)) return true;
  return false;
}

function parseSignal(value) {
  const match = String(value || '').match(/(\d{1,3})/);
  return match ? Math.min(100, Number(match[1])) : null;
}

function interfaceSort(a, b) {
  if (a.systemInterface !== b.systemInterface) return a.systemInterface ? 1 : -1;
  const order = ['Wi-Fi', 'Ethernet', 'VPN', 'Tunnel', 'Virtual', 'Other', 'Loopback'];
  return order.indexOf(a.type) - order.indexOf(b.type) || a.alias.localeCompare(b.alias, 'zh-CN');
}

module.exports = { classifyInterface, getNetworkOverview, isSystemInterface, normalizeNetworkOverview };
