const { runPowerShellJson } = require('./powershell');

const SET_ADAPTER_STATE_SCRIPT = `
$Adapter = Get-NetAdapter -IncludeHidden -InterfaceIndex ([int]$InputData.interfaceIndex) -ErrorAction Stop
if ($InputData.enabled) {
  $Adapter | Enable-NetAdapter -Confirm:$false -ErrorAction Stop
} else {
  $Adapter | Disable-NetAdapter -Confirm:$false -ErrorAction Stop
}
[PSCustomObject]@{ interfaceIndex = [int]$InputData.interfaceIndex; enabled = [bool]$InputData.enabled } | ConvertTo-Json -Compress
`;

async function setAdapterEnabled(input) {
  return runPowerShellJson(SET_ADAPTER_STATE_SCRIPT, input, { commandName: input.enabled ? '启用网络适配器' : '禁用网络适配器' });
}

module.exports = { SET_ADAPTER_STATE_SCRIPT, setAdapterEnabled };
