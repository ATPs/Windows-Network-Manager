function buildProfilePlan(profile, snapshot) {
  const operations = [];
  const interfaces = snapshot.overview.interfaces || [];
  const routes = snapshot.routes.routes || [];

  for (const action of profile.wifiActions || []) {
    if (action.action === 'unchanged') continue;
    const adapter = resolveInterface(action.interfaceRef, interfaces);
    if (!adapter) { operations.push(failed('wifi', action, '接口当前不存在。')); continue; }
    if (!adapter.wifi) { operations.push(failed('wifi', action, '目标接口不是 Wi-Fi。')); continue; }
    if (action.action === 'connect' && !adapter.wifi.profiles.includes(action.profileName)) {
      operations.push(failed('wifi', { ...action, interfaceIndex: adapter.interfaceIndex }, 'Wi-Fi profile 当前不存在。')); continue;
    }
    const satisfied = action.action === 'connect'
      ? adapter.wifi.connected && adapter.wifi.currentProfile === action.profileName
      : !adapter.wifi.connected;
    operations.push(operation('wifi', satisfied ? 'skipped' : 'pending', { ...action, interfaceIndex: adapter.interfaceIndex, alias: adapter.alias }, satisfied ? '当前状态已经满足。' : null));
  }

  for (const action of profile.metricActions || []) {
    const adapter = resolveInterface(action.interfaceRef, interfaces);
    if (!adapter) { operations.push(failed('metric', action, '接口当前不存在。')); continue; }
    const key = action.addressFamily === 'IPv6' ? 'ipv6' : 'ipv4';
    const current = adapter.metrics[key];
    if (!current) { operations.push(failed('metric', action, '接口不支持该地址族。')); continue; }
    const payload = { ...action, interfaceIndex: adapter.interfaceIndex, alias: adapter.alias };
    if (!snapshot.isAdmin) { operations.push(failed('metric', payload, '需要管理员权限。')); continue; }
    const satisfied = current.automatic === action.automatic && (action.automatic || current.value === action.metric);
    operations.push(operation('metric', satisfied ? 'skipped' : 'pending', payload, satisfied ? 'Metric 已满足。' : null, adapter.defaultRoute ? ['接口承载默认网关。'] : []));
  }

  for (const action of profile.routeActions || []) {
    const adapter = resolveInterface(action.interfaceRef, interfaces);
    if (!adapter) { operations.push(failed('route', action, '接口当前不存在。')); continue; }
    const payload = { ...action, interfaceIndex: adapter.interfaceIndex, alias: adapter.alias };
    const matching = routes.find((route) => route.destinationPrefix === action.destinationPrefix && route.nextHop === action.nextHop && route.interfaceIndex === adapter.interfaceIndex);
    if (action.desiredState === 'absent' && action.managedByApp !== true) {
      operations.push(failed('route', payload, '未标记为本软件管理，不能自动删除。')); continue;
    }
    const satisfied = action.desiredState === 'present' ? Boolean(matching) : !matching;
    if (satisfied) { operations.push(operation('route', 'skipped', payload, '路由状态已经满足。')); continue; }
    if (!snapshot.isAdmin) { operations.push(failed('route', payload, '需要管理员权限。')); continue; }
    const conflicts = action.desiredState === 'present'
      ? routes.filter((route) => route.destinationPrefix === action.destinationPrefix && !matching)
      : [];
    operations.push(operation('route', 'pending', { ...payload, existingRoute: matching || null }, null, conflicts.length ? ['存在相同目标的其他路由。'] : []));
  }

  return {
    profileId: profile.id,
    profileName: profile.name,
    operations,
    counts: countStatuses(operations),
    requiresConfirmation: operations.some((item) => item.status === 'pending'),
  };
}

function resolveInterface(reference, interfaces) {
  if (!reference) return null;
  const guid = String(reference.guid || '').replace(/[{}]/g, '').toLowerCase();
  if (guid) {
    const match = interfaces.find((item) => String(item.guid || '').toLowerCase() === guid);
    if (match) return match;
  }
  if (reference.macAddress && reference.description) {
    const match = interfaces.find((item) => item.macAddress === reference.macAddress && item.description === reference.description);
    if (match) return match;
  }
  return interfaces.find((item) => item.alias === reference.alias)
    || interfaces.find((item) => item.interfaceIndex === reference.interfaceIndex)
    || null;
}

function operation(type, status, payload, reason = null, warnings = []) { return { id: `${type}-${Math.random().toString(36).slice(2)}`, type, status, payload, reason, warnings }; }
function failed(type, payload, reason) { return operation(type, 'failed', payload, reason); }
function countStatuses(items) { return items.reduce((out, item) => ({ ...out, [item.status]: (out[item.status] || 0) + 1 }), {}); }

module.exports = { buildProfilePlan, resolveInterface };
