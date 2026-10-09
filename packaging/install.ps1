#Requires -Version 5.1
<#
.SYNOPSIS
  Install OMNI into %LOCALAPPDATA%\Programs\OMNI.

.DESCRIPTION
  Copies the runtime files (OMNI.exe, ui.html, ui_fallback.html,
  omni.ico, omni.png, bin/**) from the repo working tree — or from
  an extracted portable dir via -SourceDir — into the target dir, creates a
  Start Menu shortcut (OMNI.lnk) plus an "Uninstall OMNI" shortcut,
  and NEVER deletes models/: an existing models/ dir in the target is kept
  and reported.

.PARAMETER SourceDir
  Directory holding the runtime files (default: repo root).

.PARAMETER TargetDir
  Install dir (default: %LOCALAPPDATA%\Programs\LocalMind). Exposed for
  testability; normal installs omit it.

.PARAMETER ShortcutDir
  Start Menu folder for the shortcuts (default: StartMenu\Programs\LocalMind).
  Exposed for testability; normal installs omit it.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File packaging/install.ps1
#>
[CmdletBinding()]
param(
  [string]$SourceDir = (Split-Path -Parent $PSScriptRoot),
  [string]$TargetDir = (Join-Path $env:LOCALAPPDATA 'Programs\OMNI'),
  [string]$ShortcutDir = (Join-Path ([Environment]::GetFolderPath('StartMenu')) 'Programs\OMNI')
)

$ErrorActionPreference = 'Stop'

$Files = @('OMNI.exe', 'ui.html', 'ui_fallback.html', 'omni.ico', 'omni.png', 'version.txt', 'LEEME.txt')
foreach ($f in $Files) {
  # ui_fallback.html lives in src-rust/ in the repo tree; in a portable dir it sits next to the exe.
  $candidates = @((Join-Path $SourceDir $f), (Join-Path $SourceDir "src-rust/$f"))
  if (-not ($candidates | Where-Object { Test-Path $_ })) { throw "Missing source file: $f (looked in $SourceDir and $SourceDir/src-rust)" }
}
$BinSrc = Join-Path $SourceDir 'bin'
if (-not (Test-Path $BinSrc)) { throw "Missing source dir: $BinSrc" }

# Refuse to overwrite a running OMNI.exe *from the target dir*.
# (Other OMNI processes — e.g. the owner's live app elsewhere — are not ours.)
$TargetExe = Join-Path $TargetDir 'OMNI.exe'
$runningHere = Get-Process -Name 'OMNI' -ErrorAction SilentlyContinue | Where-Object {
  try { $_.Path -and ($_.Path -eq $TargetExe) } catch { $false }
}
if ((Test-Path $TargetExe) -and $runningHere) {
  throw 'OMNI.exe is running from the install dir. Close it first, then re-run install.ps1.'
}
# Una actualización pendiente (staging en %TEMP%\omni-update) se aplica al
# salir de la app: no reinstalar encima o el swap fallaría a medias.
$PendingUpdate = Get-ChildItem ([System.IO.Path]::GetTempPath()) -Directory -Filter 'omni-update' -ErrorAction SilentlyContinue
if ($PendingUpdate) {
  throw 'Hay una actualización pendiente de instalar. Abre OMNI, reiníciala para aplicarla y vuelve a ejecutar install.ps1.'
}

$null = New-Item -ItemType Directory -Path $TargetDir -Force
$BinDest = New-Item -ItemType Directory -Path (Join-Path $TargetDir 'bin') -Force
foreach ($f in $Files) {
  $src = @((Join-Path $SourceDir $f), (Join-Path $SourceDir "src-rust/$f")) | Where-Object { Test-Path $_ } | Select-Object -First 1
  Copy-Item $src (Join-Path $TargetDir $f) -Force
}
# Copy bin/* INTO the existing target/bin (dest must exist first, otherwise
# PowerShell nests it as bin/bin).
Copy-Item (Join-Path $BinSrc '*') $BinDest -Recurse -Force

# models/ live in the app dir: keep existing ones, otherwise create empty dir.
$ModelsDir = Join-Path $TargetDir 'models'
$HadModels = Test-Path $ModelsDir
if (-not $HadModels) { $null = New-Item -ItemType Directory -Path $ModelsDir -Force }
$ModelCount = @(Get-ChildItem $ModelsDir -Filter '*.gguf' -ErrorAction SilentlyContinue).Count
if ($HadModels) {
  Write-Host "Kept existing models/ ($ModelCount .gguf)."
} else {
  Write-Host 'Created empty models/ — copy your .gguf weights there.'
}

# --- Shortcuts via WScript.Shell ---
$null = New-Item -ItemType Directory -Path $ShortcutDir -Force
$Shell = New-Object -ComObject WScript.Shell
$lnk = $Shell.CreateShortcut((Join-Path $ShortcutDir 'OMNI.lnk'))
$lnk.TargetPath = $TargetExe
$lnk.WorkingDirectory = $TargetDir
$lnk.IconLocation = (Join-Path $TargetDir 'omni.ico')
$lnk.Save()
$unl = $Shell.CreateShortcut((Join-Path $ShortcutDir 'Uninstall OMNI.lnk'))
$unl.TargetPath = 'powershell.exe'
$unl.Arguments = "-ExecutionPolicy Bypass -File `"$TargetDir\uninstall.ps1`""
$unl.WorkingDirectory = $TargetDir
$unl.Save()
# Leave a copy of the uninstaller next to the install for the shortcut above.
Copy-Item (Join-Path $PSScriptRoot 'uninstall.ps1') (Join-Path $TargetDir 'uninstall.ps1') -Force

Write-Host "Installed to $TargetDir"
Write-Host "Shortcut: $ShortcutDir\OMNI.lnk"
