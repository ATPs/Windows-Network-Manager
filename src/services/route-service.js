const { ConflictError, NotFoundError, ValidationError } = require('../utils/errors');
const { validateAddressFamily, validateInterfaceIndex, validateRouteInput, validateSafeString } = require('../validation/network-validation');
const { addRoute: addRouteCommand, getRawRoutes, removeRoute: removeRouteCommand } = require('../windows/routes');
const { getRawNetworkOverview } = require('../windows/adapters');
const { requireAdministrator } = require('../windows/privilege');
const { consumeConfirmation, issueConfirmation } = require('./confirmation-service');
const { normalizeNetworkOverview } = require('./network-overview-service');

async function getRoutes() {
  const [rawRoutes, rawOverview] = await Promise.all([getRawRoutes(), getRawNetworkOverview()]);
  return normalizeRoutes(rawRoutes, normalizeNetworkOverview(rawOverview));
}

function normalizeRoutes(rawRoutes, overview) {
  const metrics = new Map();
  const interfaceTypes = new Map();
  for (const item of overview.interfaces || []) {
    interfaceTypes.set(item.interfaceIndex, item.type);
    if (item.metrics.ipv4) metrics.set(`IPv4:${item.interfaceIndex}`, item.metrics.ipv4.value);
    if (item.metrics.ipv6) metrics.set(`IPv6:${item.interfaceIndex}`, item.metrics.ipv6.value);
  }
  const persistentKeys = new Set((rawRoutes.persistent || []).map(routeKey));
  const map = new Map();
  for (const route of [...(rawRoutes.active || []), ...(rawRoutes.persistent || [])]) {
    const key = routeKey(route);
    const existing = map.get(key);
    const stores = new Set(existing?.policyStores || []);
    stores.add(route.policyStore);
    const persistent = persistentKeys.has(key);
    const family = route.addressFamily || 'IPv4';
    const interfaceMetric = metrics.get(`${family}:${route.interfaceIndex}`) ?? null;
    const onLink = ['0.0.0.0', '::'].includes(route.nextHop);
    const defaultRoute = ['0.0.0.0/0', '::/0'].includes(route.destinationPrefix);
    const multicast = route.destinationPrefix === '224.0.0.0/4' || route.destinationPrefix?.toLowerCase() === 'ff00::/8';
    const loopback = interfaceTypes.get(route.interfaceIndex) === 'Loopback' || route.destinationPrefix?.startsWith('127.') || route.destinationPrefix === '::1/128';
    const protocol = route.protocol || existing?.protocol || (persistent ? 'NetMgmt' : '');
    const systemManaged = !persistent && protocol !== 'NetMgmt';
    map.set(key, {
      addressFamily: family,
      destinationPrefix: route.destinationPrefix,
      nextHop: route.nextHop,
      interfaceAlias: route.interfaceAlias || existing?.interfaceAlias || '',
      interfaceIndex: Number(route.interfaceIndex),
      routeMetric: Number(route.routeMetric),
      interfaceMetric,
      effectiveMetric: interfaceMetric === null ? null : Number(route.routeMetric) + interfaceMetric,
      protocol,
      policyStores: [...stores],
      persistent,
      temporary: !persistent,
      state: route.state || existing?.state || '',
      publish: route.publish || existing?.publish || '',
      validLifetime: route.validLifetime || existing?.validLifetime || '',
      preferredLifetime: route.preferredLifetime || existing?.preferredLifetime || '',
      defaultRoute,
      onLink,
      multicast,
      systemManaged,
      removable: !loopback && !multicast && (persistent || protocol === 'NetMgmt'),
      hiddenByDefault: loopback || multicast || (systemManaged && onLink && !defaultRoute),
    });
  }
  return { routes: [...map.values()], refreshedAt: new Date().toISOString() };
}

async function previewAddRoute(input) {
  const validated = validateRouteInput(input);
  const route = executableRoute(validated);
  const [routeTable, overview] = await Promise.all([getRoutes(), require('../services/network-overview-service').getNetworkOverview()]);
  if (!overview.interfaces.some((item) => item.interfaceIndex === route.interfaceIndex && item.capabilities.routes)) {
    throw new NotFoundError('所选接口当前不存在或不支持 IP 路由。');
  }
  const store = route.persistent ? 'PersistentStore' : 'ActiveStore';
  const exact = routeTable.routes.find((item) => item.destinationPrefix === route.destinationPrefix && item.nextHop === route.nextHop && item.interfaceIndex === route.interfaceIndex && item.policyStores.includes(store));
  if (exact) throw new ConflictError('完全相同的路由已经存在。', { route: exact });
  const conflicts = routeTable.routes.filter((item) => item.destinationPrefix === route.destinationPrefix && (item.nextHop !== route.nextHop || item.interfaceIndex !== route.interfaceIndex));
  const warnings = [];
  if (validated.destinationWasNormalized) warnings.push(`目标网络将规范化为 ${route.destinationPrefix}。`);
  if (conflicts.length) warnings.push('存在相同目标前缀但网关或接口不同的路由。');
  if (route.destinationPrefix === '0.0.0.0/0') warnings.push('这是默认路由，可能改变网络出口。');
  return { route, conflicts, warnings, confirmationToken: issueConfirmation('add-route', route) };
}

async function addRoute(input) {
  const route = executableRoute(validateRouteInput(input.route));
  consumeConfirmation(input.confirmationToken, 'add-route', route);
  await requireAdministrator();
  const current = await getRoutes();
  const store = route.persistent ? 'PersistentStore' : 'ActiveStore';
  if (current.routes.some((item) => item.destinationPrefix === route.destinationPrefix && item.nextHop === route.nextHop && item.interfaceIndex === route.interfaceIndex && item.policyStores.includes(store))) {
    throw new ConflictError('路由已经存在，未重复添加。');
  }
  await addRouteCommand(route);
  const verified = await getRoutes();
  const created = verified.routes.find((item) => item.destinationPrefix === route.destinationPrefix && item.nextHop === route.nextHop && item.interfaceIndex === route.interfaceIndex);
  if (!created) throw new ValidationError('Windows 未返回新路由，请刷新后检查。');
  return { route: created };
}

async function previewRemoveRoute(input) {
  const identity = validateRouteIdentity(input);
  const table = await getRoutes();
  const route = table.routes.find((item) => sameRoute(item, identity));
  if (!route) throw new NotFoundError('所选路由已不存在。');
  if (!route.removable) throw new ValidationError('该路由属于系统或无法安全定位，不能删除。');
  const payload = { ...identity, policyStores: route.policyStores.slice().sort() };
  const warnings = route.defaultRoute ? ['删除默认路由可能导致断网。'] : [];
  return { route, warnings, confirmationToken: issueConfirmation('remove-route', payload) };
}

async function removeRoute(input) {
  const identity = validateRouteIdentity(input.route);
  const policyStores = Array.isArray(input.route.policyStores) ? input.route.policyStores.slice().sort() : [];
  const payload = { ...identity, policyStores };
  consumeConfirmation(input.confirmationToken, 'remove-route', payload);
  await requireAdministrator();
  for (const policyStore of policyStores.includes('PersistentStore') ? ['PersistentStore', 'ActiveStore'] : policyStores) {
    const current = await getRoutes();
    const stillExists = current.routes.some((item) => sameRoute(item, identity) && item.policyStores.includes(policyStore));
    if (stillExists) await removeRouteCommand({ ...identity, policyStore });
  }
  const verified = await getRoutes();
  if (verified.routes.some((item) => sameRoute(item, identity))) throw new ValidationError('路由仍然存在，请刷新后重试。');
  return { removed: true };
}

function validateRouteIdentity(input) {
  if (!input || typeof input !== 'object') throw new ValidationError('路由标识无效。');
  return {
    addressFamily: validateAddressFamily(input.addressFamily),
    destinationPrefix: validateSafeString(input.destinationPrefix, '目标前缀', { maxLength: 64 }),
    nextHop: validateSafeString(input.nextHop, '下一跳', { maxLength: 64 }),
    interfaceIndex: validateInterfaceIndex(input.interfaceIndex),
  };
}

function routeKey(route) { return `${route.addressFamily}|${route.destinationPrefix}|${route.nextHop}|${route.interfaceIndex}`; }
function sameRoute(a, b) { return a.addressFamily === b.addressFamily && a.destinationPrefix === b.destinationPrefix && a.nextHop === b.nextHop && Number(a.interfaceIndex) === Number(b.interfaceIndex); }
function executableRoute(route) { return { destinationPrefix: route.destinationPrefix, nextHop: route.nextHop, interfaceIndex: route.interfaceIndex, routeMetric: route.routeMetric, persistent: route.persistent, note: route.note || '' }; }

module.exports = { addRoute, getRoutes, normalizeRoutes, previewAddRoute, previewRemoveRoute, removeRoute, routeKey };
