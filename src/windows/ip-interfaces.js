const { runPowerShellJson } = require('./powershell');

const SET_METRIC_SCRIPT = `
$Parameters = @{
  InterfaceIndex = [int]$InputData.interfaceIndex
  AddressFamily = [string]$InputData.addressFamily
  AutomaticMetric = $(if ($InputData.automatic) { 'Enabled' } else { 'Disabled' })
  Confirm = $false
}
if (-not $InputData.automatic) { $Parameters.InterfaceMetric = [int]$InputData.metric }
Set-NetIPInterface @Parameters -ErrorAction Stop
$Current = Get-NetIPInterface -InterfaceIndex ([int]$InputData.interfaceIndex) -AddressFamily ([string]$InputData.addressFamily)
[PSCustomObject]@{
  interfaceIndex = [int]$Current.InterfaceIndex
  addressFamily = [string]$Current.AddressFamily
  automaticMetric = ([string]$Current.AutomaticMetric -eq 'Enabled')
  interfaceMetric = [int]$Current.InterfaceMetric
} | ConvertTo-Json -Compress
`;

async function setInterfaceMetric(input) {
  return runPowerShellJson(SET_METRIC_SCRIPT, input, { commandName: '设置接口 Metric' });
}

module.exports = { SET_METRIC_SCRIPT, setInterfaceMetric };
