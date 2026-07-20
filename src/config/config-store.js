const fs = require('node:fs/promises');
const path = require('node:path');

const CONFIG_FILE = 'network-profiles.json';

function defaultConfig() {
  return { schemaVersion: 1, settings: { showSystemInterfaces: false, showAllRoutes: false }, routeNotes: {}, profiles: [] };
}

function createConfigStore(directory) {
  const filePath = path.join(directory, CONFIG_FILE);

  async function load() {
    try {
      const parsed = JSON.parse(await fs.readFile(filePath, 'utf8'));
      return { config: normalizeConfig(parsed), warning: null };
    } catch (error) {
      if (error.code === 'ENOENT') return { config: defaultConfig(), warning: null };
      await fs.mkdir(directory, { recursive: true });
      const backupPath = path.join(directory, `network-profiles.corrupt-${safeTimestamp()}.json`);
      try { await fs.copyFile(filePath, backupPath); } catch { /* The original may have disappeared. */ }
      return { config: defaultConfig(), warning: `配置文件损坏，已备份为 ${path.basename(backupPath)}。` };
    }
  }

  async function save(config) {
    const normalized = normalizeConfig(config);
    await fs.mkdir(directory, { recursive: true });
    const temporaryPath = `${filePath}.${process.pid}.tmp`;
    const handle = await fs.open(temporaryPath, 'w');
    try {
      await handle.writeFile(`${JSON.stringify(normalized, null, 2)}\n`, 'utf8');
      await handle.sync();
    } finally {
      await handle.close();
    }
    await fs.rename(temporaryPath, filePath);
    return normalized;
  }

  return { filePath, load, save };
}

function normalizeConfig(value) {
  const base = defaultConfig();
  if (!value || typeof value !== 'object' || Number(value.schemaVersion) !== 1) return base;
  return {
    schemaVersion: 1,
    settings: {
      showSystemInterfaces: value.settings?.showSystemInterfaces === true,
      showAllRoutes: value.settings?.showAllRoutes === true,
    },
    routeNotes: value.routeNotes && typeof value.routeNotes === 'object' && !Array.isArray(value.routeNotes) ? value.routeNotes : {},
    profiles: Array.isArray(value.profiles) ? value.profiles : [],
  };
}

function safeTimestamp() {
  return new Date().toISOString().replace(/[:.]/g, '-');
}

module.exports = { CONFIG_FILE, createConfigStore, defaultConfig, normalizeConfig };
