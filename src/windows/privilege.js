const { PermissionError } = require('../utils/errors');
const { runPowerShellJson, runPowerShell } = require('./powershell');

const ADMIN_CHECK_SCRIPT = `
$Identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$Principal = [Security.Principal.WindowsPrincipal]::new($Identity)
[PSCustomObject]@{ isAdmin = $Principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) } | ConvertTo-Json -Compress
`;

const ADMIN_RESTART_SCRIPT = `
$FilePath = [string]$InputData.filePath
$Arguments = @($InputData.arguments | ForEach-Object { [string]$_ })
$env:ELECTRON_RUN_AS_NODE = $null
$Process = Start-Process -FilePath $FilePath -ArgumentList $Arguments -Verb RunAs -PassThru
[PSCustomObject]@{ started = $null -ne $Process; processId = $Process.Id } | ConvertTo-Json -Compress
`;

async function getPrivilegeStatus() {
  const result = await runPowerShellJson(ADMIN_CHECK_SCRIPT, undefined, { commandName: '管理员权限检测' });
  return { isAdmin: result?.isAdmin === true };
}

async function requireAdministrator() {
  const status = await getPrivilegeStatus();
  if (!status.isAdmin) throw new PermissionError();
}

async function restartAsAdministrator({ filePath, arguments: args }) {
  const result = await runPowerShellJson(ADMIN_RESTART_SCRIPT, { filePath, arguments: args }, {
    commandName: '管理员身份重启',
    timeout: 60_000,
  });
  return result;
}

module.exports = { getPrivilegeStatus, requireAdministrator, restartAsAdministrator };
