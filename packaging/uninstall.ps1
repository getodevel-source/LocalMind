#Requires -Version 5.1
<#
.SYNOPSIS
  Uninstall OMNI (keeps models/ and user data unless asked).
.DESCRIPTION
  Removes the installed runtime files and Start Menu shortcuts. A running
  OMNI.exe from the target dir blocks the uninstall unless -Force is
  passed (which stops it via Stop-Process). models/ is PRESERVED unless
  -RemoveModels is passed. Other OMNI processes (e.g. the owner's live
  app elsewhere) are never touched.

.PARAMETER TargetDir
  Install dir (default: %LOCALAPPDATA%\Programs\OMNI).

.PARAMETER ShortcutDir
  Start Menu folder created by install.ps1 (default:
  StartMenu\Programs\OMNI). Exposed for testability.

.PARAMETER Force
  Stop a running OMNI.exe from the target dir instead of refusing.

.PARAMETER RemoveModels
  Also delete models/ (weights, ~14 GB). Off by default.

.PARAMETER RemoveData
  Also delete %APPDATA%\LocalMind (config, gateway key, tokens, logs,
  usage). Off by default: reinstalls keep identity and settings.
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File packaging/uninstall.ps1
#>
param(
  [string]$TargetDir = (Join-Path $env:LOCALAPPDATA 'Programs\OMNI'),
  [string]$ShortcutDir = (Join-Path ([Environment]::GetFolderPath('StartMenu')) 'Programs\OMNI'),
  [switch]$Force,
  [switch]$RemoveModels,
  [switch]$RemoveData
)

$ErrorActionPreference = 'Stop'

# --- Guardarraíl: TargetDir debe parecer una instalación OMNI real ---
# Exige huella (OMNI.exe o version.txt) para no borrar un dir equivocado
# (p. ej. un -TargetDir mal escrito). Los paths cortos o raíz se bloquean
# siempre: borrar C:\, el home o un drive nunca es una desinstalación.
function Test-UnsafeTarget([string]$Dir) {
  if ([string]::IsNullOrWhiteSpace($Dir)) { return 'vacío' }
  $Full = [System.IO.Path]::GetFullPath($Dir).TrimEnd('\', '/')
  $Root = [System.IO.Path]::GetPathRoot($Full).TrimEnd('\', '/')
  if ($Full -eq $Root) { return 'raíz del drive' }
  $UserHome = [System.IO.Path]::GetFullPath($HOME).TrimEnd('\', '/')
  $LocalApp = [System.IO.Path]::GetFullPath($env:LOCALAPPDATA).TrimEnd('\', '/')
  $AppData = [System.IO.Path]::GetFullPath($env:APPDATA).TrimEnd('\', '/')
  foreach ($Stop in @($UserHome, $LocalApp, $AppData)) {
    if ($Full -eq $Stop) { return "dir protegido ($Stop)" }
  }
  if (($Full.Split([System.IO.Path]::DirectorySeparatorChar) | Where-Object { $_ -ne '' }).Count -lt 2) {
    return 'path demasiado corto'
  }
  return $null
}
$Unsafe = Test-UnsafeTarget $TargetDir
if ($Unsafe) { throw "TargetDir inseguro ($Unsafe): '$TargetDir'. Pasa el dir de instalación real de OMNI." }
if ((Test-Path $TargetDir) -and -not ((Test-Path (Join-Path $TargetDir 'OMNI.exe')) -or (Test-Path (Join-Path $TargetDir 'version.txt')))) {
  throw "TargetDir sin huella OMNI (falta OMNI.exe y version.txt): '$TargetDir'. Nada que desinstalar ahí."
}

$TargetExe = Join-Path $TargetDir 'OMNI.exe'
$runningHere = Get-Process -Name 'OMNI' -ErrorAction SilentlyContinue | Where-Object {
  try { $_.Path -and ($_.Path -eq $TargetExe) } catch { $false }
}
if ($runningHere -and -not $Force) {
  throw 'OMNI.exe is running from the install dir. Close it first, or re-run with -Force to stop it.'
}
if ($runningHere -and $Force) {
  $runningHere | Stop-Process -Force
  Write-Host 'Stopped running OMNI.exe.'
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
# No autodestruir el uninstaller en uso: si este script corre desde el
# TargetDir (atajo del Menú Inicio → $TargetDir\uninstall.ps1), su propio
# fichero se excluye del barrido. Borrar el script en ejecución es frágil
# (y rompe re-ejecuciones); lo ya cargado en memoria sigue corriendo.
$Keep = @('models')
if ($RemoveModels) { $Keep = @() }
$SelfName = $null
try {
  $SelfFull = [System.IO.Path]::GetFullPath($PSCommandPath)
  $TargetFull = [System.IO.Path]::GetFullPath($TargetDir).TrimEnd('\', '/')
  if ($SelfFull.StartsWith($TargetFull + [System.IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    $SelfName = Split-Path $SelfFull -Leaf
    if ($Keep -notcontains $SelfName) { $Keep += $SelfName }
  }
} catch {}
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
# User data (identity + secrets): only with explicit consent.
$DataDir = Join-Path $env:APPDATA 'LocalMind'
if ($RemoveData -and (Test-Path $DataDir)) {
  Remove-Item $DataDir -Recurse -Force
  Write-Host "Removed user data: $DataDir"
} elseif (Test-Path $DataDir) {
  Write-Host "Preserved user data: $DataDir (re-run with -RemoveData to wipe config, keys and logs)."
}
