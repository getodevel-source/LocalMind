@echo off
title LocalMind - Qwen 3.8 27B Studio (AMD RX 6800 XT)
cd /d "%~dp0"

:: Asegurar que no haya instancias huerfanas previas
taskkill /F /IM llama-server.exe >nul 2>&1

set "MODEL_FILE=models\Qwen3.8-27B-IQ4_XS_4BPW.gguf"
if not exist "%MODEL_FILE%" (
  for %%f in (models\*.gguf) do (
    set "MODEL_FILE=%%f"
    goto :found_model
  )
)
:found_model
set "MMPROJ_ARG="
if exist "models\mmproj-BF16.gguf" set "MMPROJ_ARG=--mmproj models\mmproj-BF16.gguf"
if exist "models\mmproj-F16.gguf" set "MMPROJ_ARG=--mmproj models\mmproj-F16.gguf"


echo ======================================================================
echo           LOCALMIND STUDIO - QWEN 3.8 27B (IQ4_XS + MTP NATIVO)
echo ======================================================================
echo   GPU:           AMD Radeon RX 6800 XT (16 GB VRAM - Vulkan 100%% Offload)
echo   CPU Tuning:    Ryzen 5 7600 (6 Cores Zen 4 - AVX-512)
echo   Modelo:        %MODEL_FILE%
echo   Aceleracion:   Multi-Token Prediction (draft-mtp, n=2) + Flash Attention
echo   Prioridad:     Alta (High Process Priority 2)
echo ======================================================================
echo   Selecciona Perfil de Rendimiento y Contexto:
echo.
echo   [1] TURBO VRAM    - 32K tokens  ^| Maximo T/S puro (100%% GPU VRAM)
echo   [2] EQUILIBRADO   - 64K tokens  ^| Documentos y Codigo largo
echo   [3] EXTENDIDO     - 128K tokens ^| Libros y proyectos enteros
echo   [4] ULTRA LIMIT   - 262K tokens ^| Limite fisico nativo (VRAM + RAM DDR5)
echo.
echo   (Se iniciara automaticamente la Opcion [1] en 5 segundos...)
echo ======================================================================

set "PROFILE_CHOICE=1"
choice /c 1234 /t 5 /d 1 /n /m "Ingresa tu opcion [1-4]: "
if errorlevel 4 goto :p_ultra
if errorlevel 3 goto :p_deep
if errorlevel 2 goto :p_balanced
goto :p_turbo

:p_turbo
set "CTX=32768"
set "UBATCH=1024"
set "EXTRA_FLAGS=--cache-reuse 256"
set "PROFILE_NAME=TURBO VRAM (32K - Maxima Velocidad)"
goto :launch

:p_balanced
set "CTX=65536"
set "UBATCH=1024"
set "EXTRA_FLAGS=--cache-ram 4096 --cache-reuse 256"
set "PROFILE_NAME=EQUILIBRADO PRO (64K)"
goto :launch

:p_deep
set "CTX=131072"
set "UBATCH=512"
set "EXTRA_FLAGS=--cache-ram 6144 --cache-reuse 256"
set "PROFILE_NAME=EXTENDIDO (128K)"
goto :launch

:p_ultra
set "CTX=262144"
set "UBATCH=512"
set "EXTRA_FLAGS=-kvu --cache-ram 6144 --cache-reuse 512"
set "PROFILE_NAME=LIMITE HARDWARE (262K Nativo)"
goto :launch

:launch
cls
echo ======================================================================
echo   INICIANDO LOCALMIND CON PERFIL: %PROFILE_NAME%
echo ======================================================================
echo   Contexto:      %CTX% tokens
echo   Endpoints:     http://127.0.0.1:8080/v1 (OpenAI API)
echo                  http://127.0.0.1:8080 (Web Chat Oficial)
echo                  http://127.0.0.1:8080/metrics (Telemetria Prometheus)
echo ======================================================================
echo   [1/2] Cargando tensores en VRAM con MTP y Flash Attention...
echo   [2/2] Apenas este listo, se abrira automaticamente en tu navegador.
echo ======================================================================
echo   [IMPORTANTE] Para apagar el modelo y liberar el 100%% de la VRAM:
echo   Simplemente CIERRA esta ventana o presiona Ctrl+C.
echo ======================================================================
echo.

:: Helper invisible que espera al endpoint /health y abre el navegador
start /min powershell -NoProfile -WindowStyle Hidden -Command "$u='http://127.0.0.1:8080/health'; while($true){ try{ if((Invoke-WebRequest -Uri $u -TimeoutSec 1).StatusCode -eq 200){ break } }catch{} Start-Sleep -Milliseconds 500 }; Start-Process 'http://127.0.0.1:8080'"

:: Ejecutar llama-server
bin\llama-server.exe ^
  -m "%MODEL_FILE%" ^
  %MMPROJ_ARG% ^
  -ngl 99 ^
  -c %CTX% ^
  -ctk q4_0 ^
  -ctv q4_0 ^
  -fa on ^
  -a "localmind,qwen3.8-27b,bonsai-2-27b" ^
  --reuse-port ^
  --spec-type draft-mtp ^
  --spec-draft-n-max 2 ^
  --spec-draft-p-split 0.1 ^
  -t 6 ^
  -tb 6 ^
  --prio 2 ^
  --prio-batch 2 ^
  -b 2048 ^
  -ub %UBATCH% ^
  --device Vulkan0 ^
  --split-mode none ^
  --host 127.0.0.1 ^
  --port 8080 ^
  -np 1 ^
  --reasoning-preserve ^
  --metrics ^
  %EXTRA_FLAGS%

:: Limpieza garantizada al salir
taskkill /F /IM llama-server.exe >nul 2>&1
echo.
echo ======================================================================
echo   Servidor cerrado. 100%% de VRAM y memoria liberada.
echo ======================================================================
ping -n 2 127.0.0.1 >nul
