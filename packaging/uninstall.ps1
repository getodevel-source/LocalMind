#Requires -Version 5.1
<#
.SYNOPSIS
  Uninstall LocalMind (keeps models/ unless -RemoveModels).

.DESCRIPTION
  Removes the installed runtime files and Start Menu shortcuts. A running
  LocalMind.exe from the target dir blocks the uninstall unless -Force is
  passed (which stops it via Stop-Process). models/ is PRESERVED unless
  -RemoveModels is passed. Other LocalMind processes (e.g. the owner's live
  app elsewhere) are never touched.

.PARAMETER TargetDir
  Install dir (default: %LOCALAPPDATA%\Programs\LocalMind).

.PARAMETER ShortcutDir
  Start Menu folder created by install.ps1 (default:
  StartMenu\Programs\LocalMind). Exposed for testability.

.PARAMETER Force
  Stop a running LocalMind.exe from the target dir instead of refusing.

.PARAMETER RemoveModels
  Also delete models/ (weights, ~14 GB). Off by default.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File packaging/uninstall.ps1
#>
[CmdletBinding()]
param(
  [string]$TargetDir = (Join-Path $env:LOCALAPPDATA 'Programs\LocalMind'),
  [string]$ShortcutDir = (Join-Path ([Environment]::GetFolderPath('StartMenu')) 'Programs\LocalMind'),
  [switch]$Force,
  [switch]$RemoveModels
)

$ErrorActionPreference = 'Stop'

$TargetExe = Join-Path $TargetDir 'LocalMind.exe'
$runningHere = Get-Process -Name 'LocalMind' -ErrorAction SilentlyContinue | Where-Object {
  try { $_.Path -and ($_.Path -eq $TargetExe) } catch { $false }
}
if ($runningHere -and -not $Force) {
  throw 'LocalMind.exe is running from the install dir. Close it first, or re-run with -Force to stop it.'
}
if ($runningHere -and $Force) {
  $runningHere | Stop-Process -Force
  Write-Host 'Stopped running LocalMind.exe.'
}

if (Test-Path $ShortcutDir) {
  Remove-Item $ShortcutDir -Recurse -Force
  Write-Host "Removed shortcuts: $ShortcutDir"
} else {
  Write-Host 'No shortcuts found (nothing to remove).'
}

if (-not (Test-Path $TargetDir)) {
  Write-Host "Nothing to remove: $TargetDir does not exist."
  return
}

# Remove runtime files, keep models/ by default.
$Keep = @('models')
if ($RemoveModels) { $Keep = @() }
Get-ChildItem $TargetDir -Force | Where-Object { $Keep -notcontains $_.Name } | Remove-Item -Recurse -Force
if ($RemoveModels) {
  Write-Host 'Removed models/ as requested (-RemoveModels).'
} else {
  $ModelCount = @(Get-ChildItem (Join-Path $TargetDir 'models') -Filter '*.gguf' -ErrorAction SilentlyContinue).Count
  Write-Host "Preserved models/ ($ModelCount .gguf)."
}
# Drop the (now possibly empty) dir itself only when models are gone too.
if (-not (Test-Path (Join-Path $TargetDir 'models')) -and @(Get-ChildItem $TargetDir -Force).Count -eq 0) {
  Remove-Item $TargetDir -Force
  Write-Host "Removed empty $TargetDir"
}
