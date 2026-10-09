# Manual de usuario OMNI

## Requisitos

- Windows 10/11 de 64 bits con WebView2 (lo trae Edge; sin WebView2 la
  ventana no abre).
- Modelos `.gguf` aparte (el ZIP no los trae: pesan ~14 GB). 16 GB de VRAM
  para el `Qwen3.8-27B`; con menos VRAM usa un `.gguf` más pequeño.
- Puertos libres: gateway `127.0.0.1:17860` (+10 intentos) y motor
  `llama_port` (default 8080, siguiente libre si choca).

## Instalación

1. Extrae `OMNI-portable-<versión>-<fecha>.zip` donde quieras
   (p. ej. `C:\OMNI`) o ejecuta `packaging/install.ps1` para instalar en
   `%LOCALAPPDATA%\Programs\OMNI` con accesos directos.
2. Copia tus `.gguf` a `models/` junto a `OMNI.exe`.
3. Ejecuta `OMNI.exe`. En SmartScreen pulsa «Más información → Ejecutar de
   todos modos» (la app aún no firma código).

## Primer arranque

1. En el Panel elige modelo y perfil, pulsa **Arrancar**.
2. Chatea en la tab Chat. Los agentes (Pi, OMP, OpenCode, DeepSeek) se
   lanzan desde el Panel con el motor en marcha.
3. Ajustes guarda perfiles, generación, motor/avisos y rol (Oráculo/Guest).

## Actualización automática

- La app comprueba GitHub Releases al arrancar y cada 6 h (silencioso).
- Si hay novedad, el Panel muestra un banner y **Ajustes → Actualización**
  ofrece **Descargar** (con progreso y **Cancelar**).
- La instalación ocurre **al reiniciar**: cierra la app y un swap de
  directorios reemplaza el runtime (respaldo `.prev-<versión>` si algo
  falla). Tus `models/` y `%APPDATA%\LocalMind` (config, claves) se
  conservan siempre.
- Opciones en `localmind.toml` (`[update]`): `feed` (repo `owner/name`,
  vacío = oficial), `check_on_startup` (default true), `auto_download`
  (default false).
- Sin red o sin releases: la tarjeta dice «Al día» o el motivo del fallo;
  nada se descarga solo.

## Dónde está cada cosa

| Qué | Dónde |
|---|---|
| App (exe, UI, `bin/`, `models/`) | junto a `OMNI.exe` (portable o `%LOCALAPPDATA%\Programs\OMNI`) |
| Config `localmind.toml`, clave `gateway.key`, token `hf.token` | `%APPDATA%\LocalMind` |
| Logs `logs/localmind.log` (1 MiB ×3) | `%APPDATA%\LocalMind\logs` |
| Lock single-instance | `%TEMP%\localmind.lock` |
| Staging de update | `%TEMP%\omni-update\<versión>\staging` |

## Backup y restauración

- Copia `%APPDATA%\LocalMind\localmind.toml` + `gateway.key` para
  conservar ajustes e identidad. Los `.gguf` se respaldan aparte.
- Para migrar de PC: instala, pega `models/` y restaura esos dos ficheros.

## Desinstalación

- `packaging/uninstall.ps1`: borra runtime + accesos. Conserva `models/`
  salvo `-RemoveModels`, y conserva `%APPDATA%\LocalMind` salvo
  `-RemoveData` (config, claves, tokens, logs: úsalo antes de vender o
  regalar el PC).

## Conexión / LAN

- Default: solo `127.0.0.1` (esta PC). Con `[lan].enabled = true` (Ajustes
  → Rol → Oráculo → Exponer, **rige al reiniciar**) el gateway escucha en
  `0.0.0.0` **en HTTP plano**: la clave y los prompts viajan sin TLS en tu
  LAN. Úsalo solo en redes de confianza (la tarjeta lo avisa; `GET /api/lan`
  declara `plaintext_http: true`).
- Empareja el Guest con el QR o copiando `omni://<ip>:<puerto>#k=<clave>`
  (Conexión). Tras **Regenerar clave**, reinicia para que tome efecto.

## Problemas comunes

| Síntoma | Causa y arreglo |
|---|---|
| No abre / se cierra al instante | WebView2 ausente (instala Edge); mira `%APPDATA%\LocalMind\logs\localmind.log` |
| «Puerto ocupado» | otro OMNI o app en 17860–17870: cierra el otro o cambia `http_port` |
| El motor no arranca | sin `.gguf` en `models/`; VRAM insuficiente; mira el log |
| Segunda instancia no abre | normal: el lock `%TEMP%\localmind.lock` enfoca la ventana viva |
| Update dice «sin paquete verificado» | la release no publica `SHA256 <asset> <hex>`: espera a la release corregida |
| Update «tamaño inesperado / SHA no coincide» | descarga corrupta: reintenta Descargar |
| Reinstalar encima falla | cierra OMNI; si hay update pendiente (`%TEMP%\omni-update`), reinicia la app para aplicarlo primero |
| Importar modelo no responde | el diálogo es nativo: si lo cancelas, la UI dice «cancelado»; si lo dejas 120 s abierto, expira solo con el mismo aviso |
