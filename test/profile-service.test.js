const test = require('node:test');
const assert = require('node:assert/strict');
const { createProfileService } = require('../src/services/profile-service');
const { defaultConfig } = require('../src/config/config-store');

test('profile apply reports partial success instead of aborting after one failure', async () => {
  let config = defaultConfig();
  const store = {
    load: async () => ({ config, warning: null }),
    save: async (value) => { config = structuredClone(value); return config; },
  };
  const adapter = { interfaceIndex: 5, alias: 'WLAN', guid: 'guid-1', macAddress: 'AA', description: 'Wi-Fi', capabilities: { routes: true }, ipv4Addresses: [{ address: '192.168.3.2' }], metrics: { ipv4: { automatic: true, value: 30 }, ipv6: null }, defaultRoute: false, wifi: null };
  const overview = { interfaces: [adapter] };
  const service = createProfileService(store, {
    getNetworkOverview: async () => overview,
    getRoutes: async () => ({ routes: [] }),
    getPrivilegeStatus: async () => ({ isAdmin: true }),
    requireAdministrator: async () => undefined,
    setInterfaceMetric: async () => { throw new Error('mock metric failure'); },
    addRoute: async () => ({ created: true }),
  });
  const reference = { guid: 'guid-1', alias: 'WLAN', interfaceIndex: 5 };
  const saved = await service.saveProfile({
    name: '部分成功测试',
    wifiActions: [],
    metricActions: [{ interfaceRef: reference, addressFamily: 'IPv4', automatic: false, metric: 10 }],
    routeActions: [{ interfaceRef: reference, destinationPrefix: '198.51.100.0/24', nextHop: '192.168.3.1', routeMetric: 5, desiredState: 'present', managedByApp: true }],
  });
  const preview = await service.previewApplyProfile(saved.id);
  const result = await service.applyProfile({ profileId: saved.id, confirmationToken: preview.confirmationToken });
  assert.equal(result.counts.failed, 1);
  assert.equal(result.counts.success, 1);
  assert.match(result.results.find((item) => item.status === 'failed').reason, /mock metric failure/);
});
