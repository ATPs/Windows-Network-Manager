const { NotFoundError } = require('../utils/errors');
const { decodeWindowsOutput } = require('../parsers/windows-output-decoder');
const { parseWifiInterfaces, parseWifiProfiles } = require('../parsers/netsh-wlan-parser');
const { runExecutable } = require('./process-runner');

async function runNetsh(args, options = {}) {
  const result = await runExecutable('netsh.exe', args, { ...options, commandName: 'netsh WLAN' });
  return { stdout: decodeWindowsOutput(result.stdout), stderr: decodeWindowsOutput(result.stderr) };
}

async function getWifiInterfaces() {
  const { stdout } = await runNetsh(['wlan', 'show', 'interfaces']);
  return { interfaces: parseWifiInterfaces(stdout), rawOutput: stdout };
}

async function getWifiProfiles(interfaceName) {
  const { stdout } = await runNetsh(buildProfilesArgs(interfaceName));
  return { profiles: parseWifiProfiles(stdout), rawOutput: stdout };
}

async function getWifiOverview() {
  const { interfaces, rawOutput } = await getWifiInterfaces();
  const warnings = [];
  const enriched = await Promise.all(interfaces.map(async (item) => {
    try {
      const profiles = await getWifiProfiles(item.name);
      return { ...item, profiles: profiles.profiles };
    } catch (error) {
      warnings.push(`无法读取“${item.name}”的 Wi-Fi profiles。`);
      return { ...item, profiles: [], profilesError: true };
    }
  }));
  return { interfaces: enriched, rawOutput, warnings };
}

async function assertWifiProfileExists(interfaceName, profileName) {
  const { interfaces } = await getWifiInterfaces();
  if (!interfaces.some((item) => item.name === interfaceName)) throw new NotFoundError('找不到所选 Wi-Fi 网卡。');
  const { profiles } = await getWifiProfiles(interfaceName);
  if (!profiles.includes(profileName)) throw new NotFoundError('所选 Wi-Fi profile 当前不存在。');
}

async function connectWifi(interfaceName, profileName) {
  await assertWifiProfileExists(interfaceName, profileName);
  return runNetsh(buildConnectArgs(interfaceName, profileName));
}

async function disconnectWifi(interfaceName) {
  const { interfaces } = await getWifiInterfaces();
  if (!interfaces.some((item) => item.name === interfaceName)) throw new NotFoundError('找不到所选 Wi-Fi 网卡。');
  return runNetsh(buildDisconnectArgs(interfaceName));
}

function buildProfilesArgs(interfaceName) { return ['wlan', 'show', 'profiles', `interface=${interfaceName}`]; }
function buildConnectArgs(interfaceName, profileName) { return ['wlan', 'connect', `name=${profileName}`, `interface=${interfaceName}`]; }
function buildDisconnectArgs(interfaceName) { return ['wlan', 'disconnect', `interface=${interfaceName}`]; }

module.exports = { buildConnectArgs, buildDisconnectArgs, buildProfilesArgs, connectWifi, disconnectWifi, getWifiInterfaces, getWifiOverview, getWifiProfiles, runNetsh };
