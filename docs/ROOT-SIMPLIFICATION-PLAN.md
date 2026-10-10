# LocalMind — Vuelta a las raíces: plan de simplificación Servidor/Cliente

Fecha: 2026-10-03. Estado: propuesta (no se tocó código de producto).
Origen: desglose completo del árbol + 3 análisis paralelos + decisiones del dueño.

## 0. Visión (dicha por el dueño)

> Una persona con una PC gamer, si sus recursos lo permiten, ejecuta un modelo de IA local
> como **servidor**: corre el modelo ahí y lo usa, o deja que **otra computadora se conecte
> como cliente**. La app da la opción de ser **servidor o cliente**. Una PC puede ponerse
> a disposición para **solo computar** y orquestar con otra, o hacer todo en la misma.

Modos objetivo:

| Modo | Esta PC… | Motor local | Gateway | UI |
|---|---|---|---|---|
| 🖥️ Servidor | solo computa | SÍ | expone LAN (+túnel) con auth + QR/URL | estado, logs, accesos |
| 💻 Cliente | solo consume | NO | apunta a URL remota + clave + "probar conexión" | chat contra remoto |
| 🧠 Todo aquí | computa + consume | SÍ | loopback actual | comportamiento actual |

Decisiones del dueño ya tomadas (2026-10-03):

1. **Red: LAN + internet con túnel** (no solo LAN).
2. **Alcance: mantener telemetría y lanzadores, cortar lo demás.**
3. **Primer paso: limpieza backend primero** (antes que UI/red).

## 1. Foto real (medida en disco, no estimada)

| Capa | Medición | Qué es |
|---|---|---|
| Backend Rust `src-rust/src/` | **14.664 líneas, 16 archivos** | `process.rs` 3.415 · `server.rs` 2.872 · `config.rs` 1.619 · `presets.rs` 1.310 · `diffusion.rs` 1.276 · `models.rs` 1.262 · `launcher.rs` 945 · `translate.rs` 932 · `usage.rs` 432 · `agents.rs` 429 · resto |
| UI `ui.html` | **4.843 líneas, 203.061 bytes**, 1 archivo, vanilla JS, cero frameworks/CDN | 7 tabs: Panel/Chat/Modelos/Uso/Logs/Specs/Ajustes. `ui_fallback.html` 305 bytes (pantalla muerta) |
| Contrato `docs/SRS.md` | v1.11, 704 líneas | §1.2 declara **fuera de alcance: servidor de red (no LAN, no internet)** — hay que revertir esta decisión |
| Binarios `bin/` | 92 ficheros, ~178 MB | llama.cpp build 10743 (`llama-server-impl.dll` 7,5 MB + `ggml-vulkan.dll` 57 MB + ~50 stubs) |
| Modelos `models/` | ~27 GB fuera del ZIP | Qwen3 27B 13,9 GB + mmproj 888 MB + Bonsai |
| ZIP portable `dist/` | ~38 MB, 111 ficheros | Exe ~3,4 MB + `ui.html` + `bin/` + `models/` vacía. Sin instalador real, sin firma, sin auto-update |
| Arquitectura viva | `LocalMind.exe (tao+wry+tiny_http)` → `ProcessManager` → `bin/llama-server.exe` en `127.0.0.1:puerto` → gateway `:17860` → WebView2 | 100 % loopback |

Superficie HTTP real (`server.rs`): `GET /`, iconos, ~30 rutas `/api/*`
(`models, version, logs/export, hardware, profiles, profiles/export|import|save|delete, import_model,
open_models_dir, status, settings, config, metrics, start, stop, logs, events(SSE), select_folder, launch,
launch_omp, launch_pi, agents, open_browser, models/download|cancel|import_pick, usage, usage/raw`)
+ gateway `/v1/models`, `POST /v1/chat/completions`, `POST /v1/messages` (Anthropic),
`POST /v1/responses` (Codex).

## 2. Por qué hoy es imposible ser Servidor/Cliente

No falta "un botón". Hay 5 bloqueos arquitectónicos:

1. **Bind fijo `127.0.0.1`.** `0.0.0.0` prohibido por código (`diffusion.rs: FORBIDDEN_LISTEN_ADDR`).
   Motor (`process.rs:696`), gateway y `sd` atan loopback.
2. **CORS loopback-only** (`meta.rs: is_loopback_origin`, SRS P29).
3. **Auth de un solo usuario local** (`auth.rs`: `gateway.key` en `%APPDATA%`, sin pairing).
4. **Lanzadores hardcodeados** a `http://127.0.0.1:<port>/v1` (`agents.rs`, `launcher.rs`).
5. **UI sin concepto de rol**: cero hits de "Servidor/Cliente/LAN/remoto" en `ui.html`;
   todo (perfiles, VRAM, PSU-safe, auto-stop) presupone motor local.

## 3. Núcleo vs expansión (veredicto del desglose)

### 3.1 Núcleo — mantener (sirve a Servidor/Cliente)

- `process.rs` (3.415): spawn/health/stop, poller 500 ms, auto-stop, logs. Deuda: `start()` ~360
  líneas, poller con 6 responsabilidades, gate de 4 requests (~120 s).
- `server.rs` (2.872): gateway + proxy. Deuda: `handle_request` ~40 `if`, validación
  `/api/config` (~210) duplicada, proxy bloqueante `ureq`, mezcla gateway con UI-desktop.
- `config.rs` (1.619): quedará `engine` (modelo, contexto, hilos, puertos, device) + `last`.
  Deuda: migración v0→v5, `cache_reuse` inerte, secciones `speculation/power_safe/notifications/diffusion`.
- `models.rs` (1.262): listar `.gguf` + importar local. Deuda: job global único (409),
  doble inventario LLM/difusión, `dest_for` con aplanado especial.
- `main.rs` (282): quedará bootstrap headless. Deuda: WebView obligatoria, `sleep(3600)`,
  single-instance vía PowerShell.

### 3.2 Mantener por decisión del dueño

- **Lanzadores**: `launcher.rs` (945) + `agents.rs` (429) — pi/omp/opencode/deepseek/web contra gateway.
- **Telemetría**: `usage.rs` (432) — se queda `log_usage`; `usage_summary/raw` pasa a opcional.
- **Compat de agentes**: `translate.rs` (932) — `/v1/messages` + `/v1/responses`. Sin esto los
  lanzadores no funcionan. Congelado: ningún tercer dialecto.

### 3.3 Cortar / extraer (decisión del dueño: "lo demás cortamos")

| Módulo | Tamaño | Veredicto |
|---|---|---|
| `diffusion.rs` (1.276) + `presets.rs` (1.310) + `openspec/changes/diffusion-generation/` (~4.600 pendientes, forecast 4.758, riesgo High) | ~2.600 + pendientes | **Desenganchar tras `enabled=false`.** Segundo motor/modalidad/catálogo/coordinador VRAM. Qwen-Image (~13 GB) + LLM (~6 GB) en 16 GB = alternancia, no convivencia. Puerta incumplida: medición VRAM 3.3 antes de UI |
| `notify.rs` (92) | toasts PowerShell | **A plugin** tras trait `Notifier` + `Noop` |
| `profiles.rs` (43) | DTO fino | **Inline en `config.rs`** |
| `auth.rs` (180), `filelog.rs` (138), `meta.rs` (335) | núcleo sano | **No tocar** (solo mover predicado CORS junto a auth como micro-recorte) |

Recorte esperado: **~40 % del código con 100 % de la visión**.

## 4. Plan por fases

### Fase A — Limpieza backend (PRIMERO, sin cambiar comportamiento)

1. Difusión tras `enabled=false` + feature-flag (no borrar lo puro testeado, desenganchar default).
2. `notify` → `Notifier` + `Noop`.
3. `profiles` → inline en `config.rs`.
4. `config` → squash migración a 1 versión; mover validación `POST /api/config` de `server.rs` a `config.rs`; eliminar `cache_reuse` inerte.
5. `server` → router por tabla (no 40 `if`); separar rutas UI-desktop del gateway.
6. `process` → partir poller (health/gate/idle); gate a 1 probe; PSU/MTP/ETA como opt-in documentado.
7. `translate` → declarar frontera `compat/` (sin mover código aún), congelar superficie.
8. Puertas: `cargo test` + `node tests/ui-render.mjs` + `node tests/bench.mjs --self-test` +
   `node --check tests/*.mjs` + `node tests/ui-tabs.mjs` contra app viva, todo verde.

### Fase B — Servidor / Cliente / Todo-aquí (red LAN + túnel)

- Bind configurable `127.0.0.1` → `0.0.0.0` opt-in **solo en modo Servidor**, auth obligatoria.
- CORS loopback → allowlist LAN + origen del túnel.
- Pairing: QR/URL + token de un solo uso (evolución de `gateway.key`).
- Discovery: mostrar `192.168.x.x:17860`, botón "probar conexión" en Cliente.
- Lanzadores en Cliente inyectan URL remota (no `127.0.0.1`).
- Túnel para internet sin TLS casero (asistido: Cloudflare/Tailscale/WireGuard — a decidir en diseño,
  no en este plan).
- Primera pantalla de rol persistida: Servidor / Cliente / Todo aquí.

### Fase C — UI simplificada

- Rol en primer arranque + persistencia.
- Panel: perfil "Recomendado" + Iniciar/Detener; lo experto a Ajustes.
- Specs plegado tras "avanzado"; fusionar hints duplicados de lanzadores.
- `ui_fallback.html` con diagnóstico (Reintentar / Abrir logs / Reinstalar).
- Resolver drift: idle 1500 s vs SRS 5400 s; exponer (aunque sea read-only)
  `start_cooldown_secs/max_starts_per_hour/power_safe/slow_gate_tps`.

## 5. Riesgos y notas

- Revertir "no LAN" del SRS es decisión de alcance, no bug: actualizar `docs/SRS.md` §1.2/§2.5.
- Sin firma ni auto-update (D-24): fricción SmartScreen real; documentar hash + update-check.
- `bin.bak-*/bin.prev-10683/bin-hip/`, `*.bak-*`, `target/` (284 MB) ensucian raíz pero no viajan al ZIP.

## 6. Cierre de ejecución (2026-10-03, rama `simplificacion/raices-fase-a`)

Fases A/B/C ejecutadas y verificadas (`cargo test` 163/0, clippy 53, puertas UI
verdes, E2E vivo de dos instancias). Dos ítems del plan quedan DIFERIDOS a
propósito, con motivo:

- **Router por tabla del `handle_request`**: los brazos ya quedaron finos (A4
  sacó la validación, B3 suma handlers chicos) y los 163 tests fijan el
  comportamiento. Reescribir el dispatch es riesgo sin retorno funcional.
- **Squash de la migración v0→v5**: INSEGURO de hacer — las "fotos previas"
  (`previous_built_in_field`) son las que distinguen un perfil custom del
  usuario de un default viejo; colapsarlas puede SOBREESCRIBIR customs.
  Se mantiene la migración tal cual.

Hallazgos al cierre (ya corregidos salvo indicación):

- `steamwebhelper` ocupa el 8080 → el proxy con motor detenido le pegaba a él
  (corregido con `engine_reachable`: `stopped`/`error` = `engine_down`).
- `eventsSource` sin declarar → excepción en cada boot (corregido, 1 línea).
- Single-instance furable entre TEMP distintos + `remove_file` incondicional
  del lock (PENDIENTE: endurecer `acquire_single_instance`, no tocado).
- Puertos vivos observados: Steam 8080; gateway 17860+10.

## 6. Trazabilidad

- Medición: `Get-ChildItem src-rust\src` + conteo de líneas (14.664 Rust + 4.843 `ui.html`).
- Endpoints: grep `/api/` en `server.rs` (31 rutas) + `/v1/` + `translate`.
- Binds: grep `127.0.0.1|0.0.0.0|bind` (250+ hits, `FORBIDDEN_LISTEN_ADDR` en `diffusion.rs`).
- UI: grep `tab-|Servidor|Cliente|LAN` en `ui.html` (98 hits, cero de rol).
- Alcance: `docs/SRS.md:30`, `openspec/changes/diffusion-generation/proposal.md`.
