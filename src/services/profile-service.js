const crypto = require('node:crypto');
const { ConflictError, NotFoundError, ValidationError } = require('../utils/errors');
const { normalizeIPv4Cidr, validateAddressFamily, validateMetric, validateProfileName, validateSafeString } = require('../validation/network-validation');
const { getPrivilegeStatus, requireAdministrator } = require('../windows/privilege');
const { connectWifi, disconnectWifi } = require('../windows/wifi');
const { setInterfaceMetric } = require('../windows/ip-interfaces');
const { addRoute, removeRoute } = require('../windows/routes');
const { getNetworkOverview } = require('./network-overview-service');
const { getRoutes } = require('./route-service');
const { consumeConfirmation, issueConfirmation } = require('./confirmation-service');
const { buildProfilePlan } = require('./profile-planner');

function createProfileService(store, dependencies = {}) {
  const deps = { getNetworkOverview, getRoutes, getPrivilegeStatus, requireAdministrator, connectWifi, disconnectWifi, setInterfaceMetric, addRoute, removeRoute, ...dependencies };

  async function listProfiles() {
    const loaded = await store.load();
    return { profiles: loaded.config.profiles, warning: loaded.warning, settings: loaded.config.settings };
  }

  async function saveProfile(input) {
    const profile = validateNetworkProfile(input);
    const loaded = await store.load();
    const now = new Date().toISOString();
    const existing = loaded.config.profiles.find((item) => item.id === profile.id);
    const saved = { ...profile, id: profile.id || crypto.randomUUID(), createdAt: existing?.createdAt || now, updatedAt: now };
    const profiles = existing
      ? loaded.config.profiles.map((item) => item.id === saved.id ? saved : item)
      : [...loaded.config.profiles, saved];
    await store.save({ ...loaded.config, profiles });
    return saved;
  }

  async function deleteProfile(profileId) {
    const id = validateSafeString(profileId, '方案 ID', { maxLength: 100 });
    const loaded = await store.load();
    if (!loaded.config.profiles.some((item) => item.id === id)) throw new NotFoundError('方案不存在。');
    await store.save({ ...loaded.config, profiles: loaded.config.profiles.filter((item) => item.id !== id) });
    return { deleted: true };
  }

  async function previewApplyProfile(profileId) {
    const profile = await getProfile(profileId);
    const [overview, routes, privilege] = await Promise.all([deps.getNetworkOverview(), deps.getRoutes(), deps.getPrivilegeStatus()]);
    const plan = buildProfilePlan(profile, { overview, routes, isAdmin: privilege.isAdmin });
    return { ...plan, confirmationToken: plan.requiresConfirmation ? issueConfirmation('apply-profile', { profileId: profile.id, updatedAt: profile.updatedAt }) : null };
  }

  async function applyProfile(input) {
    const profile = await getProfile(input.profileId);
    consumeConfirmation(input.confirmationToken, 'apply-profile', { profileId: profile.id, updatedAt: profile.updatedAt });
    const [overview, routes, privilege] = await Promise.all([deps.getNetworkOverview(), deps.getRoutes(), deps.getPrivilegeStatus()]);
    const plan = buildProfilePlan(profile, { overview, routes, isAdmin: privilege.isAdmin });
    const results = [];
    for (const item of plan.operations) {
      if (item.status !== 'pending') { results.push(item); continue; }
      try {
        if (item.type === 'wifi') await applyWifi(item.payload);
        if (item.type === 'metric') { await deps.requireAdministrator(); await deps.setInterfaceMetric(item.payload); }
        if (item.type === 'route') { await deps.requireAdministrator(); await applyRoute(item.payload); }
        results.push({ ...item, status: 'success' });
      } catch (error) {
        results.push({ ...item, status: 'failed', reason: error.message });
      }
    }
    const actualState = await deps.getNetworkOverview();
    return { profileId: profile.id, results, counts: countResults(results), actualState };
  }

  async function applyWifi(payload) {
    if (payload.action === 'connect') {
      await deps.connectWifi(payload.alias, payload.profileName);
      await poll(async () => {
        const state = await deps.getNetworkOverview();
        const adapter = state.interfaces.find((item) => item.interfaceIndex === payload.interfaceIndex);
        return adapter?.wifi?.connected && adapter.wifi.currentProfile === payload.profileName;
      });
    } else {
      await deps.disconnectWifi(payload.alias);
      await poll(async () => {
        const state = await deps.getNetworkOverview();
        return !state.interfaces.find((item) => item.interfaceIndex === payload.interfaceIndex)?.wifi?.connected;
      });
    }
  }

  async function applyRoute(payload) {
    if (payload.desiredState === 'present') {
      await poll(async () => {
        const state = await deps.getNetworkOverview();
        return (state.interfaces.find((item) => item.interfaceIndex === payload.interfaceIndex)?.ipv4Addresses.length || 0) > 0;
      });
      await deps.addRoute({ ...payload, persistent: payload.persistent === true });
    } else if (payload.existingRoute) {
      for (const policyStore of payload.existingRoute.policyStores || ['ActiveStore']) {
        await deps.removeRoute({ ...payload.existingRoute, policyStore });
      }
    }
  }

  async function getProfile(profileId) {
    const id = validateSafeString(profileId, '方案 ID', { maxLength: 100 });
    const loaded = await store.load();
    const profile = loaded.config.profiles.find((item) => item.id === id);
    if (!profile) throw new NotFoundError('方案不存在。');
    return profile;
  }

  return { applyProfile, deleteProfile, listProfiles, previewApplyProfile, saveProfile };
}

function validateNetworkProfile(input) {
  if (!input || typeof input !== 'object') throw new ValidationError('网络方案无效。');
  return {
    id: input.id ? validateSafeString(input.id, '方案 ID', { maxLength: 100 }) : null,
    name: validateProfileName(input.name),
    note: input.note ? validateSafeString(input.note, '方案备注', { maxLength: 1000 }) : '',
    wifiActions: (input.wifiActions || []).map((item) => ({ interfaceRef: validateInterfaceRef(item.interfaceRef), action: ['connect', 'disconnect', 'unchanged'].includes(item.action) ? item.action : 'unchanged', profileName: item.action === 'connect' ? validateSafeString(item.profileName, 'Wi-Fi profile') : null })),
    metricActions: (input.metricActions || []).map((item) => ({ interfaceRef: validateInterfaceRef(item.interfaceRef), addressFamily: validateAddressFamily(item.addressFamily), automatic: item.automatic === true, metric: item.automatic === true ? null : validateMetric(item.metric) })),
    routeActions: (input.routeActions || []).map((item) => {
      const destination = normalizeIPv4Cidr(item.destinationPrefix);
      return { id: item.id || crypto.randomUUID(), interfaceRef: validateInterfaceRef(item.interfaceRef), destinationPrefix: destination.normalized, nextHop: require('../validation/network-validation').validateIPv4(item.nextHop, '下一跳'), routeMetric: validateMetric(item.routeMetric), persistent: item.persistent === true, desiredState: item.desiredState === 'absent' ? 'absent' : 'present', managedByApp: item.managedByApp === true, note: item.note ? validateSafeString(item.note, '路由备注', { maxLength: 500 }) : '' };
    }),
  };
}

function validateInterfaceRef(value) {
  if (!value || typeof value !== 'object') throw new ValidationError('方案接口引用无效。');
  const clean = (entry, label, maxLength) => entry ? validateSafeString(String(entry), label, { maxLength }) : null;
  const interfaceIndex = value.interfaceIndex === null || value.interfaceIndex === undefined ? null : require('../validation/network-validation').validateInterfaceIndex(value.interfaceIndex);
  if (!value.guid && !value.alias && !interfaceIndex) throw new ValidationError('方案接口引用缺少标识。');
  return { guid: clean(value.guid, '接口 GUID', 64), macAddress: clean(value.macAddress, 'MAC 地址', 32), description: clean(value.description, '接口描述', 255), alias: clean(value.alias, '接口名称', 255), interfaceIndex };
}

async function poll(check, timeout = 30_000, interval = 1_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await check()) return true;
    await new Promise((resolve) => setTimeout(resolve, interval));
  }
  throw new ValidationError('等待网络状态更新超时。');
}

function countResults(items) { return items.reduce((out, item) => ({ ...out, [item.status]: (out[item.status] || 0) + 1 }), {}); }

module.exports = { createProfileService, poll, validateNetworkProfile };
