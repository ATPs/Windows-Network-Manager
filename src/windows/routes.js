const { toArray } = require('../utils/normalize');
const { runPowerShellJson } = require('./powershell');

const GET_ROUTES_SCRIPT = `
function Convert-Route($Route, $Store) {
  [PSCustomObject]@{
    addressFamily = [string]$Route.AddressFamily
    destinationPrefix = [string]$Route.DestinationPrefix
    nextHop = [string]$Route.NextHop
    interfaceAlias = [string]$Route.InterfaceAlias
    interfaceIndex = [int]$Route.InterfaceIndex
    routeMetric = [long]$Route.RouteMetric
    protocol = [string]$Route.Protocol
    policyStore = $Store
    state = [string]$Route.State
    publish = [string]$Route.Publish
    validLifetime = [string]$Route.ValidLifetime
    preferredLifetime = [string]$Route.PreferredLifetime
  }
}
$Active = @(Get-NetRoute -PolicyStore ActiveStore | ForEach-Object { Convert-Route $_ 'ActiveStore' })
$Persistent = @(Get-NetRoute -PolicyStore PersistentStore | ForEach-Object { Convert-Route $_ 'PersistentStore' })
[PSCustomObject]@{ active = $Active; persistent = $Persistent } | ConvertTo-Json -Compress -Depth 5
`;

const ADD_ROUTE_SCRIPT = `
$Store = if ($InputData.persistent) { 'PersistentStore' } else { 'ActiveStore' }
New-NetRoute -AddressFamily IPv4 -DestinationPrefix ([string]$InputData.destinationPrefix) -NextHop ([string]$InputData.nextHop) -InterfaceIndex ([int]$InputData.interfaceIndex) -RouteMetric ([int]$InputData.routeMetric) -PolicyStore $Store -Confirm:$false -ErrorAction Stop | Out-Null
[PSCustomObject]@{ created = $true; policyStore = $Store } | ConvertTo-Json -Compress
`;

const REMOVE_ROUTE_SCRIPT = `
Remove-NetRoute -AddressFamily ([string]$InputData.addressFamily) -DestinationPrefix ([string]$InputData.destinationPrefix) -NextHop ([string]$InputData.nextHop) -InterfaceIndex ([int]$InputData.interfaceIndex) -PolicyStore ([string]$InputData.policyStore) -Confirm:$false -ErrorAction Stop
[PSCustomObject]@{ removed = $true; policyStore = [string]$InputData.policyStore } | ConvertTo-Json -Compress
`;

async function getRawRoutes() {
  const data = await runPowerShellJson(GET_ROUTES_SCRIPT, undefined, { commandName: '读取路由表', timeout: 30_000 });
  return { active: toArray(data?.active), persistent: toArray(data?.persistent) };
}

async function addRoute(input) {
  return runPowerShellJson(ADD_ROUTE_SCRIPT, input, { commandName: '添加静态路由' });
}

async function removeRoute(input) {
  return runPowerShellJson(REMOVE_ROUTE_SCRIPT, input, { commandName: '删除静态路由' });
}

module.exports = { ADD_ROUTE_SCRIPT, GET_ROUTES_SCRIPT, REMOVE_ROUTE_SCRIPT, addRoute, getRawRoutes, removeRoute };
