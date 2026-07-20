const { toArray } = require('../utils/normalize');
const { runPowerShellJson } = require('./powershell');

const OVERVIEW_SCRIPT = `
$Adapters = @(Get-NetAdapter -IncludeHidden | ForEach-Object {
  [PSCustomObject]@{
    interfaceIndex = [int]$_.InterfaceIndex
    alias = [string]$_.Name
    description = [string]$_.InterfaceDescription
    guid = [string]$_.InterfaceGuid
    macAddress = [string]$_.MacAddress
    interfaceType = [int]$_.InterfaceType
    hardwareInterface = [bool]$_.HardwareInterface
    virtual = [bool]$_.Virtual
    adminStatus = [string]$_.AdminStatus
    operationalStatus = [string]$_.Status
    mediaConnectionState = [string]$_.MediaConnectionState
    linkSpeed = [string]$_.LinkSpeed
  }
})
$IpInterfaces = @(Get-NetIPInterface | ForEach-Object {
  [PSCustomObject]@{
    interfaceIndex = [int]$_.InterfaceIndex
    alias = [string]$_.InterfaceAlias
    addressFamily = [string]$_.AddressFamily
    automaticMetric = ([string]$_.AutomaticMetric -eq 'Enabled')
    interfaceMetric = [long]$_.InterfaceMetric
    connectionState = [string]$_.ConnectionState
    dhcp = ([string]$_.Dhcp -eq 'Enabled')
    mtu = [long]$_.NlMtu
  }
})
$IpConfigurations = @(Get-NetIPConfiguration -All | ForEach-Object {
  [PSCustomObject]@{
    interfaceIndex = [int]$_.InterfaceIndex
    alias = [string]$_.InterfaceAlias
    ipv4Gateways = @($_.IPv4DefaultGateway | ForEach-Object { [string]$_.NextHop })
    ipv6Gateways = @($_.IPv6DefaultGateway | ForEach-Object { [string]$_.NextHop })
  }
})
$Addresses = @(Get-NetIPAddress | ForEach-Object {
  [PSCustomObject]@{
    interfaceIndex = [int]$_.InterfaceIndex
    addressFamily = [string]$_.AddressFamily
    ipAddress = [string]$_.IPAddress
    prefixLength = [int]$_.PrefixLength
    addressState = [string]$_.AddressState
    type = [string]$_.Type
  }
})
$Dns = @(Get-DnsClientServerAddress | ForEach-Object {
  [PSCustomObject]@{
    interfaceIndex = [int]$_.InterfaceIndex
    addressFamily = [string]$_.AddressFamily
    servers = @($_.ServerAddresses | ForEach-Object { [string]$_ })
  }
})
$ConnectionProfiles = @(Get-NetConnectionProfile -ErrorAction SilentlyContinue | ForEach-Object {
  [PSCustomObject]@{
    interfaceIndex = [int]$_.InterfaceIndex
    name = [string]$_.Name
    category = [string]$_.NetworkCategory
    ipv4Connectivity = [string]$_.IPv4Connectivity
    ipv6Connectivity = [string]$_.IPv6Connectivity
  }
})
[PSCustomObject]@{
  adapters = $Adapters
  ipInterfaces = $IpInterfaces
  ipConfigurations = $IpConfigurations
  addresses = $Addresses
  dns = $Dns
  connectionProfiles = $ConnectionProfiles
} | ConvertTo-Json -Compress -Depth 6
`;

async function getRawNetworkOverview() {
  const data = await runPowerShellJson(OVERVIEW_SCRIPT, undefined, {
    commandName: '读取网络接口',
    timeout: 30_000,
  });
  return {
    adapters: toArray(data?.adapters),
    ipInterfaces: toArray(data?.ipInterfaces),
    ipConfigurations: toArray(data?.ipConfigurations),
    addresses: toArray(data?.addresses),
    dns: toArray(data?.dns),
    connectionProfiles: toArray(data?.connectionProfiles),
  };
}

module.exports = { getRawNetworkOverview, OVERVIEW_SCRIPT };
