const { ParseError } = require('../utils/errors');
const { runExecutable } = require('./process-runner');

const POWERSHELL_PREFIX = `
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$OutputEncoding = [Console]::OutputEncoding
$InputData = $null
if ($env:WNM_INPUT_BASE64) {
  $InputJson = [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($env:WNM_INPUT_BASE64))
  $InputData = $InputJson | ConvertFrom-Json
}
`;

function buildPowerShellInvocation(script, input) {
  const controlledScript = `${POWERSHELL_PREFIX}\n${script}`;
  return {
    file: 'powershell.exe',
    args: [
      '-NoLogo',
      '-NoProfile',
      '-NonInteractive',
      '-EncodedCommand',
      Buffer.from(controlledScript, 'utf16le').toString('base64'),
    ],
    env: {
      ...process.env,
      WNM_INPUT_BASE64: input === undefined
        ? ''
        : Buffer.from(JSON.stringify(input), 'utf8').toString('base64'),
    },
  };
}

async function runPowerShellJson(script, input, options = {}) {
  const invocation = buildPowerShellInvocation(script, input);
  const result = await runExecutable(invocation.file, invocation.args, {
    ...options,
    env: invocation.env,
    commandName: options.commandName || 'PowerShell',
  });
  const text = result.stdout.toString('utf8').replace(/^\uFEFF/, '').trim();
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new ParseError('无法解析 Windows 返回的数据。', { commandName: options.commandName || 'PowerShell' });
  }
}

async function runPowerShell(script, input, options = {}) {
  const invocation = buildPowerShellInvocation(script, input);
  return runExecutable(invocation.file, invocation.args, {
    ...options,
    env: invocation.env,
    commandName: options.commandName || 'PowerShell',
  });
}

module.exports = { buildPowerShellInvocation, runPowerShell, runPowerShellJson };
