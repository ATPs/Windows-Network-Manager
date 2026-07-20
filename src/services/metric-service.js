const { NotFoundError, ValidationError } = require('../utils/errors');
const { validateMetricInput } = require('../validation/network-validation');
const { setInterfaceMetric: setMetricCommand } = require('../windows/ip-interfaces');
const { requireAdministrator } = require('../windows/privilege');
const { consumeConfirmation, issueConfirmation } = require('./confirmation-service');
const { getNetworkOverview } = require('./network-overview-service');

async function previewMetricChange(input) {
  const metric = validateMetricInput(input);
  const overview = await getNetworkOverview();
  const adapter = overview.interfaces.find((item) => item.interfaceIndex === metric.interfaceIndex);
  const familyKey = metric.addressFamily === 'IPv6' ? 'ipv6' : 'ipv4';
  if (!adapter || !adapter.metrics[familyKey] || !adapter.capabilities[`metric${metric.addressFamily}`]) throw new NotFoundError('接口不存在或不允许修改该地址族的 Metric。');
  const current = adapter.metrics[familyKey];
  const unchanged = current.automatic === metric.automatic && (metric.automatic || current.value === metric.metric);
  const warnings = [];
  if (adapter.defaultRoute) warnings.push('该接口承载默认网关，修改 Metric 可能改变默认网络出口。');
  return { input: metric, current, unchanged, warnings, confirmationToken: unchanged ? null : issueConfirmation('set-metric', metric) };
}

async function setInterfaceMetric(input) {
  const metric = validateMetricInput(input.metric);
  consumeConfirmation(input.confirmationToken, 'set-metric', metric);
  await requireAdministrator();
  await setMetricCommand(metric);
  const overview = await getNetworkOverview();
  const adapter = overview.interfaces.find((item) => item.interfaceIndex === metric.interfaceIndex);
  const current = adapter?.metrics[metric.addressFamily === 'IPv6' ? 'ipv6' : 'ipv4'];
  if (!current || current.automatic !== metric.automatic || (!metric.automatic && current.value !== metric.metric)) {
    throw new ValidationError('Metric 设置未通过重新读取验证。');
  }
  return { metric: current, interfaceIndex: metric.interfaceIndex, addressFamily: metric.addressFamily };
}

module.exports = { previewMetricChange, setInterfaceMetric };
