const { NotFoundError, ValidationError } = require('../utils/errors');
const { validateInterfaceIndex } = require('../validation/network-validation');
const { setAdapterEnabled: setAdapterCommand } = require('../windows/adapter-control');
const { requireAdministrator } = require('../windows/privilege');
const { consumeConfirmation, issueConfirmation } = require('./confirmation-service');
const { getNetworkOverview } = require('./network-overview-service');

async function previewAdapterState(input) {
  const payload = { interfaceIndex: validateInterfaceIndex(input?.interfaceIndex), enabled: input?.enabled === true };
  const overview = await getNetworkOverview();
  const adapter = overview.interfaces.find((item) => item.interfaceIndex === payload.interfaceIndex);
  if (!adapter) throw new NotFoundError('网络适配器不存在。');
  if (!adapter.capabilities.adapterControl) throw new ValidationError('该系统接口不允许启用或禁用。');
  const currentlyEnabled = adapter.adminStatus !== 'Down' && adapter.adminStatus !== 'Disabled';
  const warnings = [];
  if (!payload.enabled) {
    warnings.push('禁用适配器可能中断当前网络或远程连接。');
    if (adapter.defaultRoute) warnings.push('该适配器当前承载默认网关。');
  }
  return { adapter, unchanged: currentlyEnabled === payload.enabled, warnings, confirmationToken: currentlyEnabled === payload.enabled ? null : issueConfirmation('adapter-state', payload) };
}

async function setAdapterState(input) {
  const payload = { interfaceIndex: validateInterfaceIndex(input?.interfaceIndex), enabled: input?.enabled === true };
  consumeConfirmation(input.confirmationToken, 'adapter-state', payload);
  await requireAdministrator();
  await setAdapterCommand(payload);
  return { requested: payload };
}

module.exports = { previewAdapterState, setAdapterState };
