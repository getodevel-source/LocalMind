# Start LocalMind Qwen 3.8 27B Server for AMD RX 6800 XT with Tuned Hardware Profiles
param(
    [ValidateSet("turbo", "balanced", "deep", "ultra")]
    [string]$Profile = "turbo"
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition

$bin = Join-Path $ScriptDir "bin\llama-server.exe"
$model = Join-Path $ScriptDir "models\Qwen3.8-27B-IQ4_XS_4BPW.gguf"
if (-not (Test-Path $model)) {
    $found = Get-ChildItem (Join-Path $ScriptDir "models\*.gguf") | Where-Object { $_.Name -notmatch "mmproj" } | Select-Object -First 1
    if ($found) { $model = $found.FullName }
}
$mmproj = Join-Path $ScriptDir "models\mmproj-BF16.gguf"
$mmprojArgs = if (Test-Path $mmproj) { @("--mmproj", $mmproj) } else { @() }


$ctx = 32768
$extraArgs = @("--cache-reuse", "256")

switch ($Profile) {
    "turbo" {
        $ctx = 32768
        $desc = "Turbo VRAM (32K - Maxima Velocidad T/S)"
        $extraArgs = @("--cache-reuse", "256")
    }
    "balanced" {
        $ctx = 65536
        $desc = "Equilibrado Pro (64K - Documentos y Codigo)"
        $extraArgs = @("--cache-ram", "4096", "--cache-reuse", "256")
    }
    "deep" {
        $ctx = 131072
        $desc = "Extendido (128K - Libros y Repositorios)"
        $extraArgs = @("--cache-ram", "6144", "--cache-reuse", "256")
    }
    "ultra" {
        $ctx = 262144
        $desc = "Limite Hardware (262K Nativo - VRAM + RAM DDR5)"
        $extraArgs = @("-kvu", "--cache-ram", "6144", "--cache-reuse", "512")
    }
}

Write-Host "======================================================================" -ForegroundColor Cyan
Write-Host "   LocalMind Studio - Qwen 3.8 27B (IQ4_XS + MTP Nativo)" -ForegroundColor Green
Write-Host "   Perfil Activo: $desc" -ForegroundColor Yellow
Write-Host "   Aceleracion: AMD Radeon RX 6800 XT (Vulkan 100% Offload)" -ForegroundColor Yellow
Write-Host "   Contexto: $ctx tokens (KV Cache 4-bit)" -ForegroundColor Yellow
Write-Host "   Endpoint: http://127.0.0.1:8080/v1" -ForegroundColor Green
Write-Host "======================================================================" -ForegroundColor Cyan

& $bin `
  -m $model `
  @mmprojArgs `
  -ngl 99 `
  -c $ctx `
  -ctk q4_0 `
  -ctv q4_0 `
  -fa on `
  -a "localmind,qwen3.8-27b,bonsai-2-27b" `
  --spec-type draft-mtp `
  --spec-draft-n-max 2 `
  --spec-draft-p-split 0.1 `
  -t 6 `
  -tb 6 `
  --prio 2 `
  --prio-batch 2 `
  -b 2048 `
  -ub 512 `
  --device Vulkan0 `
  --split-mode none `
  --host 127.0.0.1 `
  --port 8080 `
  -np 1 `
  --reasoning-preserve `
  --metrics `
  @extraArgs
