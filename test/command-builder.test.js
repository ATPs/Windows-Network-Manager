const test = require('node:test');
const assert = require('node:assert/strict');
const { buildPowerShellInvocation } = require('../src/windows/powershell');
const { buildConnectArgs, buildDisconnectArgs, buildProfilesArgs } = require('../src/windows/wifi');

test('PowerShell invocation uses fixed encoded script and environment data', () => {
  const input = { interfaceIndex: 5, destinationPrefix: '172.16.0.0/12' };
  const invocation = buildPowerShellInvocation('$InputData | ConvertTo-Json -Compress', input);
  assert.equal(invocation.file, 'powershell.exe');
  assert.ok(invocation.args.includes('-NoProfile'));
  assert.ok(invocation.args.includes('-NonInteractive'));
  assert.ok(invocation.args.includes('-EncodedCommand'));
  assert.equal(invocation.args.some((arg) => arg.includes(input.destinationPrefix)), false);
  assert.deepEqual(JSON.parse(Buffer.from(invocation.env.WNM_INPUT_BASE64, 'base64').toString('utf8')), input);
});

test('netsh commands are represented as argument arrays', () => {
  assert.deepEqual(buildProfilesArgs('WLAN 2'), ['wlan', 'show', 'profiles', 'interface=WLAN 2']);
  assert.deepEqual(buildConnectArgs('WLAN 2', '单位 网络'), ['wlan', 'connect', 'name=单位 网络', 'interface=WLAN 2']);
  assert.deepEqual(buildDisconnectArgs('WLAN 2'), ['wlan', 'disconnect', 'interface=WLAN 2']);
});

test('process runner explicitly disables shell execution', () => {
  const source = require('node:fs').readFileSync(require.resolve('../src/windows/process-runner'), 'utf8');
  assert.match(source, /shell:\s*false/);
  assert.doesNotMatch(source, /\bexec\s*\(/);
});
