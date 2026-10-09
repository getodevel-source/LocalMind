# Changelog OMNI

Todos los cambios publicables de la app. El formato sigue
[Keep a Changelog](https://keepachangelog.com/es-ES/1.0.0/) y el versionado
[SemVer](https://semver.org/lang/es/).

## [Unreleased]

### Corregido
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
