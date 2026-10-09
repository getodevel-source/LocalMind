# Changelog OMNI

Todos los cambios publicables de la app. El formato sigue
[Keep a Changelog](https://keepachangelog.com/es-ES/1.0.0/) y el versionado
[SemVer](https://semver.org/lang/es/).

## [Unreleased]

## [2.0.7] - 2026-10-09

### Corregido (P0: el drill real rompió el canal en vivo)
- El swap operaba sobre el `base_dir` de datos en vez del dir del exe: en la
  máquina de desarrollo intentó mover `C:\PROYECTOS\OMNI` y la app quedó
  pidiendo reinicio para siempre. Ahora `do_quit` usa `app_dir()` (dir del
  exe) y `prepare_install_on_exit` rechaza dirs sin `OMNI.exe` o con cara de
  repo dev (`check_app_dir` + test).
- Nuevo `POST /api/update/restart` (409 sin `ready`) + botón «Reiniciar e
  instalar» en la tarjeta (claves es/en): salida ordenada por la vía oficial
  sin taskkill. El tick del loop ejecuta stop+swap+Exit en ≤5 s.
- Harness `tests/update-flow.mjs` (6 checks, sin red ni swap): forma del
  canal, restart-409, botón en UI, scripts sin rutas dev, version.update.

## [2.0.6] - 2026-10-09

### Añadido (adaptativo: la app se ajusta al usuario, no al revés)
- Capacidades por modelo en `GET /api/models` (`capabilities: {family,
  ctx_native, thinking}`, heurística honesta por nombre, sin parser GGUF).
- Hardware completo en `GET /api/hardware` (`cpu_physical`, `ram_total_mb`,
  `vram_total_mb` numéricos para la UI y el guard).
- UI adaptativa: contexto acotado al nativo del modelo (opciones de más
  desactivadas), aviso si el modelo no trae thinking, hilos reescritos con
  los físicos reales (recomendado/mitad/todos), backend visible solo-lectura,
  sugerencia de modelo pequeño en estado vacío. Claves es/en.
- Puerta lenta relativa al tamaño (`slow_threshold`: 2B→25, 27B→20, 70B→12;
  cfg como techo) y sampler por familia (Llama/Mistral propios).
- Warning en Logs si se pide effort a modelo sin thinking (no se rechaza).

## [2.0.5] - 2026-10-09

### Corregido (canal auto-update: el drill v2.0.4 lo rompió en vivo)
- `stage_zip` ya no falla si el ZIP trae `models/`: la omite (ni puntero ni
  pesos pisan lo del usuario) y rechaza `.gguf` fuera de `models/`. Tests:
  `staging_omite_models_y_acepta_puntero_viejo`,
  `staging_rechaza_gguf_fuera_de_models`.
- El ZIP portable ya no trae `models/` (ni el puntero): `get_base_dir` cae
  al dir del exe igual (rama 3).
- El swap repone `models/` desde el respaldo `.prev-<ver>` (robocopy) y al
  arrancar hay healing (`heal_models`: copia lo ausente sin pisar). Test
  `heal_models_repone_desde_respaldo_sin_pisar`.

## [2.0.4] - 2026-10-09

### Probado
- Drill del canal auto-update: release mínima para verificar detección,
  descarga verificada e instalación al reiniciar desde la propia app.

## [2.0.3] - 2026-10-09

### Añadido (portabilidad: pensado para otro hardware, no solo esta máquina)
- Guard de VRAM pre-arranque: `POST /api/start` rechaza con 400 y GB
  concretos si `modelo + KV(contexto)` no cabe en la VRAM (margen 0.9,
  `cache_ram` descuenta). Sin dato de VRAM nunca bloquea. Test
  `vram_fit_avisa_con_gb_y_no_bloquea_sin_dato`.
- Onboarding: estado vacío de Modelos con sugerencia + botón que rellena un
  repo pequeño real (`Qwen/Qwen2.5-1.5B-Instruct-GGUF`, ~2 GB). Claves
  `models.starterHint/starterFill` es/en. No descarga solo.
- Puerta lenta relativa al tamaño: `slow_threshold` (≤9 GB→25, ≤17 GB→20,
  >17 GB→12 t/s; cfg como techo). Un 2B en integrada y un 70B ya no comparten
  el umbral calibrado para 27B. Test `slow_threshold_escala_con_tamano_y_cfg_es_techo`.
- Sampler por familia (`Qwen|Llama|Mistral|Generic` por nombre de archivo):
  Llama (0.6/40/0.9) y Mistral (0.7/40/1.0) dejan de recibir el sampler Qwen;
  Qwen/Bonsai/desconocido = Qwen (medido). Solo rellena ausentes; lo
  explícito gana. Test `sampler_por_familia_qwen_llama_mistral_generico`.
- Backend GPU visible y honesto: fila solo-lectura en Ajustes → Motor con el
  `device` efectivo (`Vulkan0`…); `engine.device` sigue solo-lectura en API
  (se cambia en el TOML). Clave `settings.backendNote` es/en.

## [2.0.2] - 2026-10-09

### Corregido
- Canal de auto-update para terceros: el feed default apuntaba al nombre
  viejo del repo (`getodevel-source/LocalMind`, mudado a `OMNI`). Ahora
  apunta a `getodevel-source/OMNI` con test (`canal_default_apunta_al_repo_omni`).
  Sin esto, una config limpia chequeaba contra la redirección en vez del repo real.

## [2.0.1] - 2026-10-09

### Corregido
- VRAM falsa en AMD: drivers que saturan `AdapterRAM` con techos falsos
  (medido: 4095 MB en una RX 6800 XT de 16 GB) ya no ensucian el dato real
  del motor: el WMI solo se usa sin dato de `--list-devices` y con umbral
  ≥8 GB (D-21). `GET /api/hardware` muestra lo real del motor.
- El chat servía Qwen3.8 siempre en su modo más lento y propenso a fallar
  (`xhigh` por defecto de la plantilla + sampler ajeno): ahora el gateway
  aplica `reasoning_effort: medium` y el sampler oficial cuando el cliente
  no fija los valores, y el chat trae selector de razonamiento
  (Medio por defecto, Bajo/Alto/Max/Off) más los valores de Ajustes →
  Generación. Lo explícito (agentes, benches) no se toca.
- Single-instance sin carrera (D-14): el lock ya no se borra a ciegas antes
  de crear; solo un huérfano confirmado (sonda de lectura con `share_mode(0)`)
  se reclama. Con dueño vivo se enfoca la ventana y se sale.
- El fallback de perfiles ya es bilingüe (D-48): `label`/`hint` por `T()`
  en es/en en vez de español hardcodeado.
- Contratos de docs fijados a la UI real de 4 tabs (Panel/Chat/Conexión/
  Ajustes): SRS §§1.2/3.7/D-27 + `TAB_ALIASES` para deep-links viejos.
### Seguridad
- `gateway.key` con ACL solo-usuario (`icacls /inheritance:r`, best-effort
  al crear/cargar/rotar): otro usuario local ya no la lee por defecto.
### Robustez
- Update que sobrevive al apagón duro: al arrancar se recuperan stagings
  válidos olvidados en `%TEMP%\omni-update` como pendientes; la descarga
  exige 2× espacio libre medido (507 `Sin espacio…` si no cabe, sin bloquear
  si no se puede medir).
- VRAM visible (D-21 parcial): `GET /api/hardware` anota uso real vía
  `nvidia-smi` cuando existe (en AMD sin contador estable no se inventa);
  el chip «Motor lento» trae botón «Reiniciar motor» (gesto manual único,
  respeta cooldown).
- Avisos que evitan sorpresas: toast único al ocultar a la bandeja (la X no
  cierra, la VRAM sigue retenida) y pre-aviso 5 min antes del auto-stop
  (cualquier actividad lo cancela).
- Empaquetado de un solo runtime: `build-portable.ps1` rechaza `bin-hip/` y
  `bin.prev*/` en staging y en el ZIP (el canal es Vulkan en `bin/`).
- Firma opcional (D-24 parcial): `build-portable.ps1 -Sign` firma `OMNI.exe`
  con Authenticode (pfx o almacén, con timestamp); sin secreto avisa y sigue
  sin firmar. Sin cert no hay historia SmartScreen.
- LAN honesta sin TLS: el gateway sigue en HTTP plano (`tiny_http` sin stack
  TLS; cambiarlo exige otro servidor) y ahora lo declara (`GET /api/lan`
  con `plaintext_http: true`) y lo avisa en la tarjeta (es/en). P7 decidida
  por el dueño (D-29 cerrada): sin visión hasta revisar P7, con test.
- VRAM AMD honesta (D-21 parcial-honesta): techo instalado vía WMI
  `AdapterRAM` cuando no hay `nvidia-smi`; el uso en AMD sigue sin contador
  y no se inventa.
- Estructura: `mod sse` → `sse.rs` y puerta pura del motor → `engine_gate.rs`
  (7 fns + 5 tests movidos); `mod proxy` se queda (acoplado a `server.rs`).
- DeepSeek versionado (D-28 mitigada): el patch declara `formato v1`;
  D-32 cerrada como harness-side (doble evidencia del timeout 600 s).
### Añadido
- Actualización automática dentro de la app: chequeo silencioso contra
  GitHub Releases al arrancar (cada 6 h), tarjeta «Actualización» en
  Ajustes (buscar / descargar / cancelar + aviso al reiniciar) y banner en
  el Panel. Descarga con resume, verificación obligatoria por tamaño +
  `SHA256 <asset> <hex>` del body de la release (sin checksum no se
  instala), staging validado y swap de directorios al salir con respaldo
  `.prev-<ver>`. `models/` y `%APPDATA%\LocalMind` jamás se tocan.
  Endpoints: `GET /api/update`, `POST /api/update/check`,
  `POST /api/update/download`, `POST /api/update/cancel`
  (`GET /api/version` expone además `update:{state,latest,pending}`).
  Config `[update]`: `feed` (vacío = repo por defecto), `check_on_startup`
  (default true), `auto_download` (default false).
- Pipeline de release: workflow `release.yml` (tag `vX.Y.Z` → build →
  `SHA256SUMS.txt` → GitHub Release con el ZIP portable + body con línea
  `SHA256 <asset> <hex>`), `CHANGELOG.md` y `version.txt` trazable
  (versión + commit + fecha UTC).

## [2.0.0] - 2026-10-03

### Añadido
- ZIP portable (`packaging/build-portable.ps1`), instalador
  (`packaging/install.ps1`) y desinstalador (`packaging/uninstall.ps1`)
  con preservación de `models/`.
- Gateway local + motor `llama-server`, 4 tabs (Panel, Chat, Conexión,
  Ajustes), modo Oráculo/Guest, pairing LAN por QR, descarga verificada
  de modelos desde Hugging Face.
