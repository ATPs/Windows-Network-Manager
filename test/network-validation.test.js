const test = require('node:test');
const assert = require('node:assert/strict');
const { normalizeIPv4Cidr, validateIPv4, validateIPv6, validateInterfaceIndex, validateMetric, validateSafeString } = require('../src/validation/network-validation');

test('validates IPv4 and IPv6 addresses', () => {
  assert.equal(validateIPv4('192.168.3.1'), '192.168.3.1');
  assert.equal(validateIPv6('2001:db8::1'), '2001:db8::1');
  assert.throws(() => validateIPv4('300.1.1.1'));
  assert.throws(() => validateIPv6('not-ipv6'));
});

test('normalizes IPv4 CIDR network address', () => {
  assert.deepEqual(normalizeIPv4Cidr('172.31.9.10/12'), { input: '172.31.9.10/12', normalized: '172.16.0.0/12', wasNormalized: true, prefixLength: 12 });
  assert.equal(normalizeIPv4Cidr('0.0.0.0/0').normalized, '0.0.0.0/0');
  assert.throws(() => normalizeIPv4Cidr('172.16.0.0/33'));
});

test('validates Metric and InterfaceIndex as strict integers', () => {
  assert.equal(validateMetric('9999'), 9999);
  assert.equal(validateInterfaceIndex('23'), 23);
  for (const value of [0, -1, 1.5, '1.5', ' 5', 'abc', 10000]) assert.throws(() => validateMetric(value));
  for (const value of [0, -1, 1.5, '1 2', '']) assert.throws(() => validateInterfaceIndex(value));
});

test('rejects controls and overlong strings', () => {
  assert.throws(() => validateSafeString('WLAN\n2', '名称'));
  assert.throws(() => validateSafeString('a'.repeat(256), '名称'));
});
