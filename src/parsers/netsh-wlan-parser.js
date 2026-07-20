const FIELD_NAMES = {
  name: new Set(['name', '名称']),
  description: new Set(['description', '描述', '说明']),
  guid: new Set(['guid']),
  state: new Set(['state', '状态']),
  ssid: new Set(['ssid']),
  signal: new Set(['signal', '信号']),
  profile: new Set(['profile', '配置文件']),
  radioType: new Set(['radio type', '无线电类型']),
};

function parseWifiInterfaces(output) {
  const interfaces = [];
  let current = null;
  for (const line of String(output || '').split(/\r?\n/)) {
    const match = line.match(/^\s*([^:：]+?)\s*[:：]\s*(.*?)\s*$/);
    if (!match) continue;
    const key = match[1].trim().toLocaleLowerCase();
    const field = Object.entries(FIELD_NAMES).find(([, names]) => names.has(key))?.[0];
    if (!field) continue;
    if (field === 'name') {
      if (current?.name) interfaces.push(finalize(current));
      current = { name: clean(match[2]) };
    } else if (current) {
      current[field] = clean(match[2]);
    }
  }
  if (current?.name) interfaces.push(finalize(current));
  return interfaces;
}

function parseWifiProfiles(output) {
  const profiles = [];
  const seen = new Set();
  for (const line of String(output || '').split(/\r?\n/)) {
    const match = line.match(/^\s*([^:：]+?)\s*[:：]\s*(.*?)\s*$/);
    if (!match) continue;
    const key = match[1].trim().toLocaleLowerCase();
    if (!key.includes('profile') && !key.includes('配置文件')) continue;
    const value = clean(match[2]);
    if (value && !seen.has(value)) { seen.add(value); profiles.push(value); }
  }
  return profiles;
}

function finalize(value) {
  const normalized = (value.state || '').toLocaleLowerCase();
  const connected = ['connected', '已连接'].includes(normalized);
  return { ...value, state: value.state || '未知', connected };
}

function clean(value) {
  const text = String(value || '').trim();
  return text || null;
}

module.exports = { parseWifiInterfaces, parseWifiProfiles };
