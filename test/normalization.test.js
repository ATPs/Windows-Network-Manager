const test = require('node:test');
const assert = require('node:assert/strict');
const { normalizeNetworkOverview } = require('../src/services/network-overview-service');
const { normalizeRoutes } = require('../src/services/route-service');

test('normalizes single/partial interface records and classifies capabilities', () => {
  const result = normalizeNetworkOverview({ adapters: [{ interfaceIndex: 7, alias: 'Ethernet', description: 'USB Ethernet', interfaceType: 6, hardwareInterface: true, virtual: false }], ipInterfaces: [{ interfaceIndex: 7, alias: 'Ethernet', addressFamily: 'IPv4', automaticMetric: false, interfaceMetric: 10, dhcp: true }], ipConfigurations: [], addresses: [], dns: [], connectionProfiles: [] });
  assert.equal(result.interfaces[0].type, 'Ethernet');
  assert.equal(result.interfaces[0].metrics.ipv4.value, 10);
  assert.equal(result.interfaces[0].ipv6Addresses.length, 0);
});

test('merges active and persistent route views and computes effective metric', () => {
  const route = { addressFamily: 'IPv4', destinationPrefix: '172.16.0.0/12', nextHop: '192.168.3.1', interfaceAlias: 'WLAN', interfaceIndex: 5, routeMetric: 5, protocol: 'NetMgmt', state: 'Alive', publish: 'No' };
  const overview = { interfaces: [{ interfaceIndex: 5, type: 'Wi-Fi', metrics: { ipv4: { value: 10 }, ipv6: null } }] };
  const result = normalizeRoutes({ active: [{ ...route, policyStore: 'ActiveStore' }], persistent: [{ ...route, protocol: '', policyStore: 'PersistentStore' }] }, overview);
  assert.equal(result.routes.length, 1);
  assert.equal(result.routes[0].persistent, true);
  assert.equal(result.routes[0].effectiveMetric, 15);
  assert.deepEqual(result.routes[0].policyStores.sort(), ['ActiveStore', 'PersistentStore']);
});
