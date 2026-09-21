@echo off
title LocalMind Control Center
cd /d "%~dp0"

:: Limpiar instancias previas para garantizar 100% de memoria libre
taskkill /F /IM llama-server.exe >nul 2>&1

echo ======================================================================
echo    INICIANDO LOCALMIND CONTROL CENTER (UI TODO EN UNO)
echo ======================================================================
echo    [1/2] Levantando Dashboard y monitor de GPU...
echo    [2/2] Abriendo ventana nativa de aplicacion...
echo ======================================================================
echo    [NOTA] Cuando cierres la aplicacion, se liberara el 100%% de la VRAM.
echo ======================================================================

:: Helper invisible que espera al Dashboard y abre Chrome en modo App nativa
start /min powershell -NoProfile -WindowStyle Hidden -Command "$u='http://127.0.0.1:7860/api/status'; while($true){ try{ if((Invoke-WebRequest -Uri $u -TimeoutSec 1).StatusCode -eq 200){ break } }catch{} Start-Sleep -Milliseconds 300 }; if(Test-Path 'C:\Program Files\Google\Chrome\Application\chrome.exe'){ Start-Process 'C:\Program Files\Google\Chrome\Application\chrome.exe' '--app=http://127.0.0.1:7860' } else { Start-Process 'http://127.0.0.1:7860' }"

:: Ejecutar backend de FastAPI
.venv\Scripts\python.exe app.py

:: Al salir del backend, asegurar que todo quede purgado al 100%
taskkill /F /IM llama-server.exe >nul 2>&1
echo ======================================================================
echo    LocalMind Control Center cerrado. Memoria liberada.
echo ======================================================================
ping -n 2 127.0.0.1 >nul
exit
