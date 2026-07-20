const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const iconv = require('iconv-lite');
const { parseWifiInterfaces, parseWifiProfiles } = require('../src/parsers/netsh-wlan-parser');
const { decodeWindowsOutput } = require('../src/parsers/windows-output-decoder');

const fixture = (name) => fs.readFileSync(path.join(__dirname, 'fixtures', name), 'utf8');

test('parses English multi-adapter output, spaces and disconnected state', () => {
  const result = parseWifiInterfaces(fixture('netsh-wlan-interfaces-en.txt'));
  assert.equal(result.length, 2);
  assert.equal(result[0].name, 'Wi-Fi Main');
  assert.equal(result[0].ssid, 'Office WiFi 5G');
  assert.equal(result[0].radioType, '802.11ax');
  assert.equal(result[1].connected, false);
});

test('parses Chinese aliases and missing optional fields without dropping adapter', () => {
  const result = parseWifiInterfaces(fixture('netsh-wlan-interfaces-zh.txt'));
  assert.equal(result.length, 2);
  assert.equal(result[0].description, 'Intel 无线网卡');
  assert.equal(result[0].ssid, '单位 网络');
  assert.equal(result[1].guid, undefined);
});

test('parses and de-duplicates English and Chinese profiles', () => {
  assert.deepEqual(parseWifiProfiles(fixture('netsh-wlan-profiles-en.txt')), ['Office WiFi 5G', 'Home WiFi']);
  assert.deepEqual(parseWifiProfiles(fixture('netsh-wlan-profiles-zh.txt')), ['单位 网络', '家庭网络']);
});

test('decodes UTF-8 and CP936 Windows output', () => {
  const text = '名称 : WLAN\r\nSSID : 中文网络';
  assert.equal(decodeWindowsOutput(Buffer.from(text, 'utf8')), text);
  assert.equal(decodeWindowsOutput(iconv.encode(text, 'gbk'), [936]), text);
});
