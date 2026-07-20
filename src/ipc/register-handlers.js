const { ipcMain } = require('electron');
const { toPublicError, ValidationError } = require('../utils/errors');
const { validateWifiRequest } = require('../validation/network-validation');
const { getPrivilegeStatus, restartAsAdministrator } = require('../windows/privilege');
const { connectWifi, disconnectWifi, getWifiProfiles } = require('../windows/wifi');
const { previewMetricChange, setInterfaceMetric } = require('../services/metric-service');
const { getNetworkOverview } = require('../services/network-overview-service');
const { addRoute, getRoutes, previewAddRoute, previewRemoveRoute, removeRoute } = require('../services/route-service');
const { previewAdapterState, setAdapterState } = require('../services/adapter-control-service');
const { createConfigStore } = require('../config/config-store');
const { createProfileService } = require('../services/profile-service');

function registerHandlers(app) {
  const store = createConfigStore(app.getPath('userData'));
  const profiles = createProfileService(store);

  handle('system:get-privilege-status', () => getPrivilegeStatus());
  handle('system:restart-as-administrator', async () => {
    const args = app.isPackaged ? ['--restarted-as-admin'] : [app.getAppPath(), '--restarted-as-admin'];
    const result = await restartAsAdministrator({ filePath: process.execPath, arguments: args });
    if (result?.started) setTimeout(() => app.quit(), 500);
    return result;
  });

  handle('network:get-overview', () => getNetworkOverview());
  handle('wifi:get-profiles', async (request) => {
    const input = validateWifiRequest(request);
    return getWifiProfiles(input.interfaceName);
  });
  handle('wifi:connect', async (request) => {
    const input = validateWifiRequest(request);
    if (!input.profileName) throw new ValidationError('必须选择 Wi-Fi profile。');
    await connectWifi(input.interfaceName, input.profileName);
    return { message: `已向“${input.interfaceName}”发送连接请求。` };
  });
  handle('wifi:disconnect', async (request) => {
    const input = validateWifiRequest(request);
    await disconnectWifi(input.interfaceName);
    return { message: `已向“${input.interfaceName}”发送断开请求。` };
  });

  handle('metric:preview', (request) => previewMetricChange(request));
  handle('metric:set', (request) => setInterfaceMetric(request));
  handle('routes:get', async () => {
    const [data, loaded] = await Promise.all([getRoutes(), store.load()]);
    return { ...data, routes: data.routes.map((route) => ({ ...route, note: loaded.config.routeNotes[routeNoteKey(route)] || '' })) };
  });
  handle('routes:preview-add', (request) => previewAddRoute(request));
  handle('routes:add', async (request) => {
    const result = await addRoute(request);
    if (request?.route?.note) {
      const loaded = await store.load();
      loaded.config.routeNotes[routeNoteKey({ addressFamily: 'IPv4', ...request.route })] = request.route.note;
      await store.save(loaded.config);
    }
    return result;
  });
  handle('routes:preview-remove', (request) => previewRemoveRoute(request));
  handle('routes:remove', async (request) => {
    const result = await removeRoute(request);
    const loaded = await store.load();
    delete loaded.config.routeNotes[routeNoteKey(request.route)];
    await store.save(loaded.config);
    return result;
  });
  handle('adapter:preview-state', (request) => previewAdapterState(request));
  handle('adapter:set-state', (request) => setAdapterState(request));

  handle('profiles:list', () => profiles.listProfiles());
  handle('profiles:save', (request) => profiles.saveProfile(request));
  handle('profiles:delete', (request) => profiles.deleteProfile(request?.profileId));
  handle('profiles:preview-apply', (request) => profiles.previewApplyProfile(request?.profileId));
  handle('profiles:apply', (request) => profiles.applyProfile(request));
  handle('settings:save', async (request) => {
    const loaded = await store.load();
    const settings = {
      showSystemInterfaces: request?.showSystemInterfaces === true,
      showAllRoutes: request?.showAllRoutes === true,
    };
    await store.save({ ...loaded.config, settings });
    return settings;
  });
}

function handle(channel, operation) {
  ipcMain.handle(channel, async (_event, request) => {
    try {
      return { ok: true, data: await operation(request) };
    } catch (error) {
      console.error(`${channel} failed:`, error);
      return { ok: false, error: toPublicError(error) };
    }
  });
}

function routeNoteKey(route) {
  return `${route.addressFamily || 'IPv4'}|${route.destinationPrefix}|${route.nextHop}|${route.interfaceIndex}`;
}

module.exports = { registerHandlers };
