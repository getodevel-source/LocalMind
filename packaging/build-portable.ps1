#Requires -Version 5.1
<#
.SYNOPSIS
  Build the OMNI portable ZIP (no models inside).

.DESCRIPTION
  Runs `cargo build --release`, then assembles
  dist/OMNI-portable-<version>-<YYYYMMDD>.zip with the runtime files the
  app needs next to the exe (see packaging/README.md for the layout and the
  base_dir rules in src-rust/src/main.rs). NEVER includes models/**,
  tests/**, .git/**, target/** or *.bak* files.

.PARAMETER SkipBuild
  Skip `cargo build --release` and package the existing release binary.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File packaging/build-portable.ps1
#>
[CmdletBinding()]
param(
  [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

# --- Version: [package] version in src-rust/Cargo.toml ---
$CargoText = Get-Content (Join-Path $RepoRoot 'src-rust/Cargo.toml') -Raw
if ($CargoText -notmatch '(?m)^version\s*=\s*"([^"]+)"') { throw 'Could not read version from src-rust/Cargo.toml' }
$Version = $Matches[1]
$DateStamp = Get-Date -Format 'yyyyMMdd'
# Commit para trazabilidad ZIP -> commit (canal de auto-update).
$Commit = 'unknown'
try { $Commit = (git -C $RepoRoot rev-parse --short HEAD 2>$null).Trim() } catch {}
# --- Engine build string: bin/llama-server.exe --version (writes to STDERR) ---
# NOTE: $ErrorActionPreference='Stop' turns native stderr lines into
# terminating errors under 2>&1, so relax it just for this call.
$EngineVersion = 'unknown'
$prevEap = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
try {
  # llama-server prints --version to STDERR, which PowerShell wraps in
  # ErrorRecord objects — stringify them explicitly.
  $EngineLines = & (Join-Path $RepoRoot 'bin/llama-server.exe') --version 2>&1 |
    ForEach-Object { "$_".Trim() } |
    Where-Object { $_ -ne '' } |
    Select-Object -First 2
  if ($EngineLines) { $EngineVersion = $EngineLines -join ' / ' }
} catch {
  Write-Warning "Could not run bin/llama-server.exe --version: $_"
} finally {
  $ErrorActionPreference = $prevEap
}
# --- Build (unless -SkipBuild) ---
$BuiltExe = Join-Path $RepoRoot 'src-rust/target/release/localmind.exe'
if (-not $SkipBuild) {
  Write-Host 'Building release binary: cargo build --release ...'
  cargo build --release --manifest-path (Join-Path $RepoRoot 'src-rust/Cargo.toml')
  if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit $LASTEXITCODE)" }
}
if (-not (Test-Path $BuiltExe)) { throw "Release binary not found: $BuiltExe" }
# --- Stage the portable tree in %TEMP% ---
$Stage = Join-Path ([System.IO.Path]::GetTempPath()) ("lm-portable-stage-" + [System.Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $Stage -Force
try {
  # Renamed release binary: cargo emits localmind.exe, the product is OMNI.exe.
  Copy-Item $BuiltExe (Join-Path $Stage 'OMNI.exe')
  foreach ($f in @('ui.html', 'omni.ico', 'omni.png')) {
    $src = Join-Path $RepoRoot $f
    if (-not (Test-Path $src)) { throw "Missing runtime file: $src" }
    Copy-Item $src $Stage
  }
  # ui_fallback.html is compiled INTO the binary (include_str!), but the source
  # layout keeps it next to the exe for reference; harmless if absent at runtime.
  Copy-Item (Join-Path $RepoRoot 'src-rust/ui_fallback.html') $Stage
  # llama.cpp runtime (whole dir; ~106 MB). Excludes nothing: bin/ is the runtime.
  Copy-Item (Join-Path $RepoRoot 'bin') (Join-Path $Stage 'bin') -Recurse
  # Empty models/ dir so first run resolves base_dir to the portable folder
  # (get_base_dir: exe dir wins when it contains models/). Compress-Archive
  # skips truly empty dirs, so ship a pointer file that also tells the user
  # where weights go.
  $ModelsDir = New-Item -ItemType Directory -Path (Join-Path $Stage 'models') -Force
  Set-Content -Path (Join-Path $ModelsDir 'PON_TUS_MODELOS_AQUI.txt') -Encoding UTF8 -Value @(
    'Copia tus archivos .gguf en esta carpeta (models/ junto a OMNI.exe).',
    'Esta carpeta NUNCA viaja dentro del ZIP (los pesos pesan ~14 GB).'
  )

  Set-Content -Path (Join-Path $Stage 'version.txt') -Encoding UTF8 -Value @(
    "OMNI $Version"
    "commit: $Commit"
    "llama.cpp: $EngineVersion"
    "Built: $(Get-Date -Format 'yyyy-MM-dd HH:mm') UTC"
  ) # version.txt es ASCII puro: sin BOM no hay problema.

  $Leeme = @(
    'OMNI portable - inicio rapido',
    '===================================',
    '',
    '1. Extrae este ZIP donde quieras (p. ej. C:\LocalMind).',
    '2. Ejecuta OMNI.exe (se abre la ventana; sin terminal).',
    '3. En la primera ejecucion NO se descarga nada.',
    '',
    'Modelos (.gguf):',
    '- Van en la carpeta models/ junto a OMNI.exe.',
    '- Si ya tienes pesos, copialos ahi antes de iniciar el motor.',
    '- La carpeta models/ NUNCA se incluye en este ZIP (pesa ~14 GB).',
    '',
    'Actualizacion automatica: la app comprueba GitHub Releases al arrancar',
    'y avisa en Ajustes -> Actualizacion (la instalacion es al reiniciar,',
    'tus modelos y ajustes se conservan).',
    '',
    'Si no abre: instala WebView2 (viene con Edge).',
    'Logs: %APPDATA%\LocalMind\logs\localmind.log',
    'Manual completo: docs/MANUAL.md del repo.',
    '',
    'Detener y liberar VRAM/RAM: boton Detener en la ventana o cierra la app.',
    "Version: $Version (commit $Commit, detalle en version.txt)."
  )
  Set-Content -Path (Join-Path $Stage 'LEEME.txt') -Encoding UTF8 -Value $Leeme

  # --- Safety: refuse to zip forbidden content ---
  $Bad = Get-ChildItem $Stage -Recurse | Where-Object {
    $_.FullName -match '\\models\\[^.][^/\\]*\.gguf$' -or
    $_.FullName -match '\\target\\' -or
    $_.FullName -match '\\\.git\\' -or
    $_.Name -like '*.bak*'
  }
  if ($Bad) { throw ("Forbidden content staged: " + (($Bad | Select-Object -First 5 FullName) -join '; ')) }

  $Dist = Join-Path $RepoRoot 'dist'
  $null = New-Item -ItemType Directory -Path $Dist -Force
  $ZipName = "OMNI-portable-$Version-$DateStamp.zip"
  $ZipPath = Join-Path $Dist $ZipName
  if (Test-Path $ZipPath) { Remove-Item $ZipPath -Force }
  Compress-Archive -Path (Join-Path $Stage '*') -DestinationPath $ZipPath -CompressionLevel Optimal

  $Zip = Get-Item $ZipPath
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $Archive = [System.IO.Compression.ZipFile]::OpenRead($Zip.FullName)
  try {
    $Entries = $Archive.Entries
    Write-Host "ZIP: $($Zip.FullName)"
    Write-Host ("Entries: {0}   Size: {1:N2} MB" -f $Entries.Count, ($Zip.Length / 1MB))
    $Suspicious = $Entries | Where-Object { $_.FullName -match '(^|/)(models/|target/|\.git/)' -or $_.FullName -like '*.bak*' -or $_.FullName -match '\.gguf$' }
    if ($Suspicious) { throw ("ZIP contains forbidden entries: " + (($Suspicious | Select-Object -First 5 FullName) -join '; ')) }
    # OMNI.exe + ui.html obligatorios (el updater los exige en el staging).
    $Names = @($Entries | ForEach-Object { $_.FullName })
    if (-not ($Names -contains 'OMNI.exe') -or -not ($Names -contains 'ui.html')) {
      throw 'ZIP inválido para el canal de auto-update: debe traer OMNI.exe + ui.html'
    }
  } finally {
    $Archive.Dispose()
  }
  # SHA-256 del ZIP (canal de auto-update: la release publica
  # "SHA256 <asset> <hex>" en el body; este fichero es la fuente).
  $ZipHash = (Get-FileHash $ZipPath -Algorithm SHA256).Hash.ToLower()
  Set-Content -Path ($ZipPath + '.sha256.txt') -Encoding UTF8 -Value "$ZipHash  $ZipName"
  Write-Host "SHA256: $ZipHash"
} finally {
  Remove-Item $Stage -Recurse -Force -ErrorAction SilentlyContinue
}
