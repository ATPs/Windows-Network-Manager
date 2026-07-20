const test = require('node:test');
const assert = require('node:assert/strict');
const { buildProfilePlan } = require('../src/services/profile-planner');

const adapter = { interfaceIndex: 5, alias: 'WLAN', guid: 'guid-1', macAddress: 'AA', description: 'Wi-Fi', metrics: { ipv4: { automatic: true, value: 30 }, ipv6: { automatic: true, value: 30 } }, defaultRoute: true, wifi: { connected: true, currentProfile: 'Office', profiles: ['Office', 'Home'] } };
const ref = { guid: 'guid-1', alias: 'old-name', interfaceIndex: 99 };

test('planner skips satisfied work and uses GUID before stale alias/index', () => {
  const plan = buildProfilePlan({ id: 'p1', name: 'Office', wifiActions: [{ interfaceRef: ref, action: 'connect', profileName: 'Office' }], metricActions: [], routeActions: [] }, { overview: { interfaces: [adapter] }, routes: { routes: [] }, isAdmin: false });
  assert.equal(plan.operations[0].status, 'skipped');
  assert.equal(plan.operations[0].payload.interfaceIndex, 5);
});

test('planner reports missing interface and Wi-Fi profile', () => {
  const profile = { id: 'p', name: 'x', wifiActions: [{ interfaceRef: ref, action: 'connect', profileName: 'Missing' }, { interfaceRef: { alias: 'gone' }, action: 'disconnect' }], metricActions: [], routeActions: [] };
  const plan = buildProfilePlan(profile, { overview: { interfaces: [adapter] }, routes: { routes: [] }, isAdmin: true });
  assert.deepEqual(plan.operations.map((x) => x.status), ['failed', 'failed']);
});

test('planner requires admin for Metric and marks route conflicts', () => {
  const profile = { id: 'p', name: 'x', wifiActions: [], metricActions: [{ interfaceRef: ref, addressFamily: 'IPv4', automatic: false, metric: 10 }], routeActions: [{ interfaceRef: ref, destinationPrefix: '172.16.0.0/12', nextHop: '192.168.3.1', desiredState: 'present', managedByApp: true }] };
  const route = { destinationPrefix: '172.16.0.0/12', nextHop: '192.168.4.1', interfaceIndex: 8 };
  const noAdmin = buildProfilePlan(profile, { overview: { interfaces: [adapter] }, routes: { routes: [route] }, isAdmin: false });
  assert.equal(noAdmin.operations[0].status, 'failed');
  const admin = buildProfilePlan(profile, { overview: { interfaces: [adapter] }, routes: { routes: [route] }, isAdmin: true });
  assert.equal(admin.operations[1].status, 'pending');
  assert.equal(admin.operations[1].warnings.length, 1);
});

test('planner refuses unmanaged route deletion and skips absent managed route', () => {
  const actions = [{ interfaceRef: ref, destinationPrefix: '10.0.0.0/8', nextHop: '1.1.1.1', desiredState: 'absent', managedByApp: false }, { interfaceRef: ref, destinationPrefix: '10.1.0.0/16', nextHop: '1.1.1.1', desiredState: 'absent', managedByApp: true }];
  const plan = buildProfilePlan({ id: 'p', name: 'x', wifiActions: [], metricActions: [], routeActions: actions }, { overview: { interfaces: [adapter] }, routes: { routes: [] }, isAdmin: true });
  assert.deepEqual(plan.operations.map((x) => x.status), ['failed', 'skipped']);
});
