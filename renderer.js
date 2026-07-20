import { confirmAction } from './src/renderer/dialogs.js';
import { formatList, h, interfaceRef } from './src/renderer/dom.js';

const api = window.networkManager;
const state = { isAdmin: false, overview: { interfaces: [] }, routes: [], profiles: [], settings: {}, logs: [], busy: false, editingProfile: null, draftRoutes: [] };
const $ = (id) => document.getElementById(id);

function setStatus(message, kind = 'info') { $('global-status').textContent = message; $('global-status').className = `status ${kind}`; }
function addLog(action, target, result, message = '') { state.logs.unshift({ time: new Date(), action, target, result, message }); state.logs = state.logs.slice(0, 100); renderLogs(); }
function errorMessage(result) { return result?.error?.message || '操作失败。'; }

async function refreshAll() {
  if (state.busy) return;
  state.busy = true; $('global-refresh').disabled = true; setStatus('正在读取接口、路由与方案…');
  const [privilege, overview, routes, profiles] = await Promise.all([api.getPrivilegeStatus(), api.getNetworkOverview(), api.getRoutes(), api.listNetworkProfiles()]);
  if (privilege.ok) { state.isAdmin = privilege.data.isAdmin; renderPrivilege(); }
  if (overview.ok) { state.overview = overview.data; renderInterfaces(); } else setStatus(`接口读取失败：${errorMessage(overview)}`, 'error');
  if (routes.ok) { state.routes = routes.data.routes; renderRoutes(); } else addLog('读取路由', '系统', '失败', errorMessage(routes));
  if (profiles.ok) { state.profiles = profiles.data.profiles; state.settings = profiles.data.settings || {}; $('profile-warning').textContent = profiles.data.warning || ''; $('profile-warning').classList.toggle('hidden', !profiles.data.warning); applySettings(); renderProfiles(); }
  $('last-refresh').textContent = `最后刷新：${new Date().toLocaleString()}`;
  state.busy = false; $('global-refresh').disabled = false;
  setStatus(`已读取 ${state.overview.interfaces.length} 个接口和 ${state.routes.length} 条路由。`, 'success');
}

function renderPrivilege() {
  $('admin-status').textContent = state.isAdmin ? '管理员权限：已获得' : '管理员权限：未获得';
  $('admin-status').className = `badge ${state.isAdmin ? 'success' : 'warning'}`;
  for (const button of document.querySelectorAll('.admin-write')) button.disabled = !state.isAdmin || button.dataset.capable === 'false';
  for (const hint of document.querySelectorAll('.admin-hint')) hint.textContent = state.isAdmin ? '' : '此操作需要以管理员身份运行。';
  $('restart-admin').disabled = state.isAdmin;
  $('restart-admin').textContent = state.isAdmin ? '已是管理员身份' : '以管理员身份重启';
}

function applySettings() {
  $('show-system-interfaces').checked = state.settings.showSystemInterfaces === true;
  $('show-all-routes').checked = state.settings.showAllRoutes === true;
}

function renderInterfaces() {
  const visible = state.overview.interfaces.filter((item) => $('show-system-interfaces').checked || !item.systemInterface);
  const counts = visible.reduce((out, item) => ({ ...out, [item.type]: (out[item.type] || 0) + 1 }), {});
  $('interface-summary').replaceChildren(...Object.entries(counts).map(([type, count]) => h('span', { class: 'summary-chip' }, `${type}: ${count}`)));
  $('interface-list').replaceChildren(...visible.map(renderInterfaceCard));
  populateInterfaceSelects();
  renderPrivilege();
}

function renderInterfaceCard(adapter) {
  const card = h('article', { class: `interface-card ${adapter.systemInterface ? 'system-card' : ''}` });
  const title = h('div', { class: 'card-title' }, h('div', {}, h('h3', {}, adapter.alias), h('p', { class: 'muted-text' }, adapter.description || '未知描述')), h('div', {}, h('span', { class: 'type-badge' }, adapter.type), h('span', { class: `state-dot ${adapter.operationalStatus === 'Up' ? 'up' : ''}` }, adapter.operationalStatus || '未知')));
  const details = h('dl', { class: 'details-grid' },
    row('Index / GUID', `${adapter.interfaceIndex} / ${adapter.guid || '—'}`), row('MAC / Link', `${adapter.macAddress || '—'} / ${adapter.linkSpeed || '—'}`),
    row('IPv4', formatList(adapter.ipv4Addresses, (x) => `${x.address}/${x.prefixLength}`)), row('IPv6', formatList(adapter.ipv6Addresses, (x) => `${x.address}/${x.prefixLength}`)),
    row('网关', formatList([...adapter.ipv4Gateways, ...adapter.ipv6Gateways])), row('DNS', formatList(adapter.dnsServers)), row('DHCP / 类别', `${adapter.dhcpEnabled === null ? '未知' : adapter.dhcpEnabled ? '启用' : '禁用'} / ${adapter.networkCategory || '未知'}`));
  card.append(title, details);
  if (adapter.wifi) card.append(renderWifiControls(adapter));
  if (adapter.metrics.ipv4 || adapter.metrics.ipv6) card.append(h('div', { class: 'metric-grid' }, adapter.metrics.ipv4 ? renderMetric(adapter, 'IPv4', adapter.metrics.ipv4) : null, adapter.metrics.ipv6 ? renderMetric(adapter, 'IPv6', adapter.metrics.ipv6) : null));
  if (adapter.capabilities.adapterControl) {
    const enabled = adapter.adminStatus !== 'Down' && adapter.adminStatus !== 'Disabled';
    card.append(h('div', { class: 'card-actions' }, h('button', { class: 'button secondary admin-write', disabled: !state.isAdmin, onClick: () => changeAdapterState(adapter, !enabled) }, enabled ? '禁用适配器' : '启用适配器'), h('span', { class: 'admin-hint' }, state.isAdmin ? '' : '需要管理员权限')));
  }
  return card;
}

function renderWifiControls(adapter) {
  const select = h('select', {});
  for (const profile of adapter.wifi.profiles) select.append(h('option', { value: profile, selected: profile === adapter.wifi.currentProfile }, profile));
  if (!adapter.wifi.profiles.length) select.append(h('option', { value: '' }, adapter.wifi.profilesError ? 'profiles 获取失败' : '无已保存 profile'));
  return h('div', { class: 'wifi-controls' }, h('div', {}, h('strong', {}, `Wi-Fi：${adapter.wifi.ssid || '未连接'}`), h('span', { class: 'muted-text' }, ` 信号 ${adapter.wifi.signalPercent ?? '未知'}% · ${adapter.wifi.radioType || '未知制式'}`)), select,
    h('button', { class: 'button primary', disabled: !adapter.wifi.profiles.length, onClick: () => wifiAction(adapter, 'connect', select.value) }, '连接'),
    h('button', { class: 'button danger', disabled: !adapter.wifi.connected, onClick: () => wifiAction(adapter, 'disconnect') }, '断开'));
}

function renderMetric(adapter, family, metric) {
  const automatic = h('input', { type: 'checkbox', checked: metric.automatic });
  const value = h('input', { type: 'number', min: 1, max: 9999, value: metric.value, disabled: metric.automatic });
  automatic.addEventListener('change', () => { value.disabled = automatic.checked; });
  return h('div', { class: 'metric-editor' }, h('strong', {}, `${family} Metric`), h('label', { class: 'toggle' }, automatic, ' AutomaticMetric'), value,
    h('button', { class: 'button secondary admin-write', 'data-capable': adapter.capabilities[`metric${family}`], disabled: !state.isAdmin || !adapter.capabilities[`metric${family}`], onClick: () => changeMetric(adapter, family, automatic.checked, value.value) }, '应用'),
    h('small', {}, '数值越小通常优先级越高；前缀长度优先于 Metric。'));
}

function row(label, value) { return h('div', { class: 'detail-row' }, h('dt', {}, label), h('dd', {}, value)); }

async function wifiAction(adapter, action, profileName) {
  setStatus(`${action === 'connect' ? '正在连接' : '正在断开'} ${adapter.alias}…`);
  const result = action === 'connect' ? await api.connectWifi({ interfaceName: adapter.alias, profileName }) : await api.disconnectWifi({ interfaceName: adapter.alias });
  addLog(`Wi-Fi ${action}`, adapter.alias, result.ok ? '成功' : '失败', result.ok ? result.data.message : errorMessage(result));
  if (!result.ok) { setStatus(errorMessage(result), 'error'); return; }
  setTimeout(refreshAll, 1200);
}

async function changeMetric(adapter, family, automatic, metric) {
  const input = { interfaceIndex: adapter.interfaceIndex, addressFamily: family, automatic, metric };
  const preview = await api.previewMetricChange(input);
  if (!preview.ok) return setStatus(errorMessage(preview), 'error');
  if (preview.data.unchanged) return setStatus('Metric 已经满足，无需修改。', 'success');
  const confirmed = await confirmAction('确认修改 Metric', [`接口：${adapter.alias}`, `地址族：${family}`, automatic ? '启用 AutomaticMetric' : `手动 Metric：${metric}`, ...preview.data.warnings]);
  if (!confirmed) return;
  const result = await api.setInterfaceMetric({ metric: preview.data.input, confirmationToken: preview.data.confirmationToken });
  addLog('设置 Metric', adapter.alias, result.ok ? '成功' : '失败', result.ok ? family : errorMessage(result));
  if (result.ok) refreshAll(); else setStatus(errorMessage(result), 'error');
}

async function changeAdapterState(adapter, enabled) {
  const preview = await api.previewAdapterState({ interfaceIndex: adapter.interfaceIndex, enabled });
  if (!preview.ok) return setStatus(errorMessage(preview), 'error');
  if (preview.data.unchanged) return;
  const confirmed = await confirmAction(enabled ? '确认启用适配器' : '确认禁用适配器', [`接口：${adapter.alias}`, `描述：${adapter.description || '未知'}`, `IP：${formatList(adapter.ipv4Addresses, (x) => x.address)}`, ...preview.data.warnings]);
  if (!confirmed) return;
  const result = await api.setAdapterState({ interfaceIndex: adapter.interfaceIndex, enabled, confirmationToken: preview.data.confirmationToken });
  addLog(enabled ? '启用适配器' : '禁用适配器', adapter.alias, result.ok ? '成功' : '失败', result.ok ? '' : errorMessage(result));
  if (result.ok) setTimeout(refreshAll, 1500); else setStatus(errorMessage(result), 'error');
}

function populateInterfaceSelects() {
  const selects = [$('route-interface'), document.querySelector('#add-route-form select[name="interfaceIndex"]')];
  for (const select of selects) {
    const current = select.value; select.replaceChildren();
    if (select.id === 'route-interface') select.append(h('option', { value: 'all' }, '全部接口'));
    for (const adapter of state.overview.interfaces.filter((x) => x.capabilities.routes)) select.append(h('option', { value: adapter.interfaceIndex }, `${adapter.alias} (#${adapter.interfaceIndex})`));
    if ([...select.options].some((x) => x.value === current)) select.value = current;
  }
}

function renderRoutes() {
  const family = $('route-family').value, iface = $('route-interface').value, persistence = $('route-persistence').value, search = $('route-search').value.trim().toLowerCase(), showAll = $('show-all-routes').checked;
  const rows = state.routes.filter((route) => (family === 'all' || route.addressFamily === family) && (iface === 'all' || route.interfaceIndex === Number(iface)) && (persistence === 'all' || (persistence === 'persistent' ? route.persistent : route.temporary)) && (showAll || !route.hiddenByDefault) && (!search || `${route.destinationPrefix} ${route.nextHop} ${route.interfaceAlias}`.toLowerCase().includes(search)));
  $('route-table-body').replaceChildren(...rows.map((route) => h('tr', { class: route.defaultRoute ? 'default-route' : '' }, h('td', {}, route.addressFamily), h('td', {}, route.destinationPrefix), h('td', {}, route.onLink ? 'On-link' : route.nextHop), h('td', {}, `${route.interfaceAlias} (#${route.interfaceIndex})`), h('td', {}, route.routeMetric), h('td', {}, route.interfaceMetric ?? '—'), h('td', {}, route.effectiveMetric ?? '—'), h('td', {}, `${route.persistent ? '持久化' : '临时'}${route.systemManaged ? ' / 系统' : ''}${route.note ? ` / ${route.note}` : ''}`), h('td', {}, route.removable ? h('button', { class: 'button compact danger admin-write', disabled: !state.isAdmin, onClick: () => removeRoute(route) }, '删除') : '只读'))));
  populateInterfaceSelects(); renderPrivilege();
}

async function submitRoute(event) {
  event.preventDefault(); const data = new FormData(event.currentTarget);
  const input = { destinationPrefix: data.get('destinationPrefix'), nextHop: data.get('nextHop'), interfaceIndex: data.get('interfaceIndex'), routeMetric: data.get('routeMetric'), persistent: data.get('persistent') === 'on', note: data.get('note') };
  const preview = await api.previewAddRoute(input);
  if (!preview.ok) return setStatus(errorMessage(preview), 'error');
  const confirmed = await confirmAction('确认添加路由', [`目标：${preview.data.route.destinationPrefix}`, `网关：${preview.data.route.nextHop}`, `接口：#${preview.data.route.interfaceIndex}`, `Metric：${preview.data.route.routeMetric}`, preview.data.route.persistent ? '持久化' : '临时', ...preview.data.warnings]);
  if (!confirmed) return;
  const result = await api.addRoute({ route: preview.data.route, confirmationToken: preview.data.confirmationToken });
  addLog('添加路由', preview.data.route.destinationPrefix, result.ok ? '成功' : '失败', result.ok ? '' : errorMessage(result));
  if (result.ok) { event.currentTarget.reset(); refreshRoutes(); } else setStatus(errorMessage(result), 'error');
}

async function removeRoute(route) {
  const preview = await api.previewRemoveRoute(route);
  if (!preview.ok) return setStatus(errorMessage(preview), 'error');
  const confirmed = await confirmAction('确认删除静态路由', [`目标：${route.destinationPrefix}`, `网关：${route.nextHop}`, `接口：${route.interfaceAlias} (#${route.interfaceIndex})`, `Metric：${route.routeMetric}`, route.persistent ? '持久化' : '临时', ...preview.data.warnings]);
  if (!confirmed) return;
  const payload = { addressFamily: route.addressFamily, destinationPrefix: route.destinationPrefix, nextHop: route.nextHop, interfaceIndex: route.interfaceIndex, policyStores: route.policyStores };
  const result = await api.removeRoute({ route: payload, confirmationToken: preview.data.confirmationToken });
  addLog('删除路由', route.destinationPrefix, result.ok ? '成功' : '失败', result.ok ? '' : errorMessage(result));
  if (result.ok) refreshRoutes(); else setStatus(errorMessage(result), 'error');
}

async function refreshRoutes() { const result = await api.getRoutes(); if (result.ok) { state.routes = result.data.routes; renderRoutes(); } else setStatus(errorMessage(result), 'error'); }

function renderProfiles() {
  $('profile-list').replaceChildren(...state.profiles.map((profile) => h('article', { class: 'profile-card' }, h('div', {}, h('h3', {}, profile.name), h('p', { class: 'muted-text' }, profile.note || '无备注'), h('small', {}, `更新：${new Date(profile.updatedAt).toLocaleString()}`)), h('div', { class: 'card-actions' }, h('button', { class: 'button secondary', onClick: () => openProfileEditor(profile) }, '编辑/重命名'), h('button', { class: 'button secondary', onClick: () => openProfileEditor({ ...profile, id: null, name: `${profile.name} - 副本` }) }, '复制'), h('button', { class: 'button secondary', onClick: () => previewProfile(profile, false) }, '预览'), h('button', { class: 'button primary', onClick: () => previewProfile(profile, true) }, '应用'), h('button', { class: 'button danger', onClick: () => deleteProfile(profile) }, '删除')))));
  if (!state.profiles.length) $('profile-list').append(h('div', { class: 'empty-state' }, '尚未保存网络方案。'));
}

function openProfileEditor(profile = null) {
  state.editingProfile = profile; state.draftRoutes = structuredClone(profile?.routeActions || []);
  $('profile-dialog-title').textContent = profile?.id ? '编辑网络方案' : '新建网络方案';
  $('profile-form').elements.name.value = profile?.name || ''; $('profile-form').elements.note.value = profile?.note || '';
  const actions = $('profile-interface-actions'); actions.replaceChildren(...state.overview.interfaces.filter((x) => !x.systemInterface).map((adapter) => renderProfileInterfaceRow(adapter, profile)));
  $('profile-route-interface').replaceChildren(...state.overview.interfaces.filter((x) => x.capabilities.routes && !x.systemInterface).map((adapter) => h('option', { value: adapter.interfaceIndex }, `${adapter.alias} (#${adapter.interfaceIndex})`)));
  renderProfileRoutes(); $('profile-dialog').showModal();
}

function renderProfileInterfaceRow(adapter, profile) {
  const existingWifi = profile?.wifiActions?.find((x) => matchesRef(x.interfaceRef, adapter));
  const existing4 = profile?.metricActions?.find((x) => x.addressFamily === 'IPv4' && matchesRef(x.interfaceRef, adapter));
  const existing6 = profile?.metricActions?.find((x) => x.addressFamily === 'IPv6' && matchesRef(x.interfaceRef, adapter));
  const row = h('div', { class: 'profile-interface-row', 'data-index': adapter.interfaceIndex }, h('strong', {}, `${adapter.alias} (${adapter.type})`));
  if (adapter.wifi) {
    const action = selectFrom([['unchanged','Wi-Fi 不变'],['connect','连接'],['disconnect','断开']], existingWifi?.action || 'unchanged', 'wifiAction');
    const profileSelect = selectFrom(adapter.wifi.profiles.map((x) => [x,x]), existingWifi?.profileName || adapter.wifi.currentProfile || '', 'wifiProfile'); row.append(action, profileSelect);
  }
  if (adapter.metrics.ipv4) row.append(metricProfileControls('IPv4', existing4));
  if (adapter.metrics.ipv6) row.append(metricProfileControls('IPv6', existing6));
  return row;
}

function metricProfileControls(family, existing) { const mode = existing ? existing.automatic ? 'auto' : 'manual' : 'keep'; return h('label', {}, `${family}：`, selectFrom([['keep','不变'],['auto','自动'],['manual','手动']], mode, `${family}Mode`), h('input', { name: `${family}Metric`, type: 'number', min: 1, max: 9999, value: existing?.metric || 25 })); }
function selectFrom(options, selected, name) { const select = h('select', { name }); for (const [value,label] of options) select.append(h('option', { value, selected: value === selected }, label)); return select; }

function renderProfileRoutes() {
  const root = $('profile-route-actions'); root.replaceChildren(...state.draftRoutes.map((route, index) => h('div', { class: 'draft-route' }, `${route.destinationPrefix} → ${route.nextHop} / ${route.desiredState === 'absent' ? '应删除' : '应存在'}`, h('button', { class: 'icon-button', type: 'button', onClick: () => { state.draftRoutes.splice(index,1); renderProfileRoutes(); } }, '×'))));
}

async function saveProfile(event) {
  event.preventDefault(); const wifiActions = [], metricActions = [];
  for (const row of document.querySelectorAll('.profile-interface-row')) {
    const adapter = state.overview.interfaces.find((x) => x.interfaceIndex === Number(row.dataset.index)); const ref = interfaceRef(adapter);
    const wifiAction = row.querySelector('[name="wifiAction"]'); if (wifiAction && wifiAction.value !== 'unchanged') wifiActions.push({ interfaceRef: ref, action: wifiAction.value, profileName: row.querySelector('[name="wifiProfile"]').value });
    for (const family of ['IPv4','IPv6']) { const mode = row.querySelector(`[name="${family}Mode"]`); if (mode && mode.value !== 'keep') metricActions.push({ interfaceRef: ref, addressFamily: family, automatic: mode.value === 'auto', metric: row.querySelector(`[name="${family}Metric"]`).value }); }
  }
  const result = await api.saveNetworkProfile({ id: state.editingProfile?.id || null, name: event.currentTarget.elements.name.value, note: event.currentTarget.elements.note.value, wifiActions, metricActions, routeActions: state.draftRoutes });
  if (!result.ok) return setStatus(errorMessage(result), 'error');
  $('profile-dialog').close(); addLog('保存方案', result.data.name, '成功'); await refreshProfiles();
}

function addDraftRoute() {
  const adapter = state.overview.interfaces.find((x) => x.interfaceIndex === Number($('profile-route-interface').value));
  if (!adapter) return setStatus('请选择可用接口。', 'error');
  const destinationPrefix = $('profile-route-destination').value.trim();
  const nextHop = $('profile-route-next-hop').value.trim();
  if (!destinationPrefix || !nextHop) return setStatus('方案路由需要目标 CIDR 和下一跳。', 'warning');
  state.draftRoutes.push({ id: crypto.randomUUID(), interfaceRef: interfaceRef(adapter), destinationPrefix, nextHop, routeMetric: $('profile-route-metric').value, persistent: $('profile-route-persistent').checked, desiredState: $('profile-route-state').value, managedByApp: true, note: '' });
  $('profile-route-destination').value = ''; $('profile-route-next-hop').value = ''; renderProfileRoutes();
}

async function previewProfile(profile, apply) {
  const preview = await api.previewApplyProfile(profile.id); if (!preview.ok) return setStatus(errorMessage(preview), 'error');
  const lines = preview.data.operations.map((x) => `[${x.status}] ${x.type} - ${x.payload.alias || x.payload.destinationPrefix || ''} ${x.reason || ''} ${(x.warnings || []).join(' ')}`);
  if (!apply) { await confirmAction(`方案预览：${profile.name}`, lines.length ? lines : ['没有需要执行的操作。']); return; }
  if (!preview.data.requiresConfirmation) return setStatus('方案当前已经满足，或没有可执行操作。', 'success');
  if (!await confirmAction(`确认应用方案：${profile.name}`, lines)) return;
  setStatus('正在应用网络方案，请勿重复操作…'); const result = await api.applyProfile({ profileId: profile.id, confirmationToken: preview.data.confirmationToken });
  if (!result.ok) return setStatus(errorMessage(result), 'error');
  for (const item of result.data.results) addLog(`方案/${item.type}`, item.payload.alias || item.payload.destinationPrefix || profile.name, item.status, item.reason || '');
  await refreshAll();
}

async function deleteProfile(profile) { if (!await confirmAction('确认删除网络方案', [`方案：${profile.name}`, '只删除应用配置，不修改当前网络。'])) return; const result = await api.deleteNetworkProfile(profile.id); if (result.ok) { addLog('删除方案', profile.name, '成功'); refreshProfiles(); } else setStatus(errorMessage(result), 'error'); }
async function refreshProfiles() { const result = await api.listNetworkProfiles(); if (result.ok) { state.profiles = result.data.profiles; renderProfiles(); } }
function matchesRef(ref, adapter) { return (ref?.guid && ref.guid === adapter.guid) || (ref?.alias && ref.alias === adapter.alias) || ref?.interfaceIndex === adapter.interfaceIndex; }

function renderLogs() { $('log-body').replaceChildren(...state.logs.map((item) => h('tr', {}, h('td', {}, item.time.toLocaleTimeString()), h('td', {}, item.action), h('td', {}, item.target), h('td', {}, item.result), h('td', {}, item.message)))); }

$('global-refresh').addEventListener('click', refreshAll); $('refresh-routes').addEventListener('click', refreshRoutes); $('add-route-form').addEventListener('submit', submitRoute);
for (const id of ['route-family','route-interface','route-persistence','route-search','show-all-routes']) $(id).addEventListener(id === 'route-search' ? 'input' : 'change', renderRoutes);
$('show-system-interfaces').addEventListener('change', async () => { renderInterfaces(); await api.saveSettings({ showSystemInterfaces: $('show-system-interfaces').checked, showAllRoutes: $('show-all-routes').checked }); });
$('show-all-routes').addEventListener('change', async () => { renderRoutes(); await api.saveSettings({ showSystemInterfaces: $('show-system-interfaces').checked, showAllRoutes: $('show-all-routes').checked }); });
$('restart-admin').addEventListener('click', async () => { if (!await confirmAction('以管理员身份重启', ['Windows 将显示标准 UAC 提示。','当前未保存的表单内容会丢失。'])) return; const result = await api.restartAsAdministrator(); if (!result.ok) setStatus(errorMessage(result), 'error'); });
$('new-profile').addEventListener('click', () => openProfileEditor()); $('profile-form').addEventListener('submit', saveProfile); $('close-profile-dialog').addEventListener('click', () => $('profile-dialog').close()); $('cancel-profile').addEventListener('click', () => $('profile-dialog').close()); $('add-profile-route').addEventListener('click', addDraftRoute); $('clear-logs').addEventListener('click', () => { state.logs = []; renderLogs(); });

refreshAll();
