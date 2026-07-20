const net = require('node:net');
const { ValidationError } = require('../utils/errors');

const CONTROL_CHARACTERS = /[\u0000-\u001F\u007F]/;

function validateSafeString(value, label, options = {}) {
  if (typeof value !== 'string') throw new ValidationError(`${label}必须是文本。`);
  const trimmed = value.trim();
  if (!trimmed) throw new ValidationError(`${label}不能为空。`);
  if (trimmed.length > (options.maxLength ?? 255)) throw new ValidationError(`${label}过长。`);
  if (CONTROL_CHARACTERS.test(value)) throw new ValidationError(`${label}包含控制字符。`);
  return trimmed;
}

function validateInterfaceIndex(value) {
  const number = typeof value === 'string' && /^\d+$/.test(value) ? Number(value) : value;
  if (!Number.isSafeInteger(number) || number <= 0 || number > 2147483647) {
    throw new ValidationError('InterfaceIndex 必须是有效正整数。');
  }
  return number;
}

function validateMetric(value) {
  const number = typeof value === 'string' && /^\d+$/.test(value) ? Number(value) : value;
  if (!Number.isInteger(number) || number < 1 || number > 9999) {
    throw new ValidationError('Metric 必须是 1 到 9999 的整数。');
  }
  return number;
}

function validateAddressFamily(value, allowed = ['IPv4', 'IPv6']) {
  if (!allowed.includes(value)) throw new ValidationError('地址族无效。');
  return value;
}

function validateIPv4(value, label = 'IPv4 地址') {
  const address = validateSafeString(value, label, { maxLength: 15 });
  if (net.isIP(address) !== 4) throw new ValidationError(`${label}无效。`);
  return address;
}

function validateIPv6(value, label = 'IPv6 地址') {
  const address = validateSafeString(value, label, { maxLength: 45 });
  if (net.isIP(address) !== 6) throw new ValidationError(`${label}无效。`);
  return address;
}

function normalizeIPv4Cidr(value) {
  const cidr = validateSafeString(value, '目标 CIDR', { maxLength: 18 });
  const match = cidr.match(/^([^/]+)\/(\d{1,2})$/);
  if (!match) throw new ValidationError('目标 CIDR 格式无效。');
  const address = validateIPv4(match[1], '目标 IPv4 地址');
  const prefixLength = Number(match[2]);
  if (prefixLength < 0 || prefixLength > 32) throw new ValidationError('IPv4 前缀长度必须为 0 到 32。');
  const parts = address.split('.').map(Number);
  const integer = (((parts[0] * 256 + parts[1]) * 256 + parts[2]) * 256 + parts[3]) >>> 0;
  const mask = prefixLength === 0 ? 0 : (0xFFFFFFFF << (32 - prefixLength)) >>> 0;
  const network = (integer & mask) >>> 0;
  const normalizedAddress = [24, 16, 8, 0].map((shift) => (network >>> shift) & 255).join('.');
  const normalized = `${normalizedAddress}/${prefixLength}`;
  return { input: cidr, normalized, wasNormalized: normalized !== cidr, prefixLength };
}

function validateMetricInput(input) {
  if (!input || typeof input !== 'object') throw new ValidationError('Metric 请求无效。');
  const automatic = input.automatic === true;
  return {
    interfaceIndex: validateInterfaceIndex(input.interfaceIndex),
    addressFamily: validateAddressFamily(input.addressFamily),
    automatic,
    metric: automatic ? null : validateMetric(input.metric),
  };
}

function validateRouteInput(input) {
  if (!input || typeof input !== 'object') throw new ValidationError('路由请求无效。');
  const destination = normalizeIPv4Cidr(input.destinationPrefix);
  return {
    destinationPrefix: destination.normalized,
    requestedDestinationPrefix: destination.input,
    destinationWasNormalized: destination.wasNormalized,
    nextHop: validateIPv4(input.nextHop, '下一跳'),
    interfaceIndex: validateInterfaceIndex(input.interfaceIndex),
    routeMetric: validateMetric(input.routeMetric),
    persistent: input.persistent === true,
    note: input.note ? validateSafeString(input.note, '备注', { maxLength: 500 }) : '',
  };
}

function validateWifiRequest(input) {
  if (!input || typeof input !== 'object') throw new ValidationError('Wi-Fi 请求无效。');
  return {
    interfaceName: validateSafeString(input.interfaceName, '网卡名称'),
    profileName: input.profileName === undefined ? undefined : validateSafeString(input.profileName, 'Wi-Fi profile'),
  };
}

function validateProfileName(value) {
  return validateSafeString(value, '方案名称', { maxLength: 100 });
}

module.exports = {
  normalizeIPv4Cidr,
  validateAddressFamily,
  validateInterfaceIndex,
  validateIPv4,
  validateIPv6,
  validateMetric,
  validateMetricInput,
  validateProfileName,
  validateRouteInput,
  validateSafeString,
  validateWifiRequest,
};
