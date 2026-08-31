$ErrorActionPreference = 'Stop'

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$tauriConfig = Get-Content -LiteralPath (Join-Path $root 'tauri.conf.json') -Raw | ConvertFrom-Json
$version = [string]$tauriConfig.version
$installerDir = Join-Path $root 'installer'
$releaseExe = Join-Path $root 'target\release\windows-network-manager.exe'
$nsisSource = Join-Path $root "target\release\bundle\nsis\Windows Network Manager_${version}_x64-setup.exe"
$nsisTarget = Join-Path $installerDir "Windows Network Manager_${version}_x64-setup.exe"
$portableStage = Join-Path $root 'target\portable'
$portableZip = Join-Path $installerDir "Windows Network Manager_${version}_x64-portable.zip"

if (-not (Test-Path -LiteralPath $releaseExe)) {
  throw "Release executable was not found: $releaseExe"
}
if (-not (Test-Path -LiteralPath $nsisSource)) {
  throw "NSIS installer was not found: $nsisSource"
}

New-Item -ItemType Directory -Path $installerDir -Force | Out-Null
New-Item -ItemType Directory -Path $portableStage -Force | Out-Null

Copy-Item -LiteralPath $nsisSource -Destination $nsisTarget -Force
Copy-Item -LiteralPath $releaseExe -Destination (Join-Path $portableStage 'Windows Network Manager.exe') -Force
Set-Content -LiteralPath (Join-Path $portableStage 'README.txt') -Encoding UTF8 -Value @'
Windows Network Manager portable edition

Extract the complete ZIP before running Windows Network Manager.exe.
Microsoft Edge WebView2 Evergreen Runtime is required.
Network Metric and route changes require administrator privileges.
Use the in-app "Restart as administrator" button to request standard Windows UAC elevation.
'@

if (Test-Path -LiteralPath $portableZip) {
  Remove-Item -LiteralPath $portableZip -Force
}
Compress-Archive -Path (Join-Path $portableStage '*') -DestinationPath $portableZip -CompressionLevel Optimal

Get-ChildItem -LiteralPath $installerDir -File |
  Where-Object { $_.Name -like 'Windows Network Manager_*' -and $_.Name -notin @((Split-Path $nsisTarget -Leaf), (Split-Path $portableZip -Leaf)) } |
  Remove-Item -Force

foreach ($relativePath in @('target', 'dist', 'gen')) {
  $path = Join-Path $root $relativePath
  if ((Test-Path -LiteralPath $path) -and $path.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) {
    Remove-Item -LiteralPath $path -Recurse -Force
  }
}

Write-Output "Installer: $nsisTarget"
Write-Output "Portable ZIP: $portableZip"
