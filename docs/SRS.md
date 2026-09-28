# LocalMind — SRS (Software Requirements Specification)

| Campo | Valor |
|---|---|
| Versión | 1.5 |
| Fecha | 2026-09-28 |
| Base verificada | Árbol de trabajo post-integración, ronda 7 (cambios sin commitear sobre commit `1120228`): `server.rs` (sanitización `reasoning_effort` + 3 proxys), `config.rs` (nota 128K `cache_ram`), suite `tests/` (`harness-bench.mjs` + `--fair/--warmup`, `pressure-sampler.mjs`), `tests/harness-bench/results/*.json` (9 ficheros campaña fair), `docs/agents/{harness-comparison-128k,engine-pressure}.md` nuevos. `cargo test` 74/0 en vivo. Los números de línea citados corresponden a ese árbol, no al commit base |
| Alcance | Contrato **real** del sistema hoy + objetivo por fases + deuda conocida + decisiones P1–P32 cerradas 2026-09-25 |
| Decisiones | P1–P32 cerradas el 2026-09-25 (`docs/srs/respuestas-20260925.md` + `.json`); cuestionario en `docs/srs/index.html` |
| Método | Cada requisito cita `archivo:línea` del árbol verificado. Nada afirmado sin cita. Lo medido cita `tests/bench-results/*.json` o el nombre del test; lo no medido se marca `[INFERENCE]` o "no medido" |

Convención: RFC 2119 (`MUST`/`SHOULD`/`MAY`). Cada requisito tiene ID estable y estado:
✅ implementado · 🟡 parcial/degradado · ⬜ pendiente · ⚠️ deuda con ID `D-#`.

> Este documento describe **dos cosas distintas a propósito**: (a) lo que el sistema hace hoy, verificado línea por línea, y (b) lo que el dueño quiere que sea, fijado en las decisiones P# cerradas el 2026-09-25 (§11). Las fases del §6 quedan habilitadas en orden F1→F2→F3.

---

## 1. Introducción

### 1.1 Propósito
LocalMind es una **estación de IA local para Windows operada sin terminal**: un único ejecutable que administra un motor llama.cpp sobre GPU AMD de consumo, expone una API OpenAI-compatible en loopback, ofrece chat/telemetría en una ventana nativa y actúa como **centro de lanzamiento** para agentes CLI externos (OMP, Pi) con el modelo local ya configurado.

Este SRS fija el contrato del sistema para que cualquier sesión de trabajo futura sepa qué se está construyendo, qué está verificado y qué sigue pendiente de decisión.

### 1.2 Alcance
**Dentro:** ejecutable de escritorio, gestión de ciclo de vida del motor llama.cpp, perfiles de contexto/rendimiento, configuración TOML, API local, proxy OpenAI con contabilidad de uso, UI (Panel/Chat/Modelos/Uso/Logs/Specs/Ajustes, bilingüe), lanzadores pi/omp/opencode/web/deepseek, notificaciones de escritorio, logs a archivo con rotación, anti-apagón y auto-stop, telemetría local.

**Fuera (por ahora):** inferencia remota/nube, multi-usuario, servidor de red (no LAN, no internet), fine-tuning, RAG/indexado de documentos, apps móviles. Cualquier cambio de alcance entra por P1/P2/P3 del cuestionario.

### 1.3 Definiciones
| Término | Definición |
|---|---|
| Motor | Proceso `bin/llama-server.exe` (llama.cpp) que ejecuta el modelo GGUF (`process.rs:544-632` en el commit base; puerta de aceptación en `process.rs:401-439` del árbol actual) |
| GGUF / cuantización | Formato de pesos y su nivel de compresión (hoy `IQ4_XS_4BPW`, 13 GB en `models/`) |
| KV cache | Memoria de contexto del modelo; puede vivir en VRAM o desbordar a RAM (`cache_ram` por perfil, `config.rs:246-282`) |
| MTP / `draft-mtp` | Decodificación especulativa nativa (`--spec-type draft-mtp`, `config.rs:27-50`); el % de aceptación se mide en telemetría |
| Perfil | Preset con nombre que fija contexto + `cache_ram` + flags extra (`config.rs:246-282`) |
| Alias | Nombre lógico del modelo publicado a CLIs (`localmind`, `qwen3.8-27b`; `config.rs:232-236`); el proxy reescribe cualquier `model` pedido al id servido (`server.rs:155-158`) |
| Clave local | Secreto aleatorio de 32 bytes en hex en `%APPDATA%\LocalMind\gateway.key`, exigido en `/v1/*` y `/api/*` (`auth.rs:1-123`) |
| pi / omp / opencode / deepseek (+ `web`) | Agentes externos y su terminal; LocalMind los lanza contra el GATEWAY ligado (`http://127.0.0.1:<http_port>/v1`, nunca el motor crudo) con la clave y configs privadas por agente (`server.rs:1380-1460`, `launcher.rs:372-440`, `agents.rs:1-203`). Orca fue eliminado del producto y del código por decisión del dueño (cero ocurrencias de `orca`/`Orca` en `src-rust/`, `ui.html`, `packaging/`, `.github/`) |
| `base_dir` | Raíz de datos: carpeta del exe si contiene `models/`, si no el cwd, si no el padre del exe (`main.rs:21-39`) |
| Auto-stop | Apagado del motor por inactividad real, verificado contra `/slots` (`process.rs:186-228` en el commit base) |

### 1.4 Referencias (evidencia primaria)
- Código: `src-rust/src/{main,config,profiles,process,server,auth,agents,models,translate,usage,meta,notify,filelog,launcher}.rs`, `ui.html` (~3.900 líneas, Ajustes + bilingüe + rediseño), `src-rust/Cargo.toml` (`sha2`, `Cargo.toml:17-19`).
- Config viva: `%APPDATA%/LocalMind/localmind.toml` (secciones `engine`, `generation`, `notifications`, `profiles`, `last`).
- Clave local: `%APPDATA%/LocalMind/gateway.key` (generada en el primer arranque, `auth.rs:59-74`).
- Registro de uso: `%APPDATA%/LocalMind/usage.jsonl` + caché `usage-summary.json` (`usage.rs:14-20`, override `LOCALMIND_USAGE_PATH` en `usage.rs:14-18` para no contaminar en tests).
- Logs a archivo: `%APPDATA%/LocalMind/logs/localmind.log` (rotación 1 MiB × 3, `filelog.rs:3-7`).
- Duraciones de carga: `%APPDATA%/LocalMind/load-times.json` (`process.rs:444-447`).
- Verificación de modelos: registros `models/.verified/<fichero>.json` (`models.rs:75-124`).
- Configs privadas de agentes: `%APPDATA%/LocalMind/agents/<agent>/` (`agents.rs:43-48`, `agents.rs:181-203`). `%USERPROFILE%\.pi` y `%USERPROFILE%\.omp` ya no se leen ni escriben. Cero ocurrencias de `orca`/`Orca` en `src-rust/`, `ui.html`, `packaging/`, `.github/` (decisión del dueño: Orca fuera del producto).
- Empaquetado: `packaging/{build-portable,install,uninstall}.ps1`, `packaging/README.md`; ZIP observado `dist/LocalMind-portable-2.0.0-20260925.zip` (111 ficheros, ~39 MB).
- CI: `.github/workflows/ci.yml` (windows-latest: `cargo test` + harnesses offline + `node --check tests/*.mjs`; smoke/e2e excluidos a propósito por necesitar la app viva).
- UI: `docs/ui/contrast.mjs` (17/17 pares), `docs/ui/capture.ps1`, `docs/ui/screens/` (14 capturas es/en + `live-*`/`round4-*`), `docs/ui/notes.md` (notas del rediseño).
- Harnesses de terceros: `docs/agents/opencode-harness.md` (opencode-ai v1.18.10) y `docs/agents/deepseek-harness.md` (`@deepseek-ai/dsh` v0.1.5-rc.3).
- Tests: `tests/{smoke,e2e,bench,ui-render}.mjs` (ui-render 43 checks, bench self-test 23 asserts), `cargo test` (ver §9 con el conteo observado en la fecha de edición), `tests/bench-results/*.json` (12 corridas, 2026-09-24/25).
- Backends GPU: `bin/` (Vulkan, `ggml-vulkan.dll`) frente a `bin-hip/` (HIP/ROCm Windows: `ggml-hip.dll`, `amdhip64_7.dll`, `rocm_kpack.dll`, `llama-server.exe` build 11175).
- Artefactos: `bin/` (103 archivos, 106 MB), `models/` (14 GB), `src-rust/target/` (284 MB), `LocalMind.exe` (3,0 MB).
- Historial de intención: sesiones de trabajo del dueño (citas textuales en §2.1).

---

## 2. Descripción general

### 2.1 Visión y voz del dueño (requisitos de producto)
Citas textuales del dueño, verificadas en el registro de sesiones. Son requisitos de intención de primer orden: el SRS MUST ser coherente con ellas.

1. «esa UI es horrible quiero algo mas premium no hecho por un niño, ademas porq el bat en el escritorio de nuevo para detener y liberar ram si eso deberia estar en la palicacion. ademas y no menos importante la terminal que sea abre junto con el programa, **NO QUIERO UNA TERMINAL**» (2026-09-18).
2. «intente con el 128k y no termina de cargar nunca el modelo completo en la gpu… podes corroborar con auditorias que la totalidad de la aplicacion funciona» (2026-09-18).
3. «consegui unir ambas tecnicas para tratar de conseguir esos **30 t/s en 262k**, audita todo antes de terminar» (2026-09-18).
4. «si estar al maximo genera un **apagon** no es permisible… podemos poner algun tipo de limite en estos procesos» (2026-09-21).
5. «no puede estar nose al 90% principalmente la gpu» — el consumo molesta aun fuera del modelo (2026-09-21).
6. «los botones de pi y omp pueden tener otro boton dentro que permita seleccionar el proyecto… que se abra el cli correspondiente en esa ubicacion con el llm todo configurado con su contexto deseado» (2026-09-21).
7. «me interesa pensar en como podemos mejorarlo, **hacerlo mas profesional**» (2026-09-20).
8. «en esta pc tenemos una aplicacion que creamos llamada localmind, me gustaria ver si podemos trasladar todo a esta carpeta y empezar a trabajar desde aqui» / «de a partir de ahora nos manejaremos en esta carpeta» (2026-09-24/25).
9. «quiero que elimines bonsai y luego chequear que no aparezca en ningun otro lado de la aplicacion que hicimos» (2026-09-25).
10. «quiero que creemos un srs el proyecto para que sepas de ahora en adelante lo que estamos queriendo crear» (2026-09-25).

**Consecuencias normativas:** LM-UI-1 (`MUST NOT` requerir terminal), LM-MOT-9 y LM-NF-2 (metas de rendimiento), LM-NF-3 (anti-apagón es requisito duro, no optimización), LM-CLI-4 (carpeta proyecto), LM-INT-* (configs de CLIs correctas y no destructivas), LM-DST (producto instalable y "profesional").

### 2.2 Arquitectura de ejecución

```
┌─────────────────────────── LocalMind.exe (Rust: tao + wry + tiny_http) ───────────────────────────┐
│  main.rs  ── single instance (%TEMP%/localmind.lock) ── base_dir ── window 1180x820               │
│     │                                                                                             │
│     ├── HttpServer :17860+   ── GET / → ui.html (WebView2 la carga)                               │
│     │        ├── /api/*      ── control (start/stop/status/config/profiles/models/launch/…)        │
│     │        ├── /api/events ── SSE de logs · /api/logs/export · /api/version                      │
│     │        └── /v1/* ── gateway con clave+aliasing+usage ──► 127.0.0.1:<motor>                    │
│     │                                                                                             │
│     └── ProcessManager ── spawn ──► bin/llama-server.exe  (--host 127.0.0.1, puerto dinámico)     │
│              ▲ poller 500 ms: /health, puerta de aceptación, /slots, auto-stop (default 1500 s)    │
│              ├── logs/ : anillo 250 + `logs/localmind.log` (1 MiB × 3) + toasts P20                │
│                                                                                                   │
│  Escrituras externas: NINGUNA fuera de %APPDATA%/LocalMind (agents/<agent>/ privados)              │
└───────────────────────────────────────────────────────────────────────────────────────────────────┘
                       │ lanzadores contra el GATEWAY :<http_port>/v1 (clave incluida):
                       │ pi · omp · opencode · deepseek (+ web = navegador al motor)
                       ▼ terminal: Windows Terminal → `cmd start` (verify + fallback, pid/500)
```

### 2.3 Hardware objetivo real
| Componente | Valor | Evidencia |
|---|---|---|
| GPU | AMD Radeon RX 6800 XT (16 GB VRAM), backend **Vulkan** (`--device Vulkan0`) | `config.rs:190-192`; `%APPDATA%/LocalMind/localmind.toml`; `bin/ggml-vulkan.dll` (54,4 MB) |
| CPU | Zen4, 6 hilos físicos asignados (12 lógicos disponibles), `libomp140` | TOML `threads = 6`; `bin/libomp140.x86_64.dll`, `bin/ggml-cpu-zen4.dll` |
| RAM/energía | DDR5; límite anti-apagón obligatorio (§2.1.4) | `config.rs:60-70` |
| Modelo activo | Qwen3.8-27B, `IQ4_XS_4BPW` (13 GB) + `mmproj-BF16.gguf` (888 MB) | `models/` (14 GB) |

### 2.4 Clases de usuario
| Rol | Interfaz | Qué hace | Requisitos |
|---|---|---|---|
| Dueño (operador local) | Ventana LocalMind | Inicia/detiene motor, elige perfil, chatea, mira telemetría y logs | LM-UI-* |
| Agente externo (pi, omp, opencode, deepseek) | Gateway local con clave | Usa el modelo vía el GATEWAY ligado (`OPENAI_BASE_URL=http://127.0.0.1:<http_port>/v1` + clave inyectada por el lanzador); su tráfico queda contabilizado en `usage.jsonl` | LM-CLI-*, LM-PXY-*, LM-TEL-5 |
| Terceros en la misma máquina | HTTP en loopback | Sin clave → 401 en `/api/*` y `/v1/*`; sin origen loopback → sin cabeceras CORS | LM-API-6, LM-API-11 |

### 2.5 Restricciones duras
1. Windows x64 + WebView2 (runtime del sistema) — `wry`/`tao` (`Cargo.toml:6-16`).
2. Sin terminal visible para el usuario (§2.1.1).
3. Todo el tráfico es loopback; no hay tráfico saliente (`process.rs:251,259`; `server.rs:334,618`).
4. El motor MUST NOT provocar apagones: prioridad `below_normal`, `--prio-batch` desdoblado, `-ub 512` (`config.rs:60-77`; `process.rs:545-547`).
5. Los pesos (14 GB) y binarios de llama.cpp (106 MB) viven fuera de git (`.gitignore`).

---

## 3. Requisitos funcionales

### 3.1 Arranque y ciclo de vida
| ID | Requisito | Estado | Evidencia |
|---|---|---|---|
| LM-ARQ-1 | El sistema MUST ser un único ejecutable que sirve UI y API en loopback; la UI MUST tener un fallback embebido si falta `ui.html` | ✅ | `server.rs:368-389` (UI + fallback), `main.rs:90-171` (arranque + ventana) |
| LM-ARQ-2 | MUST existir una sola instancia: la segunda invocación sale (lock exclusivo en `%TEMP%/localmind.lock`, con limpieza de lock huérfano) | ✅ | `main.rs:58-92` (`acquire_single_instance` + salida) |
| LM-ARQ-3 | `base_dir` MUST resolverse por presencia de `models/` (exe → cwd → padre del exe) | ✅ | `main.rs:21-39` (sin cambios; verificado en el árbol) |
| LM-ARQ-4 | MUST existir modo headless (`--server-only`/`--headless`) para uso sin ventana | ✅ | `main.rs:113-119` |
| LM-ARQ-5 | Cerrar la ventana MUST apagar el motor y liberar VRAM | ✅ | `main.rs:161-169` (`CloseRequested` → `mgr.stop()`), `process.rs:1051-1071` (`stop` + `taskkill`) |
| LM-ARQ-6 | La ventana MUST identificarse como `LocalMind Studio`, 1180×820, mínimo 960×680, con icono propio | ✅ | `main.rs:126-133` |
| LM-ARQ-7 | El arranque concurrente MUST NOT producir dos instancias vivas | ⚠️ D-14 | `main.rs:58-89` |
| LM-ARQ-8 | Instalación/actualización (instalador, versión visible, update) | 🟡 P5 parcial | `packaging/` + LM-DST-2 (sin firma ni auto-update) |

### 3.2 Motor (llama.cpp)
| ID | Requisito | Estado | Evidencia |
|---|---|---|---|
| LM-MOT-1 | `POST /api/start` MUST resolver perfil, modelo, contexto, hilos y prioridad con precedencia request explícito → perfil explícito (contexto de ESE perfil, nunca el de una sesión vieja) → última sesión (sólo si coincide perfil+modelo) → config → default | ✅ | `process.rs:800-810` (llamada) + `process.rs:1199-1229` (`resolve_context`); regresión cubierta en `process.rs:1397-1445` |
| LM-MOT-2 | Los argumentos del motor MUST ser los canónicos: `-m -ngl 99 -c -ctk q4_0 -ctv q4_0 -a localmind --reuse-port -t -tb --prio -b -ub 512 --device Vulkan0 --split-mode none --host 127.0.0.1 --port -np 1 --poll --prio-batch`, más `-fa on`, `--metrics`, `--spec-type draft-mtp --spec-draft-n-max 2 --spec-draft-p-split 0,1`, `--reasoning-preserve`, `--cache-ram`, `--cache-reuse 256`, flags de perfil y `--mmproj` si está habilitado. El perfil `libros` (128K) lleva `extra_flags = ["--load-mode", "none"]` (grafía canónica; `--no-mmap` está deprecado en este build según `bin/llama-server.exe --help`) | ✅ | `process.rs:848-931` (args actuales); perfil 262K con `-kvu` y `cache_ram 6144` en `config.rs:272-285`; `libros` con `--load-mode none` en `config.rs:409-427` |
| LM-MOT-3 | El puerto del motor MUST elegirse libre desde `llama_port` (default 8080) | ✅ | `process.rs:453-467` (`find_free_port`), `process.rs:841` (uso) |
| LM-MOT-4 | La salud MUST sondearse cada 500 ms contra `/health`; con el primer OK se entra en verificación (`verifying`), y sólo tras la puerta de aceptación el estado pasa a `running`; 10 fallos con `running` MUST degradar a `error` | ✅ | `process.rs:240-330` (poller + puerta + 10 fallos), `process.rs:406` (500 ms), `process.rs:468-510` (puerta) |
| LM-MOT-5 | Si el proceso muere, el sistema MUST publicar `error` + `last_error` legible | ✅ | `process.rs:190-220` (poller: muerte del hijo → `error`); resumen actual en `process.rs:1230-1273` (`crash_summary`: ≤30 líneas, truncado a 600 chars + código de salida) |
| LM-MOT-6 | Con `idle_timeout_secs > 0` (default 1500 s) el motor MUST apagarse por inactividad **real**, verificando `/slots` para no cortar una generación en curso; `start()` MUST sembrar `last_activity` (defecto real corregido: antes quedaba en 0 y el auto-stop no podía disparar sin un request previo) | ✅ (D-20 cerrada por medición viva; defecto D-22 cerrado) | `process.rs:330-405` (rama idle + `/slots` + `last > 0` en `343-344`), `process.rs:1028-1029` (siembra en `start()`); evidencia viva: con `idle_timeout_secs=60` el motor pasó `running → stopped` al vencer, con la línea `[LocalMind] Auto-stop: motor apagado por inactividad…`, y el TOML del dueño se restauró byte-identical |
| LM-MOT-7 | Cada request al proxy MUST refrescar la marca de actividad (anti auto-stop); los slots ocupados también la refrescan | ✅ | `server.rs:1628`, `server.rs:1716`, `server.rs:1844` (los tres proxies), `process.rs:555-560` (`touch_activity`), `process.rs:341` (slots ocupados); verificado en vivo: la app responde (401 con clave ausente prueba el gate previo al touch) |
| LM-MOT-8 | `POST /api/stop` y el cierre de la app MUST matar el árbol de procesos (`taskkill /F /T`) | ✅ | `process.rs:383-384`, `process.rs:1051-1052` (`taskkill`) |
| LM-MOT-9 | Metas de rendimiento por perfil (t/s mínimos aceptables, incluido el objetivo declarado de 30 t/s a 262K) | 🟡 ver §4 | Medido en `tests/bench-results/*.json`; 32K supera el objetivo, 262K no (LM-NF-1/LM-NF-2) |
| LM-MOT-10 | Backends alternativos: P4 respondida "quedarse sólo Vulkan" — un build HIP/ROCm de llama.cpp para Windows (`bin-hip/`: `ggml-hip.dll`, `amdhip64_7.dll`, `rocm_kpack.dll`, `llama-server.exe` build 11175) lanzado en esta máquina expone CERO GPUs (`--list-devices` → `(none)`, fallback a CPU ≈1,2 tok/s medido en vivo) frente a Vulkan ≈30,17 t/s @32K; sin backend AMD viable no hay nada que conmutar | ✅ (decisión P4 cerrada por medición) | `bin-hip/llama-server.exe --version` → `version: 0.5.0-dev (build 11175, commit 4de092659)`; `bin-hip/llama-server.exe --list-devices` → `Available devices: (none)` (medido en vivo 2026-09-27) frente a `bin/llama-server.exe --list-devices` → `Vulkan0: AMD Radeon RX 6800 XT (16368 MiB…)`; Vulkan 32K en `tests/bench-results/bench-20260924-231526.json` (`gen_tps=30.1656`); número ROCm ≈1,2 tok/s medido en vivo (CPU fallback, sin fichero bench — el motor HIP nunca publicó GPU) |
| LM-MOT-11 | El perfil default MUST existir siempre y ser real (`velocidad`); un `last.profile` desconocido se sanea en carga (D-1 cerrada) | ✅ | `config.rs:148-150` (`DEFAULT_PROFILE_ID`), `config.rs:452-454` (saneado), `process.rs:770` (arranque); tests en `process.rs:1500-1516` |
| LM-MOT-12 | Tras `/health` OK, el arranque MUST superar una puerta de aceptación: una completion real de ≥20 completion tokens a ≥3 t/s (redondeo abajo); si no, estado `error` con `last_error` en español | ✅ | `process.rs:468-510` (`run_acceptance_gate`), veredicto en `process.rs:1134-1146`; observado en vivo (`acceptance_ok:true`, `decode_tps` 34,2 t/s @32K, `tests/bench-results/bench-20260924-234430.json`) |
| LM-MOT-13 | El estado MUST exponer progreso de arranque: `verifying`, `starting_for_secs`, `eta_secs` (duración guardada para la misma clave modelo\|contexto en `%APPDATA%/LocalMind/load-times.json`; en frío `6 s × GB del modelo`), más `decode_tps`, `acceptance_ok`, `acceptance_error` tras la puerta | ✅ | `process.rs:38-52` (campos), `process.rs:240-300` (progreso + ETA), `process.rs:511-514` + `process.rs:1147-1159` (registro y cálculo), `process.rs:283-297` (persistencia al superar la puerta) |

### 3.3 Configuración y perfiles
| ID | Requisito | Estado | Evidencia |
|---|---|---|---|
| LM-CFG-1 | La config MUST vivir en `%APPDATA%/LocalMind/localmind.toml` (fallback a `base_dir/localmind.toml`) | ✅ | `config.rs:422-477` (carga + ruta), `config.rs:493` (guardado) |
| LM-CFG-2 | Los defaults MUST cubrir todo el motor y quedar materializados en disco al primer arranque | ✅ | `config.rs:275-314` (defaults + `AppConfig::default`), `config.rs:458-460` (materialización) |
| LM-CFG-3 | Deben existir ≥4 perfiles integrados con contexto, `cache_ram` y flags: 32K/64K sin flags, 128K (`libros`) con `["--load-mode", "none"]` (cierre 128K medido, ver LM-NF-3), 262K con `-kvu`. Historial de secuencia (no se oculta): v1 `--no-mmap` → v2 reversión a `[]` (veredicto A/B prematuro) → v3 `--load-mode none` (grafía canónica, mismo efecto medido) | ✅ | `config.rs:390-443` (integrados; `libros` en `config.rs:409-427` con nota A/B 10546 tok); historial de versiones en `config.rs:152-158` (`PROFILES_VERSION = 3`) y fotos previas en `config.rs:446-460` |
| LM-CFG-4 | Migración segura `profiles_version = 3`: al cargar con versión menor, SOLO los integrados intactos se refrescan (una flag añadida por el usuario o un valor cambiado bloquea el refresh de ese perfil; ids desconocidos intactos), aplicada en `ConfigStore::load` con escritura atómica tmp+rename best-effort y línea `[LocalMind] Perfiles actualizados a v3: <ids>` vía `mgr.log` (= archivo P30 + SSE). El TOML vivo del dueño migró con diff de 2 hunks (`profiles_version` + `libros.extra_flags`) y se conservó `localmind.toml.bak-pre-v3-20260927-231019`. Legacy `turbo/balanced/deep/ultra` sigue migrándose preservando `last` (D-1) | ✅ | `config.rs:471-516` (regla `migrate_builtin_profiles_from`), `config.rs:555-586` (aplicación en `load` + atómica), `config.rs:607-621` (`take_migration_note`), `main.rs:102` (emisión vía `mgr.log`); legacy en `config.rs:439-457`; tests `mig_*` (11 en `config.rs:723-885`, `cargo test` 85/0) |
| LM-CFG-5 | Un TOML inválido MUST NOT fallar en silencio sin avisar al usuario | 🟡 D-9 | `config.rs:431-438` (`.ok()` en lectura+parse) |
| LM-CFG-6 | Los perfiles MUST poder exportarse/importarse por JSON | ✅ | `server.rs:508-548` (export + import) |
| LM-CFG-7 | Edición de perfiles desde la UI (tab Ajustes + `POST /api/profiles/save|delete`) | ✅ | `POST /api/profiles/save` en `server.rs:549-664` (valida id/contexto/cache_ram/flags con `config.rs:235-263`), `POST /api/profiles/delete` en `server.rs:665-706` (rechaza desconocido/último/en-uso); UI en `ui.html:1922-1977` (tab Ajustes) |
| LM-CFG-8 | Ubicación de config (APPDATA vs portable junto al exe) | ⬜ P27 | — |
| LM-CFG-9 | `GET /api/config` MUST exponer `{engine, generation, notifications}` y `POST /api/config` MUST aceptar un cuerpo parcial con las mismas secciones: claves desconocidas o fuera de rango → 400 nombrando el campo; `ubatch`/`device`/`llama_port`/`http_port` son de solo lectura; persistencia atómica (si el disco falla, la memoria se revierte) y los cambios de motor aplican al PRÓXIMO arranque | ✅ | `server.rs:755-944` (GET + POST con validadores `config.rs:210-233`), `server.rs:931-944` (atómico + mensaje de próximo arranque); verificado en vivo (round-trip de config y save+delete de perfil dejaron el TOML byte-identical; con motor apagado los launches no escriben) |
| LM-CFG-10 | Parámetros de generación (`temperature`, `top_p`, `max_tokens`, `seed`) MUST vivir en `generation` con defaults (0.7/1.0/2048/0) y validadores, y editarse desde Ajustes | ✅ | `config.rs:180-231` (struct + `gen_*_ok` + `default_*`), UI `ui.html:1977-2007` (tarjeta generación) |
### 3.4 Modelos
| ID | Requisito | Estado | Evidencia |
|---|---|---|---|
| LM-MOD-1 | El sistema MUST listar los `.gguf` utilizables (raíz + un nivel, sin `.cache`/`.verified`) con tamaño en GB | ✅ | `models.rs:737-790` (`list_gguf_files`), `process.rs:698-741` (pasarela `list_models`) |
| LM-MOD-2 | `mmproj` MUST ser opcional y estar apagado por defecto (sin visión) | ✅ | `config.rs:131-149` (sección), `process.rs:742-752` (`find_mmproj`); las configs de agentes declaran `[text]` honesto salvo mmproj verificado (`agents.rs:50-54`, `agents.rs:98-102`, `agents.rs:169-176`) |
| LM-MOD-3 | La importación MUST copiar el `.gguf` a `models/` | ✅ | `models.rs:791-820` (`copy_picked`); ruta legacy `POST /api/import_model` en `server.rs:707-729` (vía `process.rs:675-697`) |
| LM-MOD-4 | La importación MUST NOT exigir tipear una ruta absoluta a mano; el diálogo MUST acotarse a 120 s (`cancelled/timeout` al vencer) | ✅ (D-10 cerrada; D-18 cerrada en código) | `POST /api/models/import_pick` con diálogo nativo `rfd` (`*.gguf`, multiselección) y `recv_timeout(IMPORT_PICK_TIMEOUT_SECS=120)` en `server.rs:1133-1178`; constante y predicado en `models.rs:821-828`; UI en `ui.html:2970-2990` |
| LM-MOD-5 | MUST poder abrirse la carpeta de modelos desde la UI | ✅ | `server.rs:730-737`, botón en `ui.html:1745` |
| LM-MOD-6 | Gestor de modelos (descarga con progreso/verificación/cancelación + importación con diálogo nativo + listado + arranque) | ✅ | Superficie implementada en código: descarga (LM-MOD-9/10), `import_pick` (LM-MOD-4), listado con `rel`+verificación (LM-MOD-11), arranque de modelos anidados vía `rel` (LM-MOD-12). No implementado y fuera de este ID: borrado desde la UI y cuantizaciones alternativas (siguen ⬜ P6/P9) |
| LM-MOD-7 | El modelo fijado por el usuario (`last.model`) MUST respetarse al iniciar el motor | ⚠️ D-11 | `process.rs:780-800` (alias → primer `.gguf` → `last.model` en último lugar) |
| LM-MOD-8 | Multimodal (imagen) o voz (TTS/ASR) en el chat | ⬜ P7 | `bin/llama-tts.exe`, `bin/mtmd.dll` presentes y sin uso |
| LM-MOD-9 | La descarga MUST fijar una revisión (SHA de 40 hex, o el SHA de `main` vía la API de HF si se omite), listar el árbol en esa revisión y verificar tamaño en bytes (y `sha256` contra `lfs.oid` cuando el Hub lo publica; sin `lfs.oid` sólo tamaño); MUST resumir vía `.part` + `Range`, renombrar atómicamente tras verificar, y MUST NEVER escribir fuera de `models/` (se rechazan `..` y rutas absolutas). Destino: un solo `.gguf` queda PLANO en `models/<fichero>` (aunque el repo lo guarde en subcarpetas); multi-fichero → `models/<repo>/…` | ✅ (D-19 cerrada por medición viva, ver deuda) | `models.rs:140-215` (parse + validadores), `models.rs:256-290` (pin de revisión vía `https://huggingface.co/api/models/…`), `models.rs:402-423` (`dest_for`: plano vs repo), `models.rs:424-444` (`check_inside`), `models.rs:445-577` (worker con resume y verificación) |
| LM-MOD-10 | El estado de descarga MUST exponerse (`idle\|resolving\|downloading\|verifying\|done\|error`, `percent` entero 0–100, nunca NaN); un segundo POST con trabajo en curso MUST devolver 409; `POST …/cancel` MUST cancelar en el próximo trozo e informar `state:"error"`, `error:"cancelado"` | ✅ | `models.rs:15-24` (contrato), `models.rs:366-392` (`job_snapshot`/`job_busy`/`job_cancel`), `server.rs:1082-1132` (rutas) |
| LM-MOD-9b | La verificación fuerte quedó probada e2e con un GGUF real respaldado por LFS (`ggml-org/models` → `tinyllamas/stories260K.gguf` vía `POST /api/models/download`): el fichero aterrizó plano en `models/` con su SHA256 en disco igual al `lfs.oid` del árbol del Hub, y el fichero de prueba se eliminó después (directorio `models/` sin cambios) | ✅ (medición viva, no unitaria) | Ruta `server.rs:1082-1126`; verificación en `models.rs:445-577`; destino plano en `models.rs:402-423`; registros en `models/.verified/` (`models.rs:85-124`) |
| LM-MOD-11 | `GET /api/models` MUST listar raíz + un nivel de subdir (excluyendo `.cache`/`.verified`) y enriquecer cada ficha sin romper claves existentes: `size_bytes`, `sha256`, `verified`, `source` más `rel` (ruta relativa a `models/`; `filename` sigue siendo el basename y `path` la ruta absoluta) | ✅ | `models.rs:737-790` (`list_gguf_files`), `server.rs:439-470` (ficha con `rel`), `models.rs:832-853` (`enrich_model_entry`) |
| LM-MOD-12 | `POST /api/start` MUST aceptar el `rel` de `/api/models` además del basename para elegir el modelo; basename, `..`, rutas absolutas y con unidad MUST rechazarse con 4xx en español | ✅ | `process.rs:1168-1198` (`resolve_model_path`), usado en `process.rs:796-798` (arranque) y `process.rs:520-530` (ETA); tests `model_basename_resolves`, `model_rel_nested_resolves`, `model_traversal_rejected`, `model_absolute_rejected`, `model_unknown_returns_not_found` (`process.rs:1446-1490`) |

### 3.5 API local
| Método | Ruta | Función | Línea |
|---|---|---|---|
| OPTIONS | `*` (cualquier ruta) | Preflight CORS 200 (reflejo sólo loopback; `is_public_path` lo exime de clave) | `server.rs:330-333` (`is_public_path`), `server.rs:357-367` (handler) |
| GET | `/`, `/index.html` | Sirve `ui.html` (o fallback embebido) y fija la cookie `lm_key` | `server.rs:368-389` |
| GET | `/localmind.ico` | Recurso gráfico (404 `not_found` si falta) y cookie `lm_key` | `server.rs:390-411` |
| GET | `/localmind.png` | Recurso gráfico (404 `not_found` si falta) y cookie `lm_key` | `server.rs:412-438` |
| GET | `/api/models` | Lista de GGUF (raíz + 1 nivel) con `rel`, `size_bytes`, `sha256`, `verified`, `source` | `server.rs:439-470` |
| GET | `/api/version` | Versión app + `engine_build`/`engine_commit`/`engine_path` (nulos si el motor no responde) | `server.rs:471-483` |
| GET | `/api/logs/export` | Export del anillo en memoria (`text/plain` + `attachment`) | `server.rs:484-500` |
| GET | `/api/hardware` | CPU + GPUs detectadas | `server.rs:501-507` |
| GET | `/api/profiles` | Perfiles vigentes | `server.rs:508-514` |
| GET | `/api/profiles/export` | Descarga JSON de perfiles | `server.rs:515-524` |
| POST | `/api/profiles/import` | Reemplaza perfiles (400 si vacío/inválido) | `server.rs:525-548` |
| POST | `/api/profiles/save` | Crea/actualiza perfil validado | `server.rs:549-664` |
| POST | `/api/profiles/delete` | Borra perfil (rechaza desconocido/último/en-uso) | `server.rs:665-706` |
| POST | `/api/import_model` | Copia un `.gguf` a `models/` (ruta legacy; la UI usa `import_pick`) | `server.rs:707-729` |
| POST | `/api/open_models_dir` | Abre Explorer en `models/` | `server.rs:730-737` |
| GET | `/api/status` | Estado del motor (+ `verifying`, `starting_for_secs`, `eta_secs`, `decode_tps`, `acceptance_ok`, `acceptance_error`) | `server.rs:738-742` |
| GET | `/api/settings` | Última sesión, ruta de config, puertos | `server.rs:743-754` |
| GET | `/api/config` | Config viva `{engine, generation, notifications}` | `server.rs:755-760` |
| POST | `/api/config` | Config parcial validada (400 con campo, atómica, motor al próximo arranque) | `server.rs:761-944` |
| GET | `/api/metrics` | Proxy de `/metrics` del motor (502 si no corre) | `server.rs:945-980` |
| POST | `/api/start` | Inicia motor | `server.rs:981-1003` |
| POST | `/api/stop` | Detiene motor | `server.rs:1004-1009` |
| GET | `/api/logs` | Últimas ≤250 líneas en memoria | `server.rs:1010-1016` |
| GET | `/api/events` | SSE de logs (historial + vivo) | `server.rs:1017-1032` |
| POST | `/api/select_folder` | Diálogo nativo de carpeta | `server.rs:1033-1048` |
| POST | `/api/launch` | Lanzador genérico `{agent,cwd,effort,task}` (400 desconocido, 409 sin motor, 501 sin binario) | `server.rs:1049-1052`, `server.rs:1255-1328` |
| POST | `/api/launch_omp` | Reenvío fino a `launch_cli(omp)` (compat; la UI usa `/api/launch`) | `server.rs:1053-1057`, `server.rs:1329-1353` |
| POST | `/api/launch_pi` | Reenvío fino a `launch_cli(pi)` (compat) | `server.rs:1058-1062`, `server.rs:1329-1353` |
| GET | `/api/agents` | Fichas en orden fijo `pi, omp, opencode, web, deepseek` con `available` | `server.rs:1063-1067`, `launcher.rs:307-325` |
| POST | `/api/open_browser` | Abre una URL local en el navegador | `server.rs:1068-1081` |
| POST | `/api/models/download` | Inicia descarga HF `{"repo","revision?","files?"}` (400 validación, 409 si hay otra en curso) | `server.rs:1082-1126` |
| POST | `/api/models/download/cancel` | Cancela la descarga en curso | `server.rs:1127-1132` |
| POST | `/api/models/import_pick` | Diálogo nativo `rfd` + copia a `models/` (120 s, `cancelled/timeout`) | `server.rs:1133-1178` |
| GET | `/api/usage` | Resumen incremental de uso (sin offset interno) | `server.rs:1179-1183` |
| GET | `/api/usage/raw?limit=n` | Últimas `limit` líneas como `{lines, skipped}` (máx 500; acepta `?limit=`) | `server.rs:1184-1204` |
| GET | `/v1/models`, `/v1/models/` | Lista OpenAI (`object:list`, `data[]` con id servido + alias) | `server.rs:1205-1212` |
| POST | `/v1/chat/completions` | Proxy OpenAI con streaming incremental + registro de uso | `server.rs:1213-1218`, `server.rs:1620-1704` |
| POST | `/v1/messages` | Proxy Anthropic Messages (traducción ida y vuelta) | `server.rs:1219-1224`, `server.rs:1705-1835` |
| POST | `/v1/responses` | Proxy OpenAI Responses (traducción ida y vuelta) | `server.rs:1225-1229`, `server.rs:1836-1943` |
| * | otro | `404 {"error":"not_found"}` | `server.rs:1230` |
| ID | Requisito | Estado | Evidencia |
|---|---|---|---|
| LM-API-1 | El servidor MUST usar puerto preferido 17860 con hasta +10 libres, y fallar con mensaje claro si no hay ninguno | ✅ | `server.rs:281-300` (`HttpServer::start`), `config.rs:93-94` |
| LM-API-2 | Toda la API MUST ser loopback | ✅ | `server.rs:289` (`Server::http(("127.0.0.1", p))`) |
| LM-API-3 | `/api/metrics` MUST devolver 502 `engine_down` salvo motor sano y en ejecución | 🟡 D-3 | `server.rs:945-980` (puerta 502 en `948`/`975`) |
| LM-API-4 | La semántica de `/api/open_browser` MUST quedar definida (¿UI de llama.cpp en el puerto del motor, o UI de LocalMind?) | ⬜ P25 | `server.rs:1068-1081`, `server.rs:1234-1244` (`open_browser_action`); UI en `ui.html:2140` (`launch.webDesc`) |
| LM-API-5 | Errores de entrada (JSON inválido, perfiles vacíos) MUST devolver 4xx, no `panic` | ✅ (rutas nuevas; D-12 residual en parsers legacy) | `server.rs:1082-1126` (400 en download), `server.rs:1722` (400 Anthropic JSON), `server.rs:767-774` (400 config), `models.rs:24` (sin `unwrap` en request/IO/JSON) |
| LM-API-6 | `/v1/*` y `/api/*` MUST exigir la clave local (`Authorization: Bearer`, `x-api-key` o cookie `lm_key`); sin credencial válida MUST devolver 401 `{"error":"unauthorized"}`; públicas sólo `GET /`, `/index.html`, `/localmind.ico`, `/localmind.png` (que fijan la cookie) y `OPTIONS *` | ✅ (D-7 cerrada; verificado en vivo: la app respondió 401 sin clave en `/api/status`, `/api/config`, `/api/agents`, `/v1/models`) | `server.rs:330-339` (`is_public_path`), `server.rs:434-436` (puerta), `server.rs:77-79` (401), `auth.rs:91-123` (`is_authorized`), `auth.rs:59-79` (creación + cookie) |
| LM-API-7 | La clave MUST generarse (32 bytes aleatorios en hex) en el primer arranque y persistir en `%APPDATA%\LocalMind\gateway.key` (best-effort; en Windows sin tocar ACLs) | ✅ | `auth.rs:59-79` |
| LM-API-8 | `GET /api/usage` MUST devolver el resumen incremental `{totals, by_model, by_day, first_ts, last_ts}` parseando sólo líneas nuevas desde el offset de `%APPDATA%\LocalMind\usage-summary.json` (rebuild si el log rotó); `GET /api/usage/raw?limit=n` MUST devolver `{lines, skipped}` con las últimas `n` válidas (máx 500, inválidas contadas en `skipped`) | ✅ | `usage.rs:169-285` (resumen), `usage.rs:295-315` (raw); `server.rs:1179-1204` |
| LM-API-9 | `GET /api/version` MUST devolver `{app, engine_build, engine_commit, engine_path}` (app desde `CARGO_PKG_VERSION`; motor consultado una vez a `bin/llama-server.exe --version` con timeout de 5 s y caché en memoria; campos del motor `null` si no responde — la ruta nunca falla) | ✅ | `server.rs:471-483` (ruta), `meta.rs:83-155` (`engine_info`/`query_engine`), `meta.rs:25-52` (parser); hallazgo real: llama-server imprime su versión en STDERR, por eso el parser une ambos streams (`meta.rs:106-107`, `meta.rs:142-146`); test `version_real_de_llama_server` (`meta.rs:278-293`) |
| LM-API-10 | `GET /api/logs/export` MUST devolver el anillo en memoria como `text/plain; charset=utf-8` con `Content-Disposition: attachment; filename="localmind-logs-<sello>.txt"` (sello UTC `YYYYMMDD-HHMMSS` sin crates de fecha) | ✅ | `server.rs:484-500` (ruta), `meta.rs:249-271` (`utc_stamp`/`utc_stamp_now`); test `sello_utc_conocido` (`meta.rs:302`) |
| LM-API-11 | CORS MUST NOT usar wildcard en ningún lado: `Access-Control-Allow-Origin` se refleja SÓLO para orígenes loopback (`http://127.0.0.1|localhost|[::1][:puerto]`); `OPTIONS` sigue respondiendo 200 para ellos; orígenes de terceros no reciben cabeceras CORS (P29, segunda mitad, cerrada) | ✅ | `meta.rs:162-228` (`request_origin` + `is_loopback_origin`), `server.rs:35-89` (fijas + reflejo + `add_cors_for`), `server.rs:357-367` (preflight), `server.rs:353-356` (origen por request); tests `origenes_loopback_aceptados` + `origenes_ajenos_rechazados` (`meta.rs:310-332`) |

### 3.6 Proxy OpenAI
| ID | Requisito | Estado | Evidencia |
|---|---|---|---|
| LM-PXY-1 | `POST /v1/chat/completions` MUST reenviar al motor y ceder cada chunk al cliente en cuanto llega (`text/event-stream`), extrayendo el `usage` al EOF desde una cola acotada de 64 KiB | ✅ | `server.rs:1620-1704` (chat con tee), `server.rs:1983` (`TeeLogReader`), `usage.rs:64-97` (cola + extracción) |
| LM-PXY-2 | El payload MUST normalizarse en `reasoning_effort` en LOS TRES proxys (`sanitize_payload` antes de reenviar): `minimal`/`low` → `low`, `high`/`max`/`xhigh` → `xhigh`, `medium` → `medium`; desconocido o no-string → campo ELIMINADO (el motor usa su default). Motivo medido: la plantilla Qwen rechaza `max` con HTTP 500 | ✅ | `server.rs:206-245` (`sanitize_payload` + `normalize_reasoning_effort`), llamado en `server.rs:1687` (chat), `server.rs:1794` (messages), `server.rs:1909` (responses); test `reasoning_effort_max_a_xhigh_y_desconocido_se_elimina` (`server.rs:2328-2366`); prueba viva en el binario shippeado (`db1a1b8f…`, 3551232 B): triple `max`/`minimal`/`medium` → 200 con `reasoning_content`, prompts 56/44/14 (`docs/agents/harness-comparison-128k.md:228-236`) |
| LM-PXY-3 | Motor caído (`port == 0`) MUST devolver 502 `engine_down` (chat) o el error en forma Anthropic (`api_error`), sin colgar la UI | ✅ | `server.rs:1632`, `server.rs:1737` (chat/Anthropic 502), `server.rs:1856` (Responses 502); verificado por `node tests/e2e.mjs` (`GET /api/metrics engine-off 502`) |
| LM-PXY-4 | `GET /v1/models` MUST listar el id servido por el motor más los alias estables (`localmind`, `qwen3.8-27b`) en forma OpenAI (`object:list`, `data[]` con `id`/`object`/`owned_by`); cualquier `model` pedido en `/v1/*` MUST reescribirse al id servido (nunca se rechaza un alias) | ✅ (D-2 cerrada por diseño) | `server.rs:236-270` (`served_model_id` + `models_list_json`), `server.rs:217-230` (`rewrite_model_to_served`), `server.rs:1205-1212` (ruta); verificado por `node tests/e2e.mjs` (`GET /v1/models ids`) y `node tests/smoke.mjs` (check 13) |
| LM-PXY-5 | Endpoints OpenAI adicionales: `POST /v1/messages` (Anthropic) y `POST /v1/responses` (Responses) MUST traducir el request a chat/completions, reenviar al motor y traducir la respuesta de vuelta (streaming por eventos y envoltorio no-streaming; errores en la forma del protocolo) | ✅ | `translate.rs:40-178` (Anthropic→OpenAI), `translate.rs:179-280` (OpenAI→Anthropic + errores + SSE), `translate.rs:486-597` (Responses→OpenAI), `translate.rs:598-749` (SSE + envoltorio Responses); handlers en `server.rs:1705-1835` y `server.rs:1836-1943`; orden de eventos verificado por `node tests/e2e.mjs --with-engine` (≥2 deltas + usage por protocolo) |
| LM-PXY-6 | Cada request proxyeado completado MUST añadir una línea JSONL `{ts, endpoint, model, prompt_tokens, completion_tokens, ms, stream}` a `%APPDATA%\LocalMind\usage.jsonl` (lo disponible si el usage es parcial, jamás inventado; `stream_options.include_usage` inyectado si el cliente no lo fijó); si el archivo supera 1 MiB se conservan sus últimas 5000 líneas | ✅ | `usage.rs:14-20` (ruta + override), `usage.rs:43-54` (inyección), `usage.rs:107-148` (append + rotación), `server.rs:1681`, `server.rs:2017`, `server.rs:2120` (chat no-stream + tees), `server.rs:1804` + `server.rs:1915` (messages/responses); línea verificada contra el request observado por `node tests/e2e.mjs --with-engine` (`usage.jsonl last line matches`) |

### 3.7 Interfaz gráfica
| ID | Requisito | Estado | Evidencia |
|---|---|---|---|
| LM-UI-1 | La app MUST operarse sin abrir una terminal (los logs viven dentro de la tab Logs, no en una consola) | ✅ | `main.rs:90-171` (ventana + headless); §2.1.1; `docs/ui/notes.md` |
| LM-UI-2 | MUST existir una franja de estado persistente con acción Iniciar/Detener desde cualquier pestaña | ✅ | `ui.html:1444-1447` (franja + countdown), `ui.html:4113-4164` (render de estado) |
| LM-UI-3 | MUST poder elegirse modelo, perfil, contexto (32K/64K/128K/262K), hilos y prioridad | ✅ | `ui.html:1731` (selector de modelo), `ui.html:1919-1977` (editor de perfiles en Ajustes), `ui.html:2012-2037` (motor) |
| LM-UI-4 | MUST mostrarse telemetría en vivo: t/s generación, t/s prompt, tokens, %cache, %MTP, requests, PID | ✅ | `ui.html:1622-1650` (cajas `tel-*`), `ui.html:2662-2672` (render) |
| LM-UI-4b | La telemetría MUST mostrar la velocidad verificada por la puerta de aceptación (`Velocidad verificada: N t/s`) cuando `decode_tps` es numérico, y el banner de error de aceptación cuando `acceptance_ok === false` | ✅ | `ui.html:1644-1650` (cajas decode/accept), `ui.html:4217-4233` (render + banner) |
| LM-UI-5 | MUST existir chat embebido con streaming (SSE + `AbortController`), bloque de *thinking* colapsable, detener y copiar código | ✅ | `ui.html:1672-1682` (mensajes + input), `ui.html:1119-1141` (thinking), `ui.html:2065` (contrato SSE), `ui.html:4381-4411` (stream) |
| LM-UI-5b | La franja de estado MUST mostrar `Cargando… N% (ETA …)` durante la carga y `Verificando GPU…` durante la puerta de aceptación | ✅ | `ui.html:2082-2086` (i18n) + `ui.html:4113-4141` (render) |
| LM-UI-6 | El historial de chat MUST sobrevivir reinicios | ✅ | `ui.html:2583-2593` (`localmind_chat_v1`) |
| LM-UI-7 | Los logs MUST verse en vivo (SSE) con copiar/limpiar | ✅ | `ui.html:1841` (`terminal-stream`), `ui.html:1837-1838` + `ui.html:2824-2830` (copiar/limpiar) |
| LM-UI-8 | MUST existir vista de specs (hardware, flags del motor, URL OpenAI, ruta de config) | ✅ | `ui.html:1850-1863` (specs), `ui.html:2956` (`/api/hardware`), `ui.html:4058-4071` (`refreshVersion` + estado) |
| LM-UI-9 | Los controles MUST bloquearse mientras el motor corre/arranca y los lanzadores habilitarse sólo si está sano (`btn-launcher-open` deshabilitado) | ✅ | `ui.html:1594` (botón deshabilitado), `ui.html:3742-3846` (lógica `onLauncherAgentChange`/`openLauncherAgent`) |
| LM-UI-10 | Parámetros de generación editables desde Ajustes (P16 ✅ para el contrato implementado): `temperature/top_p/max_tokens/seed` con validadores + `POST /api/config` + tarjeta en Ajustes | ✅ | `config.rs:182-231`, `server.rs:755-945`, `ui.html:1977-2007` |
| LM-UI-11 | Accesibilidad: `alert()` bloqueantes eliminados (0 ocurrencias; notificaciones inline vía `toast()` en `#toast-region` con `aria-live`), thinking-toggle como `<button>` real, `role="status"` en franja/telemetría/descargas, `:focus-visible`, `aria-label`s en botones de icono, tabs por teclado | ✅ | `ui.html:1454` + `ui.html:671-692` (región + estilos toast), `ui.html:2569-2580` (`toast()`), 0 ocurrencias de `alert(`; `docs/ui/notes.md`; `node docs/ui/contrast.mjs` (17/17 pares) |
| LM-UI-12 | El chat embebido MUST declarar si soporta adjuntos (hoy sólo texto) | 🟡 P19 | Sin cambios (conflicto P7↔P19 abierto, ver §11) |
| LM-UI-13 | Rediseño completo sobre un sistema visual nuevo (P18=C): tokens, shell con header + badge de versión (`/api/version`, degrada a `local`) + chip de motor + 7 tabs (Panel/Chat/Modelos/Uso/Logs/Specs/Ajustes), contraste 17/17, capturas es/en + `live-*`/`round4-*` | 🟡 scaffold verificado (aceptación visual real = decisión del dueño, D-27) | `ui.html` (~3.904 líneas), `docs/ui/notes.md`, `docs/ui/contrast.mjs`, `docs/ui/capture.ps1`, `docs/ui/screens/`; `node tests/ui-render.mjs` (43 checks) |
| LM-UI-14 | Bilingüe es/en (P17=C): diccionario inline `I18N` + `T(key)` + `setLanguage`/`applyI18n`, switch persistido en `localStorage localmind_lang` (default `es`), conmutación de `<html lang>`, deep-link `#tab=&lang=` | 🟡 scaffold verificado (aceptación visual real = decisión del dueño) | `ui.html:2073` (`I18N`), `ui.html:2477-2519` (`LANG_KEY` + `T` + `setLanguage` + `applyI18n` + deep-link), `ui.html:4641` (switch); `docs/ui/screens/` (pares *-es/-en por tab) |
| LM-UI-15 | Tab Ajustes (ronda 4 ✅): editor de perfiles (`save`/`delete` con validación inline), parámetros de generación, toggles de motor y de notificaciones, marca LM propia (un barrido completo probó que no queda logo/sol abajo-izquierda en el layout actual) | ✅ | `ui.html:1919-2041` (tab Ajustes: perfiles, generación, motor, notificaciones), `ui.html:1384-1392` (marca LM: rectángulo + `M`), `ui.html:1579-1595` (selector único + Abrir); `node tests/ui-render.mjs` (43 checks, incl. ausencia de orca/sol) |

### 3.8 Lanzadores de agentes CLI
| ID | Requisito | Estado | Evidencia |
|---|---|---|---|
| LM-CLI-1 | OMP MUST lanzarse contra el GATEWAY ligado (`OPENAI_BASE_URL=http://127.0.0.1:<http_port>/v1` + `OPENAI_API_KEY` desde `gateway.key`), `--model localmind/qwen3.8-27b` y `--thinking <nivel>`, con `PI_CODING_AGENT_DIR` al dir privado | ✅ | `launcher.rs:370-400` (`cli_inner_cmd`: gateway + clave), `server.rs:1386-1486` (`launch_cli`); `AGENT_ORDER` en `launcher.rs:22` |
| LM-CLI-2 | Pi MUST lanzarse contra el GATEWAY ligado con `--provider localmind --model localmind/localmind --thinking <nivel>` | ✅ | `server.rs:1053-1062` (rutas compat), `server.rs:1386-1486` (núcleo) |
| LM-CLI-3 | El spawn de terminal MUST verificarse: `wt.exe` y, si su spawn devuelve `Err` (alias de ejecución de 0 bytes en `%LOCALAPPDATA%\Microsoft\WindowsApps`), fallback a `cmd /c start`; se registra pid o error y si ambas ramas fallan el endpoint responde 500 en español (antes un `let _ = spawn()` silencioso hacía pasar un fallo por 200). Orca eliminado: cero ocurrencias en el código | ✅ (defecto corregido; Orca fuera por decisión del dueño) | `server.rs:1362-1384` (`wt_command` + comentario del alias + `cmd_start_command`), `launcher.rs:471-530` (`SpawnBranch`, `spawn_terminal`), `server.rs:1458-1479` (uso + 500); `launch_opencode` igual en `server.rs:1592-1611` |
| LM-CLI-4 | El usuario MUST poder fijar la carpeta de proyecto, persistente entre sesiones, y el CLI MUST abrirse allí (selector único de agente + "Abrir" en la UI) | ✅ | `server.rs:1033-1048` (`select_folder`), `ui.html:1579-1595` (selector + Abrir), `ui.html:2067` (`localmind_project_cwd`), `ui.html:3674` (persistencia) |
| LM-CLI-5 | El id de modelo pedido a los CLIs MUST ser uno de los alias estables y el proxy MUST reescribirlo al id servido (D-2 cerrada por diseño: ya no importa si el CLI pide otro id) | ✅ | `agents.rs:36-42`, `server.rs:217-230`; verificado por `node tests/e2e.mjs --with-engine` (omp/pi responden "ok" por el proxy) |
| LM-CLI-6 | Lanzar sin motor vivo MUST devolver 409 con el motivo y MUST NOT escribir configs (D-4 cerrada) | ✅ | `server.rs:1398`, `server.rs:1565` (409), `agents.rs:21-34` (`engine_live` + `engine_down_error`); verificado por `node tests/e2e.mjs` (`POST /api/launch_omp engine-off 409`, `launch-off writes nothing`) |
| LM-CLI-7 | `POST /api/launch {agent,cwd,effort,task}` + `GET /api/agents` (orden fijo `pi, omp, opencode, web, deepseek`, todos `available:true` en esta máquina); `launch_omp`/`launch_pi` quedan como reenvíos finos de compatibilidad | ✅ | `server.rs:1049-1066` (rutas), `server.rs:1255-1345` (`handle_launch_generic` + compat), `launcher.rs:22` + `launcher.rs:307-325` (orden + fichas + `available`); 400 agente desconocido en `server.rs:1276`, `server.rs:1347` |
| LM-CLI-8 | OpenCode (harness `opencode-ai` v1.18.10, npm global) MUST lanzarse con `OPENCODE_CONFIG_CONTENT` inyectado por env (puerto/contexto vivos, clave por `{env:LOCALMIND_API_KEY}` nunca cruda) y dirs XDG aislados en el dir privado; sin motor 409, sin binario 501; verificado respondiendo del modelo local | ✅ | `launcher.rs:5-7`, `launcher.rs:80`, `launcher.rs:434-440` (env), `launcher.rs:144-156` (XDG), `server.rs:1547-1617` (`launch_opencode`); `docs/agents/opencode-harness.md:9-10`, `:26-27` (versión), `:81` (one-shot `HOLA-LOCAL`, exit 0, ≈10,5 s), `:87-90` (2 requests gateway: 597+12.281 prompt / 242+38 completion; segunda pata ≈32,9 t/s) |
| LM-CLI-9 | DeepSeek (harness oficial `@deepseek-ai/dsh` v0.1.5-rc.3, `dsh --profile headless`, `DSH_HOME` aislado) MUST apuntar al gateway con patch Cordis generado con valores vivos; verificado respondiendo del modelo local | ✅ | `launcher.rs:10-11`, `launcher.rs:137-243` (`DSH_HOME`, `deepseek_env`, patch + escritura atómica), `server.rs:1487-1540` (`launch_deepseek`); `docs/agents/deepseek-harness.md:8`, `:28` (versión), `:85-99` (runs A/B + líneas `usage.jsonl` con `model:"localmind"`), `:106` (run A: 55 s / 9.709 prompt / 53 completion ≈1,0 t/s) |
| LM-CLI-10 | Integración 128K por ruta de la app (ronda 7 ✅): los cuatro harnesses (`pi`, `omp`, `opencode`, `deepseek`) devuelven `POST /api/launch` 200 con la carga `libros`/131072 viva, spawn real registrado (línea de log con pid) y su config privada creada apuntando al origen gateway con puerto vivo + contexto 131072 (pi `models.json`, omp `models.yml`, opencode `config/opencode/opencode.json`, deepseek `profiles/headless/cordis.patch.yml`); para los one-shot (opencode/deepseek) el tráfico por el gateway quedó en `usage.jsonl`; para los interactivos (pi/omp) la tarea se ignora por diseño (`server.rs:1301-1302`) y la prueba es config+spawn+200 | ✅ | `server.rs:1246-1353` (genérico + compat), `server.rs:1490-1511` (logs con pid: `Terminal {} lanzada ({} {})` / `ERROR al lanzar`), `agents.rs:189-203` (pi/omp), `launcher.rs:445-448` (opencode.json), `launcher.rs:220-243` (patch deepseek); `server.rs:1301-1302` (task ignorada pi/omp); evidencia en `docs/agents/harness-comparison-128k.md:228-236` (4×200, spawns, configs con `:17860`+131072, `usage.jsonl` one-shot) |

### 3.9 Integración con CLIs de terceros
| ID | Requisito | Estado | Evidencia |
| LM-INT-1 | Las configs de agentes MUST vivir en dirs privados `%APPDATA%\LocalMind\agents\<agent>\` (pi `models.json`, omp `models.yml` + `config.yml` mínimo con `modelRoles.default: localmind/localmind`), escritas sólo al lanzar, con puerto/contexto/clave VIVOS; `%USERPROFILE%\.pi` y `%USERPROFILE%\.omp` MUST NEVER leerse ni escribirse (D-5 y D-6 cerradas por diseño) | ✅ | `agents.rs:1-13` (contrato), `agents.rs:44-48` (`agent_dir`), `agents.rs:55-101` (pi), `agents.rs:102-181` (omp + config), `agents.rs:182-203` (escritura), `launcher.rs:370-400` (`OPENAI_BASE_URL` gateway + `PI_CODING_AGENT_DIR`); verificado por `node tests/e2e.mjs --with-engine` (yml con clave+contexto vivo, `home agent files byte-identical`) |
| LM-INT-2 | (Retirado: la sobrescritura de `~/.omp/agent/models.yml` ya no existe; ver LM-INT-1 y D-5 cerrada) | ✅ | `agents.rs:3`, `server.rs:8-9` |
| LM-INT-3 | (Retirado: el puerto stale ya no existe; ver LM-INT-1 y D-6 cerrada) | ✅ | `agents.rs:8-9`, `server.rs:1421` (puerto vivo del `launch_cli`) |
| LM-INT-4 | La escritura MUST ocurrir al lanzar el CLI (y sólo entonces) | ✅ | `server.rs:1421` (`write_agent_dir` en `launch_cli`), `server.rs:1495-1506` (patch deepseek en `launch_deepseek`) |
| LM-INT-5 | Debe existir respaldo/rollback antes de tocar configs ajenas | ⬜ P21 | Sin objeto: ya no se tocan configs ajenas (LM-INT-1) |

### 3.10 Telemetría y diagnóstico
| ID | Requisito | Estado | Evidencia |
|---|---|---|---|
| LM-TEL-1 | La telemetría MUST leer `llamacpp:*` de `/metrics` y exponer un subconjunto estable | ✅ | `server.rs:84-125` (`parse_metrics`), `server.rs:485-520` (ruta actual) |
| LM-TEL-2 | Los logs MUST retenerse en memoria (250 líneas) y transmitirse por SSE (anillo sin cambios) | ✅ | `process.rs:338`, `process.rs:467`, `process.rs:490`, `process.rs:503` (retención 250 + suscripción), `server.rs:560-572` (ruta SSE), `server.rs:1242-1250` (headers) + `server.rs:1561-1574` (`EventReader`) |
| LM-TEL-3 | Los logs MUST persistir a archivo con rotación (P30 ✅): cada línea de `mgr.log()` también en `%APPDATA%\LocalMind\logs\localmind.log` con sello UTC, rotando a 1 MiB × 3 ficheros; best-effort (nunca falla al motor); el export ya existía (`GET /api/logs/export`) | ✅ | `filelog.rs:3-7` (1 MiB × 3), `filelog.rs:16-24` (UTC + saneado), `filelog.rs:26-61` (rotación + append best-effort), `process.rs:145` (ruta), `process.rs:598` (`log()`), `process.rs:218`, `process.rs:368`, `process.rs:400` (hilos); tests en `filelog.rs:75-114` |
| LM-TEL-4 | Los errores del motor MUST quedar registrados con contexto suficiente para diagnosticar | ✅ | `process.rs:1112-1155` (`crash_summary`) |
| LM-TEL-5 | El uso MUST registrarse por request en `%APPDATA%/LocalMind/usage.jsonl` (1 línea JSON, rotación 1 MiB → últimas 5000) y resumirse vía `GET /api/usage` + `GET /api/usage/raw`; `cargo test` MUST NOT contaminar el fichero real (override `LOCALMIND_USAGE_PATH` a scratch) | ✅ | `usage.rs:14-18` (override) + `usage.rs:350-355` (test del override), `usage.rs:99-148` (registro), `usage.rs:151-315` (resumen/raw); UI en `ui.html:1693-1732`, `ui.html:2104-2191` |
| LM-TEL-6 | El tráfico de agentes MUST pasar por el gateway ligado (nunca el motor crudo) para que la contabilidad de tokens lo cubra: un one-shot contra el puerto crudo del motor produce CERO líneas de uso, mientras la misma tarea por el gateway produce líneas `model:"localmind"` | ✅ | `launcher.rs:207-212` (deepseek: gateway, nunca motor), `launcher.rs:372` + `launcher.rs:402-415` (pi/omp/opencode: gateway), `server.rs:1380-1400` (`launch_cli` gateway); contraste medido en `docs/agents/opencode-harness.md:87-97` (puerto crudo: ninguna línea; gateway: `{"model":"qwen3.8-27b",…242…}` + `{"model":"qwen3.8-27b",…38…}`) y `docs/agents/deepseek-harness.md:91-99` (`{"model":"localmind",…9709/53…}` y 4 líneas más del run B) |

### 3.11 Distribución y mantenibilidad
| ID | Requisito | Estado | Evidencia |
|---|---|---|---|
| LM-DST-1 | El repositorio MUST NOT contener pesos ni binarios de terceros | ✅ | `.gitignore`; `models/`, `bin/`, `bin-hip/`, `dist/` y `*.bak*` fuera de git |
| LM-DST-2 | Entrega portable + instalación (P5 parcial): `build-portable.ps1` arma el ZIP sin `models/target/.git/.bak`; `install.ps1` instala en `%LOCALAPPDATA%\Programs\LocalMind` con accesos, sin pisar un exe en marcha y sin borrar `models/`; `uninstall.ps1` exige `-Force` en marcha y conserva modelos salvo `-RemoveModels`; sin firma de código ni auto-update | 🟡 (P5 parcial) | `packaging/build-portable.ps1:1-121` (ZIP + exclusiones), `packaging/install.ps1`, `packaging/uninstall.ps1`, `packaging/README.md` (alcance + límites); ZIP observado `dist/LocalMind-portable-2.0.0-20260925.zip` (111 ficheros, ~39 MB) |
| LM-DST-3 | Debe declararse la versión del build de llama.cpp usado | ✅ (vía API, no marcador en `bin/`) | `GET /api/version` → `engine_build`/`engine_commit`/`engine_path` (`meta.rs:83-155`); badge en la UI (`docs/ui/notes.md`); D-15 cerrada por implementación |
| LM-MNT-1 | MUST existir suite de tests automatizados | ✅ (CI 🟡 P31, ver fila) | `cargo test` (85 passed / 0 failed, verificado en vivo 2026-09-27 — ver §9), `tests/smoke.mjs` (14 checks), `tests/e2e.mjs` (6 checks motor-apagado), `tests/bench.mjs --self-test` (23 asserts), `tests/harness-bench.mjs` (`--fair/--thinking/--warmup/--dry-run`, 9 resultados 128K), `tests/pressure-sampler.mjs` (muestreo 5 s), `tests/ui-render.mjs` (43 checks); ver §9 |
| LM-MNT-2 | MUST existir remoto git con historia publicada | ⬜ P31 | `git remote -v` vacío (sin remoto: Actions jamás ejecutado, ver D-23) |
| LM-MNT-3 | El SRS MUST versionarse junto al código y actualizarse por fase | ✅ | este documento (v1.5) |
| LM-MNT-4 | CI en Windows (`cargo test` + harnesses offline + `node --check tests/*.mjs`) | 🟡 (workflow existe; Actions jamás ejecutado sin remoto; todos sus comandos corren en verde en local) | `.github/workflows/ci.yml`; smoke/e2e excluidos a propósito (necesitan la app viva) |

---

## 4. Requisitos no funcionales

| ID | Requisito | Objetivo | Estado | Evidencia |
|---|---|---|---|---|
| LM-NF-1 | Rendimiento de generación a 32K | Objetivo 30 t/s del dueño (§2.1.3): **superado** con 30,17 t/s medidos | ✅ | `tests/bench-results/bench-20260924-231526.json` (`32k gen_tps=30.1656`, carga 24,5 s); puerta de aceptación del mismo perfil: `decode_tps` 34,1 t/s (`bench-20260924-234430.json`) |
| LM-NF-2 | Rendimiento efectivo a 128K (ronda 7, máquina quieta CPU 5,7 %, protocolo fair `--thinking low`+warmup+orden inverso): **ningún harness llega a 30 t/s efectivo** — omp eff **17,3** (gen 18,8, 1 req), pi **11,8** (15,3, 14 req), opencode **2,4** (18,3, 3 req), deepseek **5,6** (17,6, 2 req); control directo por el gateway: primer byte 159,4 s, decode **25,0 t/s** (prompt 1.168 / completion 1.208). **Techo de decode del motor en esa carga ≈25 t/s** | 🟡 | Fair: `tests/harness-bench/results/2026-09-25T17-54-26-616Z.json` (tabla `pi 11,8/15,3 · omp 17,3/18,8 · opencode 2,4/18,3 · deepseek 5,6/17,6`); análisis en `docs/agents/harness-comparison-128k.md:60-100` (tabla fair + patas por request + control 25,0 t/s). A 262K el objetivo 30 t/s sigue NO alcanzado (14,16 t/s, `bench-20260924-231729.json`) |
| LM-NF-2b | Diagnóstico del techo (ronda 7): el gap entre el probe corto caliente (94,6 t/s decode, prompt 300) y el control 128K real (25,0 t/s, prompt 1.168/completion 1.208) es prefill + profundidad real de contexto; a 262K el barrido `--cache-ram` sigue plano (0→14,10 · 4096→14,18 · 6144→14,16 · 8192→14,20; sin `-kvu` 11,48). Prompt-eval con el mismo prompt de 12,3k: 196 tok/s a 32K frente a 39,6 tok/s a 128K (dato de sesión 2026-09-28, ~5× con la profundidad); comparativa 64K-vs-128K con tabla fair **pendiente de medición** (la tabla fair 64K aún no está corrida: no afirmar paridad hasta medirla) | 🟡 medido, causa raíz [INFERENCE] | 128K: `docs/agents/harness-comparison-128k.md:100-110` (94,6 vs 25,0 + conclusión prefill+profundidad); 262K: `config.rs:276-284`. 30 t/s efectivo a 128K/262K no es alcanzable en este hardware sin cambiar de estrategia [INFERENCE] |
| LM-NF-3 | Cierre 128K (medido, `engine/print_timing` sobre el mismo prompt de 10546 tok a 128K): mmap default → prefill **21,65 t/s (463 s)**, decode 14,01; `--load-mode none` → prefill **38,18 t/s (263 s, +76 %)**, decode 13,92 (≈ruido), commit 89,3→87,7 %, libre 620→991 MB. El perfil `libros` lleva la grafía canónica (ver LM-MOT-2/LM-CFG-3). Historia causal honesta: RAM/commit era la presión dominante y CPU quedó descartada; `cache_ram` y `threads` quedaron descartados como palancas (rondas B/C); la palanca que movió el prefill fue el modo de carga. Quedan como rondas pendientes (no medidas): `threads 6→4` como headroom y `--no-mmap` ya superado por el canónico | ✅ medido | `docs/agents/engine-pressure.md:156-168` (§9 A/B + override del dueño), nota en `config.rs:414-425`; `--help` del build (`bin/llama-server.exe --help`: `--mmap, --no-mmap` DEPRECATED in favor of `--load-mode`; modos `auto/none/mmap/mlock/mmap+mlock/dio`) |
| LM-NF-4 | Privacidad | Ningún dato sale de la máquina salvo la descarga de modelos; sin telemetría externa | ✅ | Loopback: `process.rs:431`, `process.rs:440`, `process.rs:477`, `server.rs:951`, `server.rs:1650`, `server.rs:1757`, `server.rs:1823` (todos `http://127.0.0.1:`); internet sólo a `https://huggingface.co/api/models/…` (`models.rs:256-290`) por decisión P6 |
| LM-NF-5 | Latencia percibida | Feedback inmediato al iniciar motor: `%` de progreso + ETA + fase de verificación visibles en la franja de estado | ✅ | `process.rs:240-300` (progreso+ETA), `ui.html:2082-2086` (i18n `strip.loading/verifying/loadPct`) + `ui.html:4113-4141` (render); countdown de auto-stop en `ui.html:4107-4108` |
| LM-NF-6 | Seguridad local: `/api/*` y `/v1/*` MUST exigir la clave local y CORS MUST NOT usar wildcard (reflejo sólo loopback) | ✅ | `auth.rs:1-123`, `server.rs:330-339` (`is_public_path`) + `server.rs:434-436` (puerta 401 en `77-79`), `meta.rs:162-228` + `server.rs:35-89` (CORS estricto); P29 cerrada por implementación (clave + CORS) |
| LM-NF-7 | Huella en disco | 106 MB (bin) + 14 GB (modelos) + 284 MB (build) + WebView2 por perfil | ✅ | medido |
| LM-NF-8 | Recuperación | Un crash del motor MUST ser visible y recuperable desde la UI (Reintentar = Iniciar) | ✅ | `process.rs:1230-1273` (`crash_summary`), `ui.html:1476` (banner) + `ui.html:3624-3638`, `ui.html:4233` (render de `last_error`) |
| LM-NF-9 | Avisos de escritorio (P20 ✅): toast WinRT vía PowerShell desacoplado (sin ventana, sin espera) en exactamente tres eventos — motor listo tras la puerta, fallo del motor, auto-stop — gobernados por `[notifications] enabled/on_ready/on_failure/on_autostop` (todos default true); un toast se MOSTRÓ en vivo en verificación. Límites honestos: el toast es un evento de escritorio (no auditable en log) y el harness falla suave (log + sigue) | ✅ | `notify.rs:1-64` (escape XML + `toast_command` testeable + `notify` desacoplado), `notify.rs:66-69` (puerta), `config.rs:155-176` (sección + defaults), `process.rs:196-218` (fallo), `process.rs:272-370` (listo), `process.rs:393-400` (auto-stop); UI en `ui.html:2038-2041` (4 toggles); tests `gate_disabled_or_flag_off_builds_nothing` + `xml_escapes_body_with_markup_and_quotes` (`notify.rs:75-92`) |

### Recomendación para el dueño (128K: límite de hardware, no un bug)

Los números a 128K son **límite de hardware, no un bug**: los pesos (13 GB) más el KV de 128K no caben en los 16 GB de la RX 6800 XT, así que el KV desborda a RAM del sistema por el bus (por eso `libros` ya lleva `cache_ram 6144` y `--load-mode none`). Evidencia: mismo prompt de 12,3k, prompt-eval 196 tok/s a 32K frente a 39,6 tok/s a 128K (dato de sesión 2026-09-28, ~5× con la profundidad). En la práctica: trabajo diario de agentes en 32K/64K; 128K sólo para documentos largos, aceptando el techo medido (~10–17 t/s efectivos según harness, ver LM-NF-2). La comparativa 64K-vs-128K con tabla fair queda **pendiente de medición**: no afirmar paridad hasta correrla [INFERENCE].

---

## 5. Deuda conocida

| ID | Deuda | Riesgo | Evidencia | Decisión |
|---|---|---|---|---|
| D-1 | ✅ CERRADA — Instalaciones frescas reportan `velocidad`; `last.profile` desconocido se sanea en carga; `start()` persiste el id real resuelto | — | `config.rs:150`, `config.rs:452-453`, `process.rs:766`; tests `default_profile_id_is_real` + `unknown_last_profile_falls_back_to_default` (`process.rs:1500-1516`) | P26 (cerrada) |
| D-2 | ✅ CERRADA — Cualquier `model` pedido se reescribe al id servido (`GET /v1/models` + alias estables), así que el ID fijo de lanzamiento ya no puede apuntar a un modelo distinto | — | `server.rs:155-172`, `server.rs:174-208`, `agents.rs:36-42` | P23 (cerrada por diseño) |
| D-3 | `/api/metrics` proxyea con `starting`+`healthy` o `error`+`healthy` | Bajo: telemetría engañosa | `server.rs:473-475` | P14 |
| D-4 | ✅ CERRADA — Lanzar sin motor vivo devuelve 409 y no escribe configs | — | `server.rs:768-773`, `agents.rs:19-34`; `node tests/e2e.mjs` | P22 (cerrada) |
| D-5 | ✅ CERRADA — Ya no se toca `~/.omp`: configs privadas en `%APPDATA%\LocalMind\agents\<agent>\` | — | `agents.rs:1-13`, `agents.rs:181-203`; `node tests/e2e.mjs --with-engine` (`home agent files byte-identical`) | P21 (cerrada por diseño) |
| D-6 | ✅ CERRADA — Puerto/contexto/clave siempre del estado vivo del motor; nada stale | — | `server.rs:774-775`, `agents.rs:8-9` | P22 (cerrada por diseño) |
| D-7 | ✅ CERRADA — Clave local exigida en `/v1/*` y `/api/*` (Bearer / `x-api-key` / cookie `lm_key`); 401 sin credencial | — | `auth.rs:1-123`, `server.rs:264-274`, `server.rs:362-366`; `node tests/e2e.mjs` (401) | P29 (cerrada por implementación) |
| D-8 | `unwrap()` en rutas calientes (3 en `process.rs`, 19 en `server.rs`: headers, parsing, assets) | Medio: aborta request/hilo en vez de 4xx/5xx | `process.rs:629`, `process.rs:775`, `process.rs:852`, `server.rs:31-47` | P31 |
| D-9 | TOML corrupto → defaults silenciosos sin aviso en UI | Medio: el usuario cree que corre con su config | `config.rs:299-310` (`read_to_string`+`toml::from_str` con `.ok()`) | P28 |
| D-10 | ✅ CERRADA — `POST /api/models/import_pick` con diálogo nativo `rfd`; el `prompt()` viejo eliminado (0 ocurrencias en `ui.html`) | — | `server.rs:654-690`, `models.rs:724-753`, `ui.html:2576-2590` | P6 (cerrada) |
| D-11 | `last.model` casi nunca se aplica (precedencia alias → primer `.gguf` → `last.model` último) | Medio: arranca un modelo distinto al fijado | `process.rs:701-713` | P8/P26 |
| D-12 | `handle_request` sin validación de esquema; errores de parse caen a defaults | Bajo | `server.rs:429-451` (legacy `import_model`), `translate.rs:40-43` (traductores sí validan) | P31 |
| D-13 | ✅ CERRADA — `alert()` eliminados (0 ocurrencias; `toast()` inline con `aria-live`), 22 `aria-label`s, thinking-toggle como `<button>`, `role="status"`, `:focus-visible`, tabs por teclado | — | `ui.html:1499-1500`, `ui.html:2414-2430`; `docs/ui/notes.md`; P17 (cerrada por implementación al nivel harness) |
| D-14 | Carrera en single-instance (`remove_file` + `create_new` no atómico) | Bajo | `main.rs:58-67` (commit base, sin cambios) | P31 |
| D-15 | Sin trazabilidad del build de llama.cpp en `bin/` (103 archivos, 106 MB, sin marcador de versión) | Medio: imposible reproducir el entorno | `bin/` | P5 |
| D-16 | Artefactos legacy `dashboard.html` y `chat.html` divergen del contrato (llaman `/api/open-omp`, `/ws/logs`, puerto 8080 fijo) | Medio: confusión y código muerto | `dashboard.html:373`, `dashboard.html:483-491`, `chat.html:181-187` | P15 |
| D-17 | Sin CI ejecutada ni remoto git (el workflow SÍ existe ahora y sus comandos corren en verde en local: ver LM-MNT-4 y D-23) | Alto para "producto profesional" | `git remote -v`, `.github/workflows/ci.yml`, árbol | P31 |
| D-18 | ✅ CERRADA en código — La espera del diálogo ahora se acota a 120 s (`recv_timeout(IMPORT_PICK_TIMEOUT_SECS)` → `{"status":"cancelled","reason":"timeout"}`); el hilo huérfano queda inofensivo. Honesto: el descarte por un humano real sigue sin ejercitarse (ver D-25) | — | `server.rs:687-698`, `models.rs:820-828` (constante + predicado + tests) | P6 (cerrada en código) |
| D-19 | ✅ CERRADA por medición viva — GGUF real con LFS (`ggml-org/models` → `tinyllamas/stories260K.gguf`) descargado por `POST /api/models/download`: aterrizó plano en `models/` con SHA256 en disco igual al `lfs.oid` del Hub; fichero de prueba eliminado después | — | `models.rs:541-560` (verificación), `models.rs:398-417` (destino plano); LM-MOD-9b | P6/P9 (cerrada) |
| D-20 | ✅ CERRADA por medición viva — Con `idle_timeout_secs=60` el motor pasó `running → stopped` al vencer el plazo, con la línea `[LocalMind] Auto-stop: motor apagado por inactividad…`, y el TOML del dueño se restauró byte-identical | — | `process.rs:296-341` (rama idle), LM-MOT-6 | P13 (cerrada por medición) |
| D-21 | ⚠️ Sin medición de VRAM en Windows AMD (el `vram` de `/api/hardware` viene del texto de `llama-server --list-devices`, no de una lectura del driver) | Bajo: el operador no ve el consumo real de VRAM | `process.rs:554-594` | P11 |
| D-22 | ✅ CERRADO — Defecto real: `start()` nunca sembraba `last_activity`, así que el auto-stop (`last > 0` en `process.rs:309-310`) no podía disparar en un arranque sin requests proxyeados previos; corregido con `touch_activity()` al arrancar | — | `process.rs:948-950` (siembra + comentario), `process.rs:309-310` (guarda); LM-MOT-6 | P13 (cerrado) |
| D-23 | ⚠️ GitHub Actions jamás ejecutado (el repo no tiene remoto); el workflow existe y todos sus comandos corren en verde en local | Medio: la CI no protege ningún push | `.github/workflows/ci.yml`, `git remote -v` vacío | P31 |
| D-24 | ⚠️ Sin firma de código ni historia SmartScreen; sin auto-update | Medio: fricción de instalación en producto | `packaging/README.md` (límites declarados), `packaging/` sin `.pfx`/manifiesto de firma | P5 |
| D-25 | ⚠️ `import_pick`: el descarte por un humano real sigue sin ejercitarse (el timeout 120 s sí está en código y testeado como predicado) | Bajo: el camino feliz/cancelación real no se vio | `server.rs:687-698`, `models.rs:820-828` | P6 |
| D-26 | ✅ CERRADA en ronda 4 — API de settings/perfiles + formularios de Ajustes verificados en vivo (round-trip TOML byte-identical) | — | `server.rs:549-710` (profiles), `server.rs:755-945` (config), `ui.html:1919-2041`; LM-CFG-7/9 | P16/P26/P27/P28 (cerradas al alcance implementado) |
| D-27 | ⚠️ Aceptación visual real en WebView2 pendiente (decisión del dueño); el rediseño está verificado sólo por harness (43 checks) + capturas | Medio: el scaffold puede no gustar en pantalla real | `docs/ui/screens/` (`live-*`, `round4-*`), `node tests/ui-render.mjs` (43 checks) | P18 |
| D-28 | ⚠️ El patch de perfil DeepSeek se templa en cada lanzamiento (`deepseek_profile_patch(http_port,…)`, reescrito sólo si cambia); verificado en el árbol actual, pero si el formato Cordis cambia en una versión futura del harness el patch puede quedar obsoleto | Bajo: un `dsh` futuro podría ignorar el provider | `launcher.rs:205-243` (plantilla + escritura atómica), `server.rs:1495-1506` (llamada con valores vivos) | P24 |
| D-29 | ⚠️ Conflicto P19↔P7 abierto: adjuntos con imágenes requieren mmproj, pero P7 fija sólo texto con mmproj apagado; por defecto manda P7 | Medio: P19 bloqueada hasta revisar P7 | §11, `agents.rs:169-176` (visión `[text]` honesto) | P7/P19 |
| D-30 | ✅ CERRADA — El árbol vuelve a compilar y `cargo test` da 74/0 en vivo 2026-09-27 (el `fn` faltante de `launcher.rs:799` ya está repuesto por el workstream dueño) | — | `cargo test` 74/0 en vivo; `launcher.rs:798-806` íntegro | P31 (cerrada) |
| D-31 | ⚠️ La puerta de aceptación varía entre cargas 128K idénticas: 32,7 / 32,47 / 14,46 t/s (más 13,35 una vez como valor stale del status, no como gate). Hipótesis: estado KV/VRAM al momento de la puerta [INFERENCE]. No arriesga falsos fallos (umbral 3 t/s) pero invalida comparar gates entre cargas | Bajo: sólo comparaciones intra-carga son válidas | `docs/agents/harness-comparison-128k.md:100-125` (varianza + § Gate vs control) + fair `tests/harness-bench/results/2026-09-25T17-54-26-616Z.json` (`decode_tps` 14,46) | P10 |
| D-32 | ⚠️ La pata de reacción de OpenCode expiró a los 600 s (ok=false, 1 req 592+28) mientras su throughput pasó (3 req, 208,7 s); pendiente de triaje del lado harness (no del gateway: el mismo motor sirvió las demás patas) | Medio: un harness con un modo colgado | `docs/agents/harness-comparison-128k.md:75-90` (tabla fair + hallazgo abierto); `tests/harness-bench/results/2026-09-25T17-54-26-616Z.json` (pata `ok:false`) | P24 |
| D-33 | ⚠️ Curva de profundidad sin medir: decode t/s vs longitud real de contexto (512/8k/32k) con `finish_reason` registrado; el re-run en quietud está pendiente — se omitió porque el dueño estaba usando la máquina, no por imposibilidad | Medio: el techo 25 t/s no se sabe a qué profundidad aplica | `docs/agents/harness-comparison-128k.md:8-11` (pendiente declarado) + protocolo en `:260-280`; causa de la omisión en `docs/agents/engine-pressure.md:4` (máquina en uso) | P10 |
| D-34 | ⚠️ Rerun de deriva térmica/sostenida pendiente (omitido porque el dueño estaba usando la máquina); el tiempo de motor encendido en la ventana de medición (~50 min entre campaña e integración) superó los 25 min planeados | Bajo: posible droop térmico no separado | `docs/agents/harness-comparison-128k.md:8-11` + `:145-151` (35 min campaña) + `:60-70` (wall fair 2661 s); causa de la omisión en `docs/agents/engine-pressure.md:4` | P10 |
---

## 6. Objetivo por fases
Decisiones P# cerradas el 2026-09-25 (detalle en §11 y `docs/srs/respuestas-20260925.md`); las fases quedan habilitadas en orden F1→F2→F3.

| Fase | Objetivo | Contenido | Requisitos |
|---|---|---|---|
| **F0 — Base v2.0.0 (hecho)** | Estación local funcional sin terminal | Rust+WebView2, motor, perfiles, launchers, anti-apagón, auto-stop, telemetría, chat | §3.1–3.10 (✅) |
| **F1 — Contrato e higiene** | El sistema hace lo que dice, sin sorpresas | Cerrar D-1..D-17 según decisiones; contrato no destructivo de configs ajenas; avisos de config/errores; eliminar legacy; tests + CI; logs a archivo | P15, P21, P22, P23, P25, P26, P28, P29, P30, P31 |
| **F2 — Producto** | Instalable y "profesional" | Instalador + versión visible + updates; gestor de modelos; edición de perfiles y parámetros de generación en UI; accesibilidad y estética; multimodal/voz si se decide | P5, P6, P7, P9, P16, P17, P18, P19, P20, P26, P27 |
| **F3 — Plataforma** | Extensible y multi-entorno | Multi-CLI, multi-modelo, multi-GPU/backends, headless como servicio, reedición para terceros | P1, P2, P3, P4, P8, P24 |

---

## 7. Decisiones P1–P32 (cerradas 2026-09-25)

Se respondieron en `docs/srs/index.html`; la tabla conserva la recomendación técnica original y el detalle con notas está en `docs/srs/respuestas-20260925.md` (§11 resume desvíos y el conflicto P7↔P19).

| ID | Pregunta | Recomendación |
|---|---|---|
| P1 | ¿Para quién es LocalMind (personal / equipo / producto para terceros)? | Uso personal primero, arquitectura lista para terceros |
| P2 | ¿Un modelo activo por vez o varios? | Uno (el hardware no da para dos 27B) |
| P3 | ¿Windows-only o multiplataforma? | Windows-only (es lo que el hardware/runtime permite hoy) |
| P4 | ¿Sólo Vulkan o añadir ROCm/CUDA? | Medir ROCm con evidencia antes de comprometer |
| P5 | ¿Portable o instalador con updates? | Instalador + versión visible |
| P6 | ¿Gestor de modelos con descarga? | Sí, con verificación de integridad |
| P7 | ¿Imagen/voz en el chat? | Sin visión por ahora; TTS opcional después |
| P8 | ¿Un modelo o dos (rápido + grande)? | Uno |
| P9 | ¿Política de cuantización? | Fijar IQ4_XS y documentar el trade-off |
| P10 | ¿Meta dura de t/s por perfil? | Definir 32K y 262K por separado, con medición real |
| P11 | ¿MTP/cache-reuse vs estabilidad? | Estabilidad primero, con perilla visible |
| P12 | ¿Política anti-apagón? | Mantener perfil actual y hacerlo visible |
| P13 | ¿Auto-stop configurable y con aviso? | Sí, configurable en UI |
| P14 | ¿Auto-arranque del motor al abrir? | No por defecto, recordar última sesión |
| P15 | ¿Eliminar `dashboard.html` y `chat.html`? | Eliminar (divergen del contrato) |
| P16 | ¿Parámetros de generación editables? | Sí, con preset por perfil → IMPLEMENTADO en ronda 4 (`generation` en TOML + validadores + tarjeta en Ajustes; LM-CFG-10, LM-UI-10 ✅) |
| P17 | ¿Accesibilidad e idioma? | Español + a11y básica → SUPERADO en ronda 3/4: bilingüe es/en + `toast()` sin `alert()` + 17/17 contraste (LM-UI-11 ✅, LM-UI-14 🟡 scaffold) |
| P18 | ¿Estética/tema? | Rediseño "premium" sobre el sistema visual actual → SUPERADO por decisión del dueño (P18=C: rediseño completo, LM-UI-13 🟡 scaffold, D-27) |
| P19 | ¿Adjuntos en el chat embebido? | Texto e imágenes del portapapeles → ABIERTO (conflicto P7↔P19 en §11; chat sigue texto) |
| P20 | ¿Notificaciones del sistema al terminar de cargar / auto-stop? | Sí, sólo eventos relevantes → IMPLEMENTADO en ronda 4 (3 eventos + toggles; LM-NF-9 ✅) |
| P21 | ¿Contrato de escritura en configs de terceros? | Merge no destructivo + backup → SUPERADO: dirs privados, cero escrituras ajenas (LM-INT-1 ✅) |
| P22 | ¿Puerto fijo para CLIs o descubrimiento dinámico? | Puerto estable reservado para CLIs → SUPERADO: gateway ligado + estado vivo (LM-CLI-7, LM-TEL-6 ✅) |
| P23 | ¿Respetar el modelo elegido en los launchers? | Sí → IMPLEMENTADO por aliasing + `rel` (LM-CLI-5, LM-MOD-12 ✅) |
| P24 | ¿Más CLIs soportados? | OMP/Pi/Orca primero; resto por pedido → REVISADO en ronda 4 por decisión del dueño: Orca ELIMINADO del producto y del código (cero ocurrencias); soportados pi, omp, opencode, web, deepseek (LM-CLI-7/8/9 ✅) |
| P25 | ¿Qué debe abrir "Interfaz Web Externa"? | Definir: UI de llama.cpp (hoy) o UI de LocalMind |
| P26 | ¿Edición de perfiles en la UI? | Sí, editor con validación → IMPLEMENTADO en ronda 4 (tab Ajustes + `save`/`delete` validados; LM-CFG-7 ✅, D-26 cerrada) |
| P27 | ¿Config en APPDATA o portable? | APPDATA con opción portable → PARCIAL: `GET/POST /api/config` implementado (LM-CFG-9 ✅); la opción portable sigue ⬜ |
| P28 | ¿Qué hacer con un TOML inválido? | Avisar y ofrecer reparación → PARCIAL: validadores + 400 con campo nombrado en la API nueva (D-9 sigue abierta para el TOML corrupto en arranque) |
| P29 | ¿Auth/CORS en la API local? | Token simple + CORS restringido a loopback → IMPLEMENTADO (clave + reflejo sólo loopback; LM-API-6/11, LM-NF-6 ✅) |
| P30 | ¿Persistir logs a archivo? | Sí, con rotación y exportación → IMPLEMENTADO en ronda 4 (`logs/localmind.log` 1 MiB × 3 + export; LM-TEL-3 ✅) |
| P31 | ¿Tests y CI? | Suite mínima + CI, y publicar remoto |
| P32 | ¿El SRS es la fuente de verdad versionada? | Sí, se actualiza por fase |

---

## 8. Trazabilidad y mantenimiento

- **Fuente de evidencia:** commit `1120228` + árbol de trabajo post-integración, ronda 7, sin commitear (sanitización `reasoning_effort` en `server.rs:206-245`; `config.rs:401-407` nota 128K; `tests/harness-bench.mjs` + `tests/pressure-sampler.mjs`; 9 resultados en `tests/harness-bench/results/`; `docs/agents/harness-comparison-128k.md` + `docs/agents/engine-pressure.md`). `cargo test` 74/0 en vivo 2026-09-27. Los números de línea citados corresponden a ese árbol; al commitear, re-verificar antes de citar.
- **Regla de actualización:** cada fase cerrada MUST actualizar estado (✅/🟡/⬜) y `D-#` correspondiente en este mismo archivo.
- **Regla de honestidad:** un requisito no puede marcarse ✅ sin cita verificable en el código; lo no medido se marca `[INFERENCE]` o "no medido".
- **Ronda 7 cerrada en este documento:** integración 128K de los 4 harnesses por ruta de la app (LM-CLI-10), normalización `reasoning_effort` (LM-PXY-2), números fair 128K (LM-NF-2/2b), presión RAM/commit (LM-NF-3), toolchain `harness-bench`+`pressure-sampler` (§9). Sigue abierto lo marcado ⚠️ (D-23/D-24/D-25/D-27/D-28/D-29, D-31–D-34 nuevas, P19↔P7). D-30 está cerrada (el árbol compila; `cargo test` 74/0 en vivo).

### Anexo A — Verificación de la deuda de configs de terceros (2026-09-25; superado en v1.1)
Tras el commit `1120228` ("Eliminar Bonsai…"), el disco todavía conserva referencias muertas:
- `%USERPROFILE%/.pi/agent/models.json`: provider `localmind` con `baseUrl http://127.0.0.1:8081/v1` y modelo `bonsai-2-27b`.
- `%USERPROFILE%/.omp/agent/models.yml`: provider `bonsai` completo.
- `%USERPROFILE%/.omp/agent/config.yml`: referencias a `bonsai`/`bonsai-2-27b`.
Desde la ronda de integración esos archivos ya no se leen ni escriben (LM-INT-1, D-5/D-6 cerradas): los lanzadores usan `%APPDATA%\LocalMind\agents\<agent>\` con puerto/contexto/clave vivos, y `node tests/e2e.mjs --with-engine` verifica que los ficheros del home quedan byte-identical. Decisión: P21/P22 (cerradas por diseño).

---

## 9. Verificación automatizada

Suite en `tests/` (Node ESM sin dependencias, Node ≥ 18; `cargo test` para Rust). Ningún harness toca `src-rust/`, `ui.html` ni `docs/`.

| Harness | Comando | Qué prueba | Qué NO puede |
|---|---|---|---|
| `tests/smoke.mjs` | `node tests/smoke.mjs [baseUrl] [--write-fixtures] [--force]` | Contrato de lectura de la API: 14 checks GET (+1 OPTIONS), todos con la clave auto-leída (`LM_KEY` o `gateway.key`). Verificado: 14/14 contra la app viva | No arranca ni detiene el motor; no POSTea; los branches de `/api/metrics` y `/api/events` dependen del estado observado, no lo dirigen |
| `tests/bench.mjs` | `node tests/bench.mjs --profiles 32k,64k,128k,262k […]` · `--self-test` (23 asserts puros, sin app) · `--dry-run` (plan sin requests) · `--compare <a.json> <b.json>` (deltas + veredictos por métrica) | Rendimiento por perfil: arranque pineado, espera de `running`+`acceptance_ok`, probe streaming, `/api/metrics` justo después; tasas sólo de tokens+tiempos observados (`null` si faltan, nunca 0). Resultados en `tests/bench-results/bench-<fecha>.json` (12 corridas, 2026-09-24/25). `--self-test`: 23/23 | Cada perfil cuesta una carga fría + generación (minutos); el TTFT medido por el proxy incluye la generación completa del motor (el proxy reenvía tras leer el body); 64K/128K tienen una sola corrida cada uno. Comparador en `tests/bench.mjs:174-238` |
| `tests/harness-bench.mjs` | `node tests/harness-bench.mjs --harnesses pi,omp,opencode,deepseek --profile libros --context 131072 [--fair --thinking low --warmup] [--dry-run] [--no-control]` | Campaña 128K por harness sobre UNA carga compartida: reacción (spawn→primer byte) + throughput (tokens de `usage.jsonl`, t/s por pata) + control directo; `--fair` = orden inverso + thinking explícito + warmup + patas por request; `--dry-run` imprime comandos sin tocar nada. Evidencia: 9 JSON en `tests/harness-bench/results/` (8 campaña + fair `2026-09-25T17-54-26-616Z.json`); análisis en `docs/agents/harness-comparison-128k.md` | Requiere motor vivo + harnesses instalados; cada campaña ≈35 min de motor; la pata de reacción de opencode expiró a 600 s una vez (D-32) |
| `tests/pressure-sampler.mjs` | `node tests/pressure-sampler.mjs --duration 600 [--out <f>] [--base-url …]` | Muestreo cada 5 s (RAM libre/commit, page-file, CPU total + llama RSS/CPU, GPU engines, `gen_tps` si la app responde) a JSONL; no arranca nada, corre en paralelo. Evidencia: `tests/harness-bench/results/pressure-*.jsonl` (p. ej. 900 s/142 muestras `pressure-2026-09-25T17-09-30-820Z.jsonl`) | GPU por contadores (si AMD no responde, no-medible); `gen_tps` sólo si la app responde |
| `tests/ui-render.mjs` | `node tests/ui-render.mjs` | 43 checks de render sobre `ui.html` (~3.900 líneas) con DOM sintético (franja `Cargando… N% (ETA …)` / `Verificando GPU…`, `Velocidad verificada`, sin fugas `undefined`/`NaN`, progreso de descarga, badges verificado/descargado, tarjeta de uso, I18N es/en, toast sin `alert()`, targets/handlers del shell, tab Ajustes, selector único, ausencia de orca/sol, marca LM) | No abre navegador ni WebView2: prueba el JS embarcado, no el render real (la aceptación visual es del dueño, D-27) |
| `docs/ui/contrast.mjs` | `node docs/ui/contrast.mjs` | 17/17 pares de contraste del rediseño (texto ≥7:1, secundario ≥4.5:1, hairlines ≥1.5:1) | Sólo pares declarados en el script, no captura visual |
| `cargo test` | `cargo test` (en `src-rust/`) | 85 passed / 0 failed (verificado en vivo 2026-09-27): puerta de aceptación, `crash_summary`, `load-times`, precedencia de contexto, resolución basename/`rel` (5 tests), migración `profiles_version` (11 tests `mig_*`: intactos/v1/v2/canónico, customs y desconocidos intactos, proofs en scratch APPDATA), `agents` (dirs privados, 409, gateway), `models` (sha256, tree, verified, enrich, listado anidado, timeout `import_pick`, destino plano), `meta` (versión STDERR, loopback/ajenos, sello UTC), `notify` (puerta + escape XML), `filelog` (formato + rotación ×3), `launcher` (agentes ×5, spawn verify/fallback, gateway, patch deepseek, env XDG/DSH), `usage` (incl. override `LOCALMIND_USAGE_PATH`), `translate` (Anthropic/Responses + tools), auth, tee incremental + `reasoning_effort` (`reasoning_effort_max_a_xhigh_y_desconocido_se_elimina`) | Unitario: no levanta ni el servidor ni el motor |

Mediciones vivas registradas por estos harnesses (2026-09-24/25, RX 6800 XT, Qwen3.8-27B IQ4_XS): puerta de aceptación observada (`verifying`, `acceptance_ok:true`, `decode_tps` 34,2 t/s @32K); streaming incremental (primer byte 2519 ms vs último 12609 ms); omp 18.3.0 y pi 0.87.0 con "ok" local; tabla 32K 30,17 · 64K 30,07 · 128K 29,45 · 262K 14,16 t/s con cargas de 24,5/22,3/26,2/34,4 s.

---

## 10. Changelog

### v1.1 (2026-09-26) — Ronda de integración: gateway, puerta de aceptación, modelos y telemetría
Sobre el commit `1120228` (aún sin commitear): la API local ahora exige clave (`gateway.key`, LM-API-6/LM-API-7, D-7 cerrada); el proxy suma `GET /v1/models`, `POST /v1/messages` y `POST /v1/responses` con reescritura de alias y registro de uso JSONL (LM-PXY-4/5/6); el arranque del motor pasa por una puerta de aceptación con progreso y ETA persistente (LM-MOT-12/13, LM-NF-5) y la precedencia de contexto explícito queda fijada con regresión (LM-MOT-1); los lanzadores escriben configs privadas con estado vivo y devuelven 409 sin motor sin tocar el home (LM-CLI-5/6, LM-INT-1, D-2/D-4/D-5/D-6 cerradas); la importación usa diálogo nativo (LM-MOD-4, D-10 cerrada) y la descarga HF verifica tamaño+sha256 con resume y registros (LM-MOD-9/10/11); la UI muestra progreso, velocidad verificada, descargas y uso (LM-UI-4b/5b); la suite `tests/` + `cargo test` (38/0 entonces) deja todo lo anterior verificado (§9). Medición honesta: 32K supera los 30 t/s (30,17) pero 262K se queda en 14,16 (LM-NF-2 🟡); deuda nueva y visible en D-18–D-21.

### v1.1.1 (2026-09-27) — Modelos anidados, timeout de diálogo y verificación LFS viva
Sobre v1.1 (árbol aún sin commitear): `GET /api/models` lista raíz + un nivel con `rel` (LM-MOD-11) y el arranque acepta basename o `rel` rechazando escapes (LM-MOD-12, 5 tests nuevos: `cargo test` pasa de 38 a 45/0); `import_pick` acota el diálogo a 120 s con `cancelled/timeout` (D-18 cerrada en código, LM-MOD-4); la rama sha256-con-LFS quedó probada con un GGUF real (`ggml-org/models` → `tinyllamas/stories260K.gguf`, SHA256 en disco = `lfs.oid`, fichero de prueba eliminado) y D-19 se cierra (LM-MOD-9b); LM-MOD-6 pasa a ✅ para la superficie implementada (borrado/cuantizaciones siguen ⬜ P6/P9). Honesto: el descarte del diálogo por un humano sigue sin ejercitarse.

### v1.2 (2026-09-25) — Cierre P1–P32
Cierre del cuestionario (32/32 en `docs/srs/respuestas-20260925.md` + `.json`): §7 pasa a cerradas y se agrega §11 con desvíos asumidos y el conflicto P7↔P19. Sin cambios de código en esta versión.

### v1.2 (2026-09-27) — Ronda 3: versión, CORS estricto, auto-stop vivo, ROCm descartado, portable+CI, rediseño bilingüe
Sobre v1.1.1 (árbol aún sin commitear): `GET /api/version` (app + build/commit del motor, STDERR incluido) y `GET /api/logs/export` (LM-API-9/10, módulo `meta.rs`); CORS sin wildcard — reflejo sólo loopback, P29 cerrada del todo (LM-API-11, LM-NF-6); defecto real de auto-stop corregido (`start()` siembra `last_activity`, D-22) y D-20 cerrada con medición viva (`idle_timeout_secs=60` → `running → stopped`, TOML restaurado); P4 respondida "sólo Vulkan" (`bin-hip/` expone cero GPUs, ≈1,2 tok/s CPU vs 30,17 Vulkan; LM-MOT-10); portable+instalador (`packaging/`, ZIP 111 ficheros/~39 MB) y CI en Windows (LM-DST-2 🟡, LM-MNT-4 🟡, D-23 honesto: Actions jamás ejecutado); UI reconstruida bilingüe con contraste 17/17 y 14 capturas (LM-UI-11 …

### v1.3 (2026-09-27) — Ronda 4: notificaciones, logs a archivo, settings, launchers por gateway, OpenCode+DeepSeek
Sobre v1.2 (árbol aún sin commitear): toasts WinRT en 3 eventos con toggles (P20 ✅, LM-NF-9, `notify.rs`); logs a archivo 1 MiB × 3 vía `mgr.log()` (P30 ✅, LM-TEL-3, `filelog.rs`); D-1 cerrada (`velocidad` + saneado + 2 tests); `GET/POST /api/config` + `POST /api/profiles/save|delete` con validadores y persistencia atómica (LM-CFG-7/9/10, D-26 cerrada; round-trip TOML byte-identical en vivo); `POST /api/launch` + `GET /api/agents` (`pi, omp, opencode, web, deepseek`), compat forwarders, Orca eliminado del producto y del código por decisión del dueño (P24 revisada; LM-CLI-7/8/9); todo el tráfico de agentes por el gateway ligado con clave — el puerto crudo no contabiliza y el gateway sí (LM-TEL-6, contraste `usage.jsonl` en `docs/agents/`); spawn endurecido verify+fallback con pid/500 y hallazgo del alias `wt.exe` de 0 bytes (LM-CLI-3); Ajustes + selector único + marca LM + i18n (LM-UI-10/15, `ui-render` 43 checks, capturas `live-*`/`round4-*`); OpenCode v1.18.10 (≈10,5 s, segunda pata ≈32,9 t/s) y DeepSeek v0.1.5-rc.3 (run A 55 s / 9.709+53 ≈1,0 t/s) respondiendo en local; higiene `LOCALMIND_USAGE_PATH` (LM-TEL-5); deuda nueva D-28–D-30.

### v1.4 (2026-09-27) — Ronda 7: harnesses a 128K, fix `reasoning_effort`, techo real 25 t/s
Sobre v1.3 (árbol aún sin commitear): los 4 harnesses prueban a 128K por ruta de la app (`POST /api/launch` 200 + spawn con pid + config privada al gateway con puerto vivo y ctx 131072; LM-CLI-10; interactivos pi/omp por config+spawn, one-shot opencode/deepseek además por `usage.jsonl`); `reasoning_effort` normalizado en los 3 proxys (`minimal`/`low`→`low`, `high`/`max`/`xhigh`→`xhigh`, `medium`→`medium`, desconocido eliminado — la plantilla Qwen daba 500 con `max`; LM-PXY-2; binario `db1a1b8f…`: `max`/`minimal`/`medium` → 200, prompts 56/44/14); fair quieto a 128K (CPU 5,7 %): omp eff 17,3 · pi 11,8 · opencode 2,4 · deepseek 5,6, control decode 25,0 t/s — **ningún harness llega a 30 t/s efectivo; techo del motor ≈25 t/s** (LM-NF-2/2b; fair `2026-09-25T17-54-26-616Z.json`). Cierre 128K: `libros` lleva `--load-mode none` (A/B mismo prompt 10546 tok: prefill 21,65→38,18 t/s +76 %, decode igual; LM-MOT-2/LM-CFG-3/LM-NF-3) con migración segura `profiles_version = 3` (solo intactos, atómica, `bak-pre-v3` conservado; historial v1→v2→v3 declarado; 11 tests `mig_*`, `cargo test` 85/0). Siguen sin medirse (dueño en máquina): deriva térmica y `finish_reason` del corte en 127.

### v1.5 (2026-09-28) — Diagnóstico 128K: prompt-eval, recomendación y comparativa pendiente

Sólo docs, sin código ni motor (máquina sensible tras apagado): LM-NF-2b suma el prompt-eval con el mismo prompt de 12,3k (196 tok/s a 32K frente a 39,6 tok/s a 128K, dato de sesión 2026-09-28) y marca la comparativa 64K-vs-128K con tabla fair como **pendiente de medición**; §4 nuevo «Recomendación para el dueño» fija que los números 128K son límite de hardware (KV fuera de los 16 GB) y no un bug, con guía práctica 32K/64K diario y 128K sólo para documentos largos.

## 11. Decisiones cerradas (2026-09-25)

Respuestas completas: `docs/srs/respuestas-20260925.md` (+ `.json` máquina).
Compacto: `P1:C P2:A P3:A P4:A P5:A P6:A P7:A P8:A P9:B P10:A P11:A P12:A P13:A P14:A P15:A P16:A P17:C P18:C P19:A P20:A P21:A P22:A P23:A P24:A P25:A P26:A P27:A P28:A P29:A P30:A P31:A P32:A`.

Desvíos de la recomendación técnica (asumidos por el dueño, con coste):
- P1=C (producto para terceros, no sólo esta PC): multiplica F2/F3 (instalador, presets por hardware, soporte).
- P9=B (Q4/Q5/Q6 + bench automático, no fijar IQ4_XS): más medición por perfil en F2.
- P17=C (bilingüe es/en, no sólo español+a11y): duplica textos y pruebas en F2.
- P18=C (rediseño completo, no sobre estructura actual): mayor coste de F2.

Conflicto abierto para F2: P19=A (texto + imágenes del portapapeles) requiere mmproj, pero P7=A fija sólo texto con mmproj apagado. Por defecto manda P7; revisar P7 antes de implementar P19.

Notas de coherencia con el árbol: D-2/D-4/D-5/D-6/D-7/D-10 ya figuran cerradas por diseño/implementación en §5 y el cierre P21/P22/P23/P29 las confirma; P12 registra la nota textual del dueño (anti-apagón como requisito duro); P10 remite a §9 (262K ≈14 t/s vs objetivo de 30 t/s — no cumplido).

