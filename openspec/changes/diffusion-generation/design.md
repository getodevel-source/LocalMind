# Diseño: `diffusion-generation`

Fase: **design**. Fecha: 2026-09-30. Cambio: `diffusion-generation`.
Artefacto: `design.md`. Store: híbrido (OpenSpec + Engram, clave `sdd/diffusion-generation/design`).
Entradas: `exploration.md` (fase `explore`, id Engram 1204), `proposal.md` (fase `propose`).

> Los identificadores, rutas de API, flags y nombres de fichero van en inglés porque son del
> runtime. La prosa va en español, como el resto de `docs/` y `tests/`.
> La copia de UI va en inglés, por `openspec/config.yaml` §context (`Style: identifiers and UI
> copy in English`).

---

## 0. Corrección de dos premisas antes de decidir nada

Encontré dos cosas que cambian el encargo. Las digo primero porque el diseño de abajo depende
de ellas, y ninguna es opinión: son medidas sobre este repositorio.

### 0.1 `bin/` no está en git. El plan de reversión de la propuesta no es ejecutable tal cual

`proposal.md:394-398` dice:

> `bin/` **solo se añade**. […] `git status` sobre `bin/` debe mostrar únicamente lo añadido.

**Eso no se puede hacer.** `.gitignore:13` contiene `bin/`, `.gitignore:10` contiene `models/`, y
`git ls-files bin` devuelve **0 ficheros**. `bin/` es un artefacto no versionado, igual que el
build anterior (`bin.prev-10683/`). En consecuencia:

- `git status --porcelain` **no muestra nada** de `bin/`, ni antes ni después del cambio. No es
  evidencia de nada. Green por ausencia.
- Tampoco sirve `models/`: está igualmente ignorado.

Esto no es un detalle de redacción del plan de reversión: **la invariante "los binarios de
llama.cpp no se tocan" es la que sostiene todo el punto de no retorno**, y tal como está escrita
no es verificable por el método propuesto. §7 la reemplaza por un manifiesto de hashes
verificable, y fija además una garantía estructural que hace la colisión de §2 imposible por
construcción.

### 0.2 El riesgo de contaminación por `mmproj` no es el que decía el encargo — y es peor

El encargo dice que la torre de visión de Qwen-Image-2.1 *"contaminaría la lista de modelos del
LLM"*. **No ocurre**, y conviene decirlo porque la exclusión por `mmproj` está **protegiendo** el
catálogo LLM:

- `list_gguf_files` (`models.rs:750`, `models.rs:777`) y `list_models` (`process.rs:1189`) excluyen
  por nombre cualquier fichero que contenga `mmproj`.
- `validate_req_model` (`process.rs:1781`) valida contra esa lista, y `resolve_model_path`
  (`process.rs:1876`) solo resuelve rutas que salieron de ahí.

O sea: `mmproj-Qwen3VL-8B-Instruct-F16.gguf` en `models/` **no** aparece en `GET /api/models` y
**no** se puede arrancar como modelo. Correcto, y no hay que tocarlo.

**El peligro real es otro y es más severo.** `default_mmproj_files()` (`config.rs:469`) devuelve
`["mmproj-BF16.gguf", "mmproj-F16.gguf"]` y `find_mmproj` (`process.rs:1222`) hace:

```rust
cfg.mmproj.files.iter()
   .map(|f| self.models_dir.join(f))
   .find(|p| p.exists())
```

El **segundo candidato por defecto es `mmproj-F16.gguf`**, que es exactamente el nombre genérico
que publican los releases GGUF de Qwen-VL (`Qwen/Qwen3-VL-8B-Instruct-GGUF`). Y medido ahora mismo
en esta máquina:

```
models/mmproj-BF16.gguf    931 146 432 B   <- existe; proyector real del LLM
```

Si el dueño descarga la torre de visión de Qwen-Image-2.1 a `models/` con el nombre upstream
`mmproj-F16.gguf` — que es lo natural, porque así viene — y tiene `[mmproj] auto = true`,
`llama-server` arranca contra un modelo de texto de 27B con **una torre de visión de 8B como
proyector multimodal**. Eso no es basura visual: es una configuración semánticamente falsa que el
motor acepta en silencio.

**Conclusión: la separación no puede ser un filtro de nombre (ese filtro ya existe y es correcto
para el LLM); tiene que ser una separación de espacio de nombres por directorio.** §5.

Nota lateral: `mmproj.auto` viene en `false` por defecto (`config.rs:174`), así que hoy el dueño
está protegido por el default. El riesgo es de una edición de `localmind.toml`, no del arranque
por defecto.

---

## 1. Enfoque técnico

Se añade **un segundo motor de larga duración** al patrón que ya es la identidad arquitectónica de
LocalMind ("proceso vendorizado + puerto propio + API HTTP + proxy"), y **tres módulos Rust nuevos**
que traducción las piezas que `ProcessManager` resuelve para el LLM pero que son genuinamente
distintas para difusión:

| Módulo | Responsabilidad | Por qué no cabe en `process.rs` |
|---|---|---|
| `presets.rs` | Catálogo de presets declarados + validación de bundle + filtrado de flags por backend | Es lógica **pura** con tipo propio (`ComponentSpec`, sin enum de roles). `process.rs` ya tiene 134 KB. |
| `diffusion.rs` | Ciclo de vida del motor de difusión, **coordinador de VRAM** (elunia), proxy interno a `sd-server` | El coordinador es estado compartido entre dos motores; `process.rs` posee un solo hijo. |
| `ledger.rs` | Libro de semillas + almacén de imágenes | Persistencia nueva con forma propia (JSON + JSONL + blobs), como `usage.rs`. |

El objeto de diseño central es **`EngineCoordinator`**: LocalMind pasa de *un* motor a *dos*
motores compitiendo por la misma VRAM, y el diseño no puede ser "dos `spawn()` independientes
que compiten". La regla que sale de §3 es una sola y simétrica:

> **La GPU es una exclusividad conmutada. Se le concede a quien la pide, y solo se le quita a
> quien está inactivo.**

---

## 2. Colocación de `ggml-vulkan.dll` — decisión BLOQUEANTE

### 2.1 Los hechos medidos

```
bin/ggml-vulkan.dll             57 547 264 B   <- llama.cpp PrismML build (vivo)
bin.prev-10683/ggml-vulkan.dll  54 365 184 B   <- build anterior del mismo repo
zip de sd.cpp Vulkan   ggml-vulkan.dll  ~40.7 MB  (owner: 40.69 MB; exploration.md:24 dice 42.7 MB)
```

Los dos primeros son **dos builds distintas del mismo nombre de fichero**, y ya conviven en el
repositorio como directorios hermanos. Da igual cuál es la cifra exacta del zip de sd.cpp
(`exploration` dice 42.7 MB, la verificación del owner 40.69 MB): **no es 57 547 264**, luego la
colisión es real en cualquier caso. Apunto la discrepancia porque un artefacto que se contradice
en una cifra no es evidencia; la decisión de abajo no depende de ella.

Estado actual de `bin/` (medido): **no existe `sd-server.exe`**, ni `stable-diffusion.dll`, ni
`webm.dll`/`libwebp.dll`/`libwebpmux.dll`.

### 2.2 Por qué el aplanado es imposible, no solo poco elegante

Windows resuelve las DLL de un proceso hijo en este orden (SafeDllSearchMode, por defecto desde
XP SP2), y el **primer paso gana sobre todos los demás**:

1. **El directorio desde el que se cargó la aplicación** (el directorio del `.exe`)
2. Directorio de sistema (`System32`)
3. Directorio de sistema de 16 bits
4. Directorio de Windows
5. Directorio actual (cwd)
6. Directorios de `PATH`

Hoy `llama-server.exe` vive en `bin/` y se lanza con `current_dir(base_dir)` = raíz del proyecto
(`process.rs:651`, `process.rs:1406`). Su directorio de aplicación es `bin/`, así que carga
`bin\ggml-vulkan.dll` — la de llama.cpp. **Ese es el comportamiento vivo y no se toca.**

Si el zip de sd.cpp se aplana en `bin/`:

- `sd-server.exe` en `bin\` → paso 1 → carga `bin\ggml-vulkan.dll` = **la build de llama.cpp**, de
  57.5 MB, contra un ABI `ggml` y un `stable-diffusion.dll` de 35.3 MB que esperan el suyo.
  El resultado no es un error limpio: es un backend Vulkan ajeno, que como mínimo no hace lo que
  el preset/configuración promete, y en el peor caso deja la GPU en un estado que exige reiniciar
  el driver — exactamente el modo de fallo que R13 ya declara.
- Si en cambio se **sobrescribe** `bin/ggml-vulkan.dll` con la de sd.cpp, se rompe `llama-server`,
  que es la función que hoy funciona.

**No hay orden de aplanado que funcione.** No es una preferencia de estilo.

### 2.3 Decisión: `bin/sd-cpp/` (subdirectorio dedicado por motor)

```
bin/                        <- INTACTO. La build de llama.cpp no se mueve, no se renombra, no se toca.
  llama-server.exe  …
  ggml-vulkan.dll   (57 547 264 B, sin cambios)
  sd-cpp/                   <- LO ÚNICO QUE SE AÑADE
    sd-server.exe
    stable-diffusion.dll
    ggml-vulkan.dll         (~40.7 MB — la de sd.cpp)
    webm.dll  libwebp.dll  libwebpmux.dll
```

**Justificación contra el orden de búsqueda de Windows:** el `ggml-vulkan.dll` de sd.cpp queda en el
**directorio de la aplicación** de `sd-server.exe`, o sea el paso 1 de la resolución, que gana
siempre — con y sin `SafeDllSearchMode`. No depende del cwd, no depende de `PATH`, no depende de
variables de entorno, y no puede ser sombreado por nada de `bin/` porque está en otro directorio.
Es la forma más robusta de las tres y no cuesta nada.

**Justificación contra las convenciones de `base_dir()` / `bin_dir()`:** el proyecto **ya** usa
directorios hermanos por build. `.gitignore:16-17` declara `bin-hip/` ("HIP/ROCm third-party
binaries (llama.cpp build variant)") y existe `bin.prev-10683/`. `ProcessManager::new_with_log_file`
(`process.rs:188-189`) deriva `bin_dir = base_dir.join("bin")` y `models_dir =
base_dir.join("models")`. `bin/sd-cpp/` es la misma convención un nivel más abajo, y **no obliga a
cambiar ninguna derivación existente**: `meta::engine_bin_path` (`meta.rs:79`) sigue resolviendo
`base_dir/bin/llama-server.exe` y `detect_hardware` (`process.rs:1114`) sigue usando
`bin_dir/llama-server.exe`.

### 2.4 Alternativas descartadas

| Alternativa | Por qué no |
|---|---|
| Aplanar en `bin/` | §2.2: el paso 1 del loader resuelve a la DLL equivocada en uno de los dos motores. |
| `bin/llama/` + `bin/sd/` (un directorio por motor) | **Toca los binarios de llama.cpp**: los mueve. Rompe la invariante de reversión de `proposal.md:396` y la regla dura de `config.yaml` §rules.apply ("Do not modify or regenerate `bin/` binaries"). Coste alto, beneficio cero. |
| Renombrar una de las dos DLL | El backend Vulkan de llama.cpp/sd.cpp se carga por **nombre** en runtime. Renombrar depende de `GGML_BACKEND_PATH` o de que el nombre se resuelva por heurística: frágil, depende de un comportamiento no verificado, y rompe el paquete vendorizado. Rechazado por especulación. |
| `PATH` por proceso o variable de entorno | Depende de resolución diferida y de orden de `PATH`; es el punto más frágil de la cadena. |

**Medición de contraste dentro del propio repositorio** (no es una opinión, es el precedente): el
proyecto **ya** mantiene dos builds distintas del mismo `ggml-vulkan.dll` en directorios hermanos:

```
bin/ggml-vulkan.dll             57 547 264 B   build viva (prism-b10743-adfffbe)
bin.prev-10683/ggml-vulkan.dll  54 365 184 B   build anterior
```

Es decir: la convención "un directorio por build, una DLL Vulkan por directorio" **ya es la del
proyecto**, y `bin/sd-cpp/` la continúa en lugar de inventarla.

### 2.5 `cwd` y argv concretos

`cwd` del `spawn()`: **`bin/sd-cpp/`**, no la raíz del proyecto. El paso 1 del loader ya resuelve
la DLL correcta sin esto, pero el cwd se fija igualmente porque (a) cualquier resolución
relativa que el motor haga internamente cae en el directorio de su propio motor, y (b) deja el
argv reproducible y explicable en el log. **Todas** las rutas de componentes van **absolutas**, de
modo que nada depende del cwd.

```
programa : C:\PROYECTOS\LocalMind\bin\sd-cpp\sd-server.exe
cwd      : C:\PROYECTOS\LocalMind\bin\sd-cpp
argv     :
  --diffusion-model       C:\PROYECTOS\LocalMind\models\diffusion\presets\qwen-image-2.1\qwen_image_2.1-Q4_K.gguf
  --vae                   C:\PROYECTOS\LocalMind\models\diffusion\presets\qwen-image-2.1\vae.safetensors
  --llm                   C:\PROYECTOS\LocalMind\models\diffusion\presets\qwen-image-2.1\qwen3vl_8b-Q4_K.gguf
  --llm_vision            C:\PROYECTOS\LocalMind\models\diffusion\presets\qwen-image-2.1\mmproj-F16.gguf
  --lora-model-dir        C:\PROYECTOS\LocalMind\models\diffusion\loras
  --hires-upscalers-dir   C:\PROYECTOS\LocalMind\models\diffusion\upscalers
  --embd-dir              C:\PROYECTOS\LocalMind\models\diffusion\embd
  -l                      127.0.0.1            <- explícito, aunque sea el default
  --listen-port           17861
  --max-vram              -2                   <- negativo = VRAM libre − 2 GiB
  --auto-fit
  --model-args            qwen_image_2_1_prefix_cache_type=q8_0
```

`creation_flags`: `CREATE_NO_WINDOW` (ya definida en `process.rs:16`), igual que `llama-server`.

---

## 3. Decisión 1 — Política de convivencia LLM ↔ difusión

### 3.1 Por qué las tres opciones que seWei propusieron no sirven tal cual

**Reserva dedicada (bajar el `-ngl` del LLM para dejar hueco).** 16 GB − 13 GB (pico de
Qwen-Image-2.1) = 3 GB, y eso no entra ni el GGUF del LLM ni el KV de 262K, contra los ~28 GiB de
RAM que pide sd.cpp en una máquina de 15.1 GB. Peor: degradar el offload del LLM es degradar la
función principal que hoy funciona (chat + 5 CLIs de agentes) para servir la nueva. **Rechazada.**

> **Nota de la revisión 2.** Cuando esta opción se evaluó, el cálculo se hacía contra el pico de
> `z-image-turbo` (~9 GB), el preset **pequeño**, y quedaba 16 − 9 = 7 GB: poco, pero no
> descabellado. Con `z-image-turbo` **retirado del catálogo** (§5.2), el único modelo de Nivel 1 es
> Qwen-Image-2.1 a **~13 GB**, y la aritmética deja 3 GB. **Retirar el modelo pequeño endureció esta
> opción de "mala" a "imposible".** No es un argumento para recuperarlo: es el coste honesto de la
> decisión, y la razón por la que la medición de §14 apunta al modelo grande.

**Eviction automática incondicional.** Mata sesiones de agente en mitad de un turno. `pi`, `omp`,
`opencode` y `deepseek` son procesos interactivos largos con una conexión TCP abierta al gateway;
un `502 engine_down` a mitad de un turno de edición de ficheros es daño irreversible. **Rechazada
en su forma ingenua.**

**Conmutación manual pura.** Segura, pero no cumple el requisito del dueño: la iteración es el
producto, y un ciclo manual parar/arrancar por render no es iteración. **Rechazada sola.**

### 3.2 Decisión: exclusividad conmutada con cesión **solo ante inactividad**, y restitución automática

Un `EngineCoordinator` posee un *lease* de VRAM con exactamente un titular. Tres estados:

```
llm_resident        diffusion_resident        reloading
     ^                       ^                     │
     └───────────────────────┴─────────────────────┘
```

**Regla única:** la VRAM se concede a quien la pide, y **solo se le quita a quien está inactivo**.

- **Adquisición**: quien pide la GPU es quien la usa a continuación.
  - `POST /api/diffusion/start` → pide la lease para **difusión**.
  - `POST /api/launch` (cualquier agente de `AGENT_ORDER`) → pide la lease para **LLM**.
- **Cesión**: si el titular está **ocupado**, la petición se rechaza con `409` y un motivo
  concreto. Si está **inactivo**, la cesión es automática y se registra en el log.

**Ocupado** se determina con el precedente que ya existe: `check_slots_busy(port)`
(`process.rs:865-881`), que consulta `/slots` con la clave y mira `is_processing`. Además, una
ventana de actividad reciente (`last_activity`, ya presente en `ProcessManager`, alimentado por
`touch_activity()` desde `server.rs:1828`).

**Todos** los agentes de `AGENT_ORDER = ["pi", "omp", "opencode", "web", "deepseek"]` quedan
contenidos: los cuatro primeros consumen el LLM, y `web` abre el navegador contra el puerto del
motor (`open_browser_action`, `server.rs:1364`), que sin motor abierto no tiene sentido. El
`409` lleva un campo `blocked_by` con el estado del otro motor, para que la UI pueda ofrecer el
botón exacto que lo resuelve.

### 3.3 Secuencia: arranque de difusión con el LLM inactivo

```
UI             server.rs        EngineCoordinator    ProcessManager(LLM)    ProcessManager(SD)
 |                 |                    |                    |                   |
 | POST            |                    |                    |                   |
 | /api/           |                    |                    |                   |
 | diffusion/start |                    |                    |                   |
 |---------------->|                    |                    |                   |
 |                 |-- acquire("diffusion") ->                |                   |
 |                 |                    |-- slots_busy? ---->|                   |
 |                 |                    |   false -> STOP -->|                   |
 |                 |                    |   estado=reloading|                   |
 |                 |                    |<--- stop() --------|                   |
 |                 |                    |                    |                   |
 |                 |                    |-- find_free_port(17861) [helper comun]  |
 |                 |                    |-- validate_preset() | ERR -> 400, nombra componente
 |                 |                    |-- build_sd_cmd()   |                   |
 |                 |                    |-- backend_flag_filter()  [mismo sitio que psu_unsafe_flag]
 |                 |                    |-- spawn(cwd=bin/sd-cpp) -------------->|
 |                 |                    |   estado=starting  |                   |
 |<-- 202 ---------|<-------------------|                    |                   |
 |                 |                    |     (stderr -> SSE /api/events + log)   |
 |                 |                    |                    |   new_sd_ctx...   |
 |                 |                    |                    |   svr.listen ---->|
 |                 |                    |-- poll GET /sdcpp/v1/capabilities ------>|
 |                 |                    |   cada 250 ms     |                   |
 |                 |                    |<-- 200 + JSON ----|  <- handshake (§6)|
 |                 |                    |   parse -> DiffusionCapabilities       |
 |                 |                    |   is_healthy = true (solo si parseo OK) |
 |                 |<-- SSE /api/events -|                    |                   |
```

### 3.4 Restitución del LLM: automática, y **por el mismo camino que hoy**

Cuando el motor de difusión se para (a mano, o por su propio auto-stop por inactividad,
`sd_idle_timeout_secs`, default 900 s sin trabajos), el coordinador **vuelve a arrancar el LLM**
con `cfg.last.model` + `cfg.last.profile` + `cfg.last.context` — la precedencia que `start()`
ya implementa (`process.rs:1339-1363`). Automática porque la alternativa — el chat muerto en
silencio hasta que el usuario lo note — es peor.

**Consecuencia deliberada y no negociable:** la restitución llama a `mgr.start()` y por tanto
**pasa por los guardarraíles de energía** (`start_cooldown_secs` = 120,
`max_starts_per_hour` = 4, `check_start_guard`, `process.rs:1296`). Si el guardarraíl bloquea, el
LLM **se queda apagado** y la UI lo dice con el número de segundos que faltan. No se abre una
puerta trasera para el LLM: si el hardware dice que no, es que no. Esto es lo correcto, y además
es lo que ya protege la máquina hoy.

El ciclo *stop/start* de difusión seguido de *restart* del LLM cuenta contra el tope de 4
arranques/hora. **Eso es correcto y se avisa**: el diseño no gasta menos energía, la hace
**visible**.

### 3.5 Qué ve el usuario

- **Pestaña Chat**: aviso persistente mientras `active == "diffusion"` — *"The chat is paused: the
  GPU is in use by the image generator. [Back to chat]"*. En inglés, por convención de UI copy.
- **Pestaña nueva**: el mismo aviso, en el sentido inverso, si el LLM tomó la GPU.
- **Durante la recarga**: se reutilizan **sin cambios** los campos que la UI ya sabe pintar —
  `starting_for_secs`, `eta_secs`, `verifying`, `last_error` de `ServerStatus`
  (`process.rs:26-62`). El log del hijo ya fluye por `attach_reader_threads`
  (`process.rs:804`) hacia `/api/events` (SSE) y `logs/localmind.log`. **No hay ninguna
  invención de UX de progreso: se reutiliza la existente, que ya funciona.**
- **Banner de arranque**: `start_banner` (`process.rs:858`) es el precedente; para difusión se
  escribe `diffusion_banner(preset, port, plan)` describiendo **lo que se PASA**, no lo que se
  configuró — la lección de D-44 (`process.rs:849-854`) se aplica igual.

### 3.6 Reserva de VRAM como defensa en profundidad

Independientemente de la política, el arranque de difusión pasa `--max-vram -2` (negativo = VRAM
libre − 2 GiB), con el margen configurable. Motivo: `auto-fit` de sd.cpp **no** mueve un componente
a CPU solo porque sus pesos completos excedan el VRAM, así que darle el último byteAvailable lo
invita a pelearse con el compositor de Windows. El default es generoso a propósito.

### 3.7 Si esto fuera incorrecto

Lo discuto en `risks`: la políticahmite que el LLM y la difusión nunca coexistan en 16 GB, lo que
convierte la generación en un modo que apaga el chat. Es la consecuencia honesta de los números, no
un descuido; y la alternativa (degradar el LLM) degrada lo que ya funciona.

---

## 4. Decisión 2 — `sd-server` y la postura de seguridad (C2)

### 4.1 La postura que se diseña

`proposal.md` fija la postura (**el puerto crudo no se publica; todo pasa por el proxy
autenticado**) y deja la implementación para `design`. Se diseña a eso. La implementación tiene
cinco capas, y **la primera de ellas es que el proxy, por sí solo, no es suficiente** — ver §4.4.

### 4.2 Selección de puerto

- **Default nuevo: `17861`**, con `default_sd_port()` en `config.rs`, al lado del gateway
  (`default_http_port() = 17860`, `config.rs:436`) y del motor (`default_llama_port() = 8080`).
- **Nunca el 1234 de upstream.** No por superstición: 1234 es el default *público y documentado*
  de sd.cpp, así que es el primer puerto que un escáner de `127.0.0.1` prueba. occupying un
  default conocido y público es exactamente el error que hay que no cometer.
- Resolución con `find_free_port` (`process.rs:897`), el mismo helper del LLM: preferred + 0..50.
- **`-l 127.0.0.1` explícito**, aunque sea el default upstream. Explícito es evidencia en el log
  (mismo criterio D-44) y sobrevive a un cambio de default en el build vendorizado.
- **Nunca `0.0.0.0`.** No hay bandera que lo impida; es una línea del código y una del log.

Residual conocido y aceptado: `find_free_port` hace bind, cierra y devuelve — ventana TOCTOU. Es
el comportamiento heredado del motor LLM y no se cambia aquí; §4.5 lo compensa (verificación de
identidad).

### 4.3 Forma de las rutas de proxy

**Todas bajo `/api/`**, lo que las mete **automáticamente** bajo el chequeo de D-7 en
`server.rs:532`, sin añadir un segundo camino de autenticación:

```rust
// server.rs — DESPUÉS del chequeo de auth de la línea 532, ANTES del 404 final.
if let Some(sub) = url.strip_prefix("/api/diffusion/") {
    handle_diffusion(req, method, sub.to_string(), &mgr, &sd, &cfg, origin);
    return;
}
```

El prefijo `/api/diffusion/` se resuelve **por ramas explícitas**, no por *forwarding* genérico:

| Ruta LocalMind | Método | Destino en `sd-server` |
|---|---|---|
| `/api/diffusion/capabilities` | GET | `GET /sdcpp/v1/capabilities` |
| `/api/diffusion/img_gen` | POST | `POST /sdcpp/v1/img_gen` |
| `/api/diffusion/upscale` | POST | `POST /sdcpp/v1/upscale` |
| `/api/diffusion/jobs/{id}` | GET | `GET /sdcpp/v1/jobs/{id}` |
| `/api/diffusion/jobs/{id}/cancel` | POST | `POST /sdcpp/v1/jobs/{id}/cancel` |
| `/api/diffusion/vid_gen` | POST | `POST /sdcpp/v1/vid_gen` — **solo Nivel 2, spike** |
| `/api/diffusion/start` | POST | — (LocalMind: motor) |
| `/api/diffusion/stop` | POST | — (LocalMind: motor) |
| `/api/diffusion/status` | GET | — (LocalMind: estado) |
| `/api/diffusion/presets` | GET | — (LocalMind: catálogo) |
| `/api/diffusion/image/{id}` | GET | — (LocalMind: blob, §5.4) |
| `/api/diffusion/ledger` | GET | — (LocalMind: §5) |

**Denylist explícita, y es una decisión, no una omisión.** De las tres familias que expone
`sd-server` (`examples/server/routes.h`), LocalMind **no** reenvía:

- `/v1/images/generations`, `/v1/images/edits` (familia OpenAI de sd-server) — **no**. Chocarían
  conceptualmente con la superficie `/v1/*` publicada de LocalMind, que es la que consumen los
  CLIs de agentes con la clave del gateway. Añadir `/v1/images/generations` **publicaría** generación
  a cualquier CLI con la clave: es exactamente lo que el encargo prohíbe en este cambio. La
  denylist es la que convierte esa prohibición en una propiedad del código y no en una promesa.
- `/sdapi/v1/txt2img`, `/sdapi/v1/img2img`, `/sdapi/v1/upscalers`, `/sdapi/v1/latent-upscale-modes`
  (familia A1111) — **no**. Compatibilidad WebUI que LocalMind no necesita: superficie de ataque
  sin consumidor.

Superficie reenviada: **una** familia, la nativa, con seis rutas.

### 4.4 CORS: la mitigación concreta, y por qué el proxy solo no basta

**El ataque.** Una página hostil en `https://evil.example` ejecuta:

```js
fetch("http://127.0.0.1:17861/sdcpp/v1/img_gen", {method:"POST", mode:"cors", ...})
```

`POST` con `Content-Type: application/json` dispara un preflight `OPTIONS`. sd-server lo responde
con `Access-Control-Allow-Origin: <cualquier Origin>` y `Allow-Credentials: true`. El navegador
entrega la respuesta a la página. El atacante **conduce generación** en la máquina (DoS de GPU/RAM
y disco) y **lee** las imágenes (los `b64_json` son de sobra para exfiltrar). Peor: puede usar la
máquina como oráculo prompt→imagen.

**Por qué el proxy no lo arregla: el puerto crudo sigue siendo un puerto.** `sd-server` lo liga él,
no LocalMind. "No publicar el puerto" significa que LocalMind no lo anuncia ni lo enruta; **no**
que nadie pueda alcanzarlo. Y `127.0.0.1` es un **origen potencialmente confiable** en la
especificación de mixed content: una página `https://` **sí** puede hacer `fetch` a
`http://127.0.0.1`. No hay bloqueo de mixed content que nos salve.

**Las cinco capas:**

| # | Capa | Concreción | Residual |
|---|---|---|---|
| **A** | Puerto no por defecto, loopback explícito | `17861` + `find_free_port`, `-l 127.0.0.1` en argv y en el log | Una página puede escanear. Sube la barra de "trivial" a "hay que escanear". |
| **B** | Default cerrado: auto-stop | `sd_idle_timeout_secs` = 900 s sin trabajos → el motor se para solo. Botón Stop explícito. **La mayor parte del día no hay nada escuchando.** | La ventana existe mientras se usa. |
| **C** | CORS del proxy: loopback-only, sin credenciales | Se reutiliza `cors_origin_header` (`server.rs:51`) + `is_loopback_origin` (`meta.rs:177`): un `Origin` no-loopback **no** recibe ACAO. Y **`/api/diffusion/*` nunca envía `Access-Control-Allow-Credentials`** — a diferencia de sd-server. | Ninguna. |
| **D** | La cookie no viaja cross-site | `set_cookie_value` (`auth.rs:78-80`) ya es `HttpOnly; SameSite=Strict`. Una página hostil **no puede** adjuntar `lm_key` ni aunque el chequeo de Origin se pasara por alto: el navegador directamente no la manda. | Ninguna. |
| **E** | Firewall de Windows (opt-in, elevado) | Acción de usuario explícita "Blind the diffusion port": `netsh advfirewall firewall add rule` de entrada que deniega el puerto para todo. Reportado en la UI como `hardened: true/false`. Requiere elevación → es una acción del dueño, no un paso de `apply`. | Si el dueño no lo ejecuta, sigue capa B como mitigación principal. |

**La capa D es la que realmente blinda la superficie del proxy**, y ya está desplegada: es
`SameSite=Strict` en `auth.rs:79`. Merece ser explícita en el diseño porque es la respuesta
correcta a "cómo evitamos que una página hostil llegue", y es la razón por la que el proxy
funciona **aunque** alguien afloja el CORS.

**Verificación de identidad (defensa adicional del proxy, no del navegador).** Antes de
reenviar, LocalMind exige que `/sdcpp/v1/capabilities` parsee **y** que la respuesta contenga un
campo que solo trae el build vendorizado. Un ocupante del puerto (la ventana TOCTOU de
`find_free_port`) recibe `502`, no tráfico. Marginal, pero cuesta tres líneas.

**Lo que NO se hace, y por qué:** no se parchea `sd-server` (regla de `config.yaml`: no se
construye `bin/`), y no se levanta un reverse-proxy propio con auth (sería el mismo puerto, solo
que con contraseña: el navegador lo alcanza igual).

### 4.5 Lo que la postura no cubre — se dice explícitamente

Cualquier proceso con los privilegios del propio usuario —una extensión del navegador con
permisos de host, o software malicioso corriendo como el usuario— alcanza el puerto de loopback
mientras el motor esté arriba. Eso **no se arregla en este cambio** y no debe pretenderse lo
contrario. La postura de D-7 protege contra quien tiene la clave, no contra quien tiene la
máquina.

---

## 5. Decisión 3 — Catálogo de difusión y la colisión del espacio de nombres `mmproj`

### 5.1 Principio: separación por **directorio**, no por filtro

`models.rs:750` y `process.rs:1189` ya excluyen `mmproj` por nombre, y eso **protege** al catálogo
LLM (§0.2). No se toca. Lo que se hace es **sacar la difusión de `models/` raíz** para que:

1. el espacio de nombres de la difusión **no herede** la exclusión (que ahí es incorrecta: su
   torre de visión es obligatoria), y
2. el `find_mmproj` del LLM **no pueda encontrar jamás** un componente de difusión (§0.2).

### 5.2 Layout

```
models/                              <- SOLO LLM. Comportamiento actual, sin cambios.
  Qwen3.8-27B-IQ4_XS_4BPW.gguf
  Ternary-Bonsai-2-27B-*.gguf
  mmproj-BF16.gguf                  <- proyector real del LLM (931 146 432 B)
  .cache/  .verified/
  diffusion/                         <- LO ÚNICO QUE SE AÑADE (aditivo)
    presets/
      qwen-image-2.1/                <- UNICO preset de Nivel 1 (revisión 2: z-image-turbo fuera)
        qwen_image_2.1-Q4_K.gguf
        qwen_image_2_1_vae.safetensors
        qwen3vl_8b-Q4_K.gguf
        mmproj-F16.gguf              <- torre de visi\u00f3n. Nombre upstream. Colisiona con
                                       <-   default_mmproj_files()[1] si viviera en models/
      h3-fl2va/  (Nivel 2)
    loras/         -> --lora-model-dir
    upscalers/     -> --hires-upscalers-dir   (ESRGAN RGB, NO upscalers latentes)
    embd/          -> --embd-dir
```

**No hay `manifest.json`.** La declaración de cada componente vive en
`[generation_diffusion.preset.*]` del TOML (§5.4), que es la autoridad. Un fichero de metadatos
al lado sería una segunda fuente de verdad que puede discrepar de la declaración — y una
segunda fuente de verdad es exactamente el modo de fallo que §5.4 evita al no deducir nada.

`models/` es dato de usuario y está ignorado por git (§0.1). Añadir `models/diffusion/` es
**aditivo**: no se mueve, no se renombra, no se borra nada de lo que hay.

### 5.3 El catálogo LLM queda intacto — demostrado, no arguido

`list_gguf_files` (`models.rs:737`) recorre la raíz y **un** nivel de subdirectorio, y de un
subdirectorio solo recoge **ficheros**. `models/diffusion/` contiene únicamente directorios
(`presets/`, `loras/`, `upscalers/`, `embd/`), y `models/diffusion/presets/` está a **dos**
niveles. Por lo tanto **cero** componentes de difusión alcanzan la lista LLM. Lo mismo para
`list_models` (`process.rs:1173`), que solo mira ficheros de la raíz.

Esto no es un argumento: se convierte en **test de regresión** (§8, prueba 6) sobre un
árbol de fixture, comparando `list_gguf_files` con y sin `models/diffusion/`.

### 5.4 Componentes declarados: `models::list_diffusion_components`

**Decisión de la revisión 2 (2026-09-30): LocalMind no deduce el papel de un componente.** Se
retira el `DiffusionRole` de 18 variantes, la heurística de nombre (`*vae*` → `Vae`, `mmproj*` →
`VisionEncoder`, `*t5*` → `T5Xxl`…), la lectura del `manifest.json` y la comparación
manifest-vs-nombre. En su lugar, **el preset declara cada componente**:

```toml
[generation_diffusion.preset.qwen-image-2.1]
family = "qwen-image-2.1"
diffusion_model = "presets/qwen-image-2.1/qwen_image_2.1-Q4_K.gguf"
text_encoder     = "presets/qwen-image-2.1/qwen3vl_8b-Q4_K.gguf"
vae              = { path = "presets/qwen-image-2.1/qwen_image_2_1_vae.safetensors", family = "qwen-image-2.1" }
vision_encoder   = "presets/qwen-image-2.1/mmproj-F16.gguf"
```

El tipo de un componente es **la flag que el preset le asocia**, no un enum que alguien clasifique:

```rust
pub struct ComponentSpec {
    pub key: String,        // "diffusion_model" | "text_encoder" | "vae" | "vision_encoder" | …
    pub flag: String,       // "--diffusion-model" | "--llm" | "--vae" | "--llm_vision" | …
    pub path: PathBuf,      // la ruta DECLARADA, resuelta a absoluta en build_sd_cmd
    pub family: Option<String>, // T2: la familia que el usuario afirma
    pub required: bool,
}
```

Lo que queda de enumeración en disco es **enumeración, no clasificación** — su único consumidor es
la vista de modelos y el diagnóstico:

```rust
pub struct DiffusionComponent {
    pub rel: String,            // relativo a models/diffusion/, separadores "/"
    pub name: String,
    pub ext: String,            // "gguf" | "safetensors" | "ckpt" | "pt"
    pub bytes: u64,
}
```

**Extensiones aceptadas: `.gguf`, `.safetensors`, `.ckpt`, `.pt`.** **Sin exclusión por `mmproj`** —
deliberado y explícito, y es lo contrario de lo que hace el catálogo LLM. **`DiffusionComponent` no
lleva `role` ni `family`**: no hay nada que deducir.

**El hueco de `.safetensors` se cierra solo del lado de difusión.** El catálogo LLM **no** pasa a
aceptar `.safetensors`: hacerlo rompería el contrato de `POST /api/start` (que valida contra la
lista), `resolve_model_filename` (`process.rs:1914`) y el chequeo explícito de
`import_model_from_path` (`process.rs:1160`). Es un cambio de alcance, no un arreglo de un filtro.

**Lo que se pierde, y es un precio aceptado:** la guardia de familia (T2, §5.6) pasa a comprobar la
`family` **declarada**, no la descubierta. Sigue atrapando el error más común —copiar la línea de un
preset a otro—; ya no puede detectar una `family` **mentirosa**. Declarar la familia correcta es
contrato del usuario y se documenta como tal. Adivinar es lo que produce basura silenciosa, y
silencio es exactamente lo que este rediseño elimina.

### 5.5 Endurecimiento de `find_mmproj` (el arreglo del riesgo §0.2)

De:

```rust
cfg.mmproj.files.iter()
   .map(|f| self.models_dir.join(f))
   .find(|p| p.exists())
```

A una función **pura y testeable** (`process.rs`):

```rust
/// Candidatos de mmproj del LLM, en orden. `diffusion_owned` son rutas que
/// pertenecen al catálogo de difusión y NUNCA pueden servir de proyector LLM.
pub fn pick_mmproj(
    candidates: &[PathBuf],
    diffusion_owned: &[PathBuf],
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    candidates.iter()
        .find(|p| exists(p) && !diffusion_owned.iter().any(|d| p.starts_with(d)))
        .cloned()
}
```

Y `find_mmproj` pasa a sondear **dos** rutas por candidato, en este orden:

```
models/llm-mmproj/<file>      <- ruta explícita y sin ambigüedad para el LLM (nUEVA, opcional)
models/<file>                 <- histórico, se conserva
```

**Porque mantener el sondeo plano en `models/` es obligatorio:** el `mmproj-BF16.gguf` que hoy
existe (medido: 931 146 432 B) está en `models/` raíz, y moverlo sería tocar dato de usuario.

La garantía real es `diffusion_owned`: `models/diffusion/` es del catálogo de difusión, así que
`models/diffusion/presets/qwen-image-2.1/mmproj-F16.gguf` queda excluido por construcción aunque
alguien lo registre en `[mmproj] files`. Y si alguien deja un `mmproj-F16.gguf` suelto en
`models/` raíz, el escáner de difusión lo **reclama** (por nombre, `VisionEncoder`) y el
`diffusion_owned` impide que lo use el LLM. **El aviso:**

```
Hay una torre de visión de difusión llamada «mmproj-F16.gguf» bajo
models/diffusion/. No se usará como proyector multimodal del LLM.
```

Log a nivel `warn`, una vez, en el arranque del motor — porque un conflicto silencioso es
precisamente el modo de fallo que este rediseño elimina.

### 5.6 Presets: bundle declarado y validado (riesgo T2)

```rust
pub struct DiffusionPreset {
    pub id: String,                // "qwen-image-2.1"
    pub name: String,              // UI copy, inglés
    pub family: String,            // "qwen-image-2.1" | "h3"
    pub license: LicenseNote,      // id SPDX + non_commercial + notice  (C1)
    pub vram_peak_gb: f32,         // declarado; lo mide el Nivel 1
    pub vram_class: VramClass,     // Exclusive (ver §3.1)
    pub max_resolution_mp: f32,    // 1.0 — techo de la atención MATH SDPA (O(n²))
    pub memory_args: Vec<String>,  // INMUTABLES (§6.3), aplicados DESPUÉS del filtro
    pub components: Vec<ComponentSpec>,  // lo que el preset DECLARA (§5.4)
    pub scan_dirs: Vec<ScanDir>,
}
```

```rust
/// PURA. Devuelve el argv resuelto, o la lista de componentes que faltan.
/// Nunca `unwrap`, nunca I/O: recibe lo declarado y el estado del disco.
pub fn validate_preset(
    preset: &DiffusionPreset,
    disk: &DiskView,
) -> Result<ResolvedPreset, Vec<MissingComponent>>;
```

Sus comprobaciones son, exactamente, tres: (a) la ruta declarada existe; (b) la extensión es de la
aceptada para esa flag; (c) la guardia de familia. Nada más — y nada menos, porque lo que no se
comprueba aquí se comprueba en `filter_denied_flags` (§6.2).

`MissingComponent` lleva **clave de componente, ruta esperada, dónde buscó y qué encontró**, porque
el modo de fallo que se quiere evitar es el silencioso:

```
Falta el componente «vae» para el preset «Qwen-Image-2.1».
Declarado en [generation_diffusion.preset.qwen-image-2.1]:
  vae = "presets/qwen-image-2.1/qwen_image_2_1_vae.safetensors"
Busqué en models/diffusion/ y no está.
Los VAE de Qwen-Image-2.1 no son intercambiables con los de Qwen-Image ni Wan 2.2.
```

**La guardia de familia (T2)** vive en `ComponentSpec::family`: un componente declarado con
`family` distinta de la del preset es **error de validación**, no un arranque, y se detecta **antes**
del `spawn()`. Lo que ya no puede hacer —porque no lee metadatos— es detectar un `family` mentiroso
(§5.4).

`GET /api/diffusion/presets` devuelve por preset `{id, name, license, ready: bool, missing: [...],
vram_class, memory_args, argv_preview}`. **`ready == false` ⇒ LocalMind se niega a arrancar y dice
qué falta.** Ese es el contrato.

---

## 6. Decisiones 4, 5 y 6 — handshake, filtro de flags y caché de prefijo

### 6.1 Handshake de `capabilities`

`new_sd_ctx` se construye **antes** de `svr.listen`, así que el puerto ni siquiera abre hasta que
el contexto está listo. El sondeo no es "health genérico":

1. Tras el `spawn()`, sondear `GET http://127.0.0.1:{p}/sdcpp/v1/capabilities` cada **250 ms**
   hasta un techo derivado del tamaño del bundle (`eta_secs`-style, `process.rs:1855`), con la
   misma holgura que el `eta_secs` del LLM.
2. Mientras tanto, el stderr del hijo fluye por `attach_reader_threads` (`process.rs:804`) →
   `/api/events` (SSE) y log. **El usuario ve progreso real, no un spinner mudo.**
3. `is_healthy` **solo** es `true` cuando el JSON parseó. Nunca "puerto abierto = sano", a
   diferencia del `/health` del LLM.

Parseo **tolerante**, sin pánicos, todo `Option`/default, campos desconocidos ignorados:

```rust
pub struct Capabilities {
    pub backend: Option<String>,
    pub features_by_mode:       BTreeMap<String, Vec<String>>,
    pub output_formats_by_mode: BTreeMap<String, Vec<String>>,
    pub samplers:  Vec<String>,
    pub schedulers: Vec<String>,
    pub loras:     Vec<LoraEntry>,      // name, strength_min/max, default
    pub upscalers: Vec<UpscalerEntry>,  // name, kind: Latent | EsrganRgb
    pub limits: Option<Limits>,         // min/max w/h, max_batch_count, max_queue_size, max_upscale_*
    pub upscale: bool,                  // ¿hay ESRGAN RGB cargado?
}
```

**Discovery, no números a ciegas**: la UI puebla sus desplegables de `samplers`/`schedulers`/`loras`,
elige el formato de salida de `output_formats_by_mode`, y **topa el barrido de semillas con
`limits.max_batch_count`** en vez de codificar un máximo inventado. Un campo ausente o raro
**degrada la UI, nunca impide arrancar**.

### 6.2 Filtro de flags por backend — una lista, un comportamiento

Precedente: `psu_unsafe_flag` (`config.rs:306`) se llama en `start()` (`process.rs:1448`) **antes**
del `spawn()`. El filtro de difusión vive al lado y se llama en el mismo punto del camino de
difusión.

**Decisión de la revisión 2 (2026-09-30): una lista, un comportamiento.** La revisión 1 tenía dos
severidades. El dueño lo=inline, y tiene razón por una razón concreta: dos severidades obligaban a
dos comportamientos distintos (*descartar y seguir* / *rechazar y no arrancar*), y el segundo solo
era imprescindible por un caso que la revisión 1 no aisló bien.

```rust
/// Una lista de subcadenas denegadas, un solo comportamiento:
/// todo token que contenga una de ellas SE ELIMINA y se registra UNA vez.
pub fn filter_denied_flags(argv: Vec<String>, backend: Backend) -> (Vec<String>, Vec<String>);

/// Denegadas, y solo estas:
pub const DENIED: &[&str] = &[
    "int8_convrot", "int8-convrot",             // CUELGA LA GPU EN RDNA2 (necesita WMMA/MFMA)
    "--diffusion-fa", "--flash-attn", "-fa",     // Vulkan no soporta Flash Attention
    "qwen_image_2_1_prefix_cache_type=f32",
    "qwen_image_2_1_prefix_cache_type=f16",
    "qwen_image_2_1_prefix_cache_type=auto",     // sin FA, auto cae a FP32: ~8 GiB en 15.1 GB
];
```

**Detección por *subcadena* sobre todo el argv**, no por lista exacta de flags: así se capturan
`-fa on`, `--diffusion-fa`, `--model-args key=...int8_convrot...` y las **dos grafías** del término
que documenta `exploration.md:264-265` (`int8_convrot` vs `int8_convrot`), más el nombre de
fichero `qwen_image_2.1_int8_convrot.safetensors`.

**Por qué una lista funciona donde dos severidades fallaban.** La objeción legítima a "quitarlo y
seguir" es esta: si el token `=f32` desaparece y no se pone nada en su lugar, el motor cae a
`auto` → **FP32** → ~8 GiB de RAM → OOM. Eso es **peor** que arrancar y avisar. La respuesta no es un
segundo comportamiento del filtro: es el **orden**.

```
construir argv  ->  filter_denied_flags(argv, backend)  ->  aplicar memory_args INMUTABLES  ->  spawn()
```

Los `memory_args` del preset se aplican **después** y no son sobrescribibles (§6.3), así que el
`argv` final lleva `qwen_image_2_1_prefix_cache_type=q8_0` **exactamente una vez**, se haya inyectado
lo que se haya inyectado. Un comportamiento, dos invariantes, y el orden escrito como contrato
porque es lo que las sostiene.

**Defensa a nivel de catálogo:** ningún preset declara una ruta ni una flag que contenga una
subcadena de `DENIED`. Se verifica sobre el catálogo embebido **entero**, como test de `cargo test`:
es una invariante de los datos, no una comprobación en tiempo de ejecución. Y la razón de que exista
la lista entera sigue siendo una sola, y no es una preferencia: **`int8_convrot` cuelga la GPU en
RDNA2**. Un `int8*` que llegue al `spawn()` es un driver colgado, no un error.

### 6.3 `qwen_image_2_1_prefix_cache_type` = `q8_0`, y por qué "no es una opción" es mecánico

`q8_0` no es un default de configuración: es **inmutable** (`memory_args`), y se aplica **después**
del filtro de flags (§6.2), lo que lo convierte en invariante.

- **El default.** `memory_args = ["qwen_image_2_1_prefix_cache_type=q8_0"]` en el preset
  `qwen-image-2.1`, aplicado como `--model-args qwen_image_2_1_prefix_cache_type=q8_0`.
- **Por qué es obligatorio y no ajustable.** Sin FA, `auto` cae a **FP32**: `f32` son 4 GiB por
  condición × 2 condiciones ≈ **8 GiB de RAM** en un equipo de **15.1 GB** (`exploration.md:249-251`).
  `q8_0` son ~1.06 GiB. R2.
- **El refuerzo.** `filter_denied_flags` **elimina** del `argv` cualquier token que fije `f32`,
  `f16` o `auto` en esa clave, y los `memory_args` inmutables se aplican **después**, de modo
  que el `argv` final lleva `q8_0` exactamente una vez. Sin ese orden, "q8_0 por defecto"
  sería una sugerencia que un `extra_flags` mal puesto deshace en silencio. **Con el orden, es una invariante.**
- **La UI lo explica.** El preset expone `memory: { prefix_cache_type: "q8_0", approx_gib: 1.06,
  reason: "Without flash attention, the `auto` value falls back to f32 (~4 GiB per condition × 2)" }`,
  para que el dueño vea el porqué en la ficha, no solo el número.
- **El arranque lo dice.** El banner de difusión nombra el argumento que se **pasa** (D-44).

---

## 7. Decisión 4 — Libro de semillas

### 7.1 Por qué existe

`POST /sdcpp/v1/img_gen` **no devuelve la semilla usada**. `seed` es un entero único (`-1` =
aleatorio), no hay rango ni incremento, y `batch_count` solo informa `index`. Por tanto, sin un
libro, `seed: -1` produce resultados **irreproducibles** y "esa me gusta, cambiarle el texto y
volver a esa" es imposible. LocalMind **es** el dueño del libro.

### 7.2 Ubicación y forma — siguen las convenciones que ya existen

Se replica exactamente el par de `usage.rs` (bitácora append-only + índice cacheado), que es la
única convención de persistencia no-TOML del proyecto:

```
%APPDATA%\LocalMind\generation\
  ledger.json        # índice autoritativo; tope 2000 imágenes, poda por edad
  events.jsonl       # bitácora append-only; rotación 1 MiB → 5000 líneas (mismo criterio que usage.rs:37-38)
  images\<image_id>.png
```

- `usage_path()` (`usage.rs:16`) resuelve `%APPDATA%\LocalMind\…` con **override por env**
  `LOCALMIND_USAGE_PATH` para tests. `ledger.rs` replica el patrón con
  `LOCALMIND_GENERATION_DIR`, sin inventar nada.
- La configuración ** editable vive en el TOML**: `config.rs` gana `[generation_diffusion]` con
  `sd_port`, `sd_idle_timeout_secs`, `max_vram_reserve_gib`, `vram_policy`,
  `backend` — siguiendo el patrón de `EngineConfig`/`MmprojConfig`, con `#[serde(default = …)]`
  para que un TOML viejo cargue sin migración.

### 7.3 Esquema — plano: un registro por generación

**Decisión de la revisión 2 (2026-09-30).** La revisión 1 tenía tres niveles
(`serie → generación → imagen`) y usaba la serie como "hilo de iteración". El dueño lo=inline: **un
registro por generación**, y el historial es una lista plana ordenada por fecha. "Volver a esa y
cambiarle el texto" se resuelve **rehidratando ese registro**; no hace falta un hilo que lo agrupe,
y un hilo que el usuario nunca nombra es una decisión de modelo de datos disfrazada de función.

```rust
pub struct Generation {
    pub id: String,            // "g_<unix>-<rand6>"
    pub created_ts: i64,
    pub job_id: Option<String>,     // el id que devolvió sd-server
    pub preset_id: String,
    pub kind: GenerationKind,       // ImgGen | Upscale
    pub seeds: Vec<i64>,            // longitud = batch_count. ASIGNADOS POR LOCALMIND
    pub draft: Draft,               // copia EXACTA de lo enviado
    pub status: JobStatus,          // Queued|Generating|Completed|Cancelled|Failed
    pub image_ids: Vec<String>,
    pub error: Option<String>,
}

pub struct Image {
    pub id: String,            // "i_<unix>-<rand6>"
    pub generation_id: String,
    pub index: usize,          // el `index` que devolvió la API
    pub seed: i64,             // seeds[index] — la correspondencia la sabe LocalMind
    pub width: u32, pub height: u32,
    pub output_format: String, pub mime_type: String,
    pub bytes: u64, pub rel_path: String,
}
```

**Lo que se va:** `Series`, `series_id`, `parent_id`, `label`, `generation_ids`, y con ellos el
agrupador del índice, la lista por serie de la UI y la cascada de la poda. **Lo que se queda, y es
lo que importa:** el registro sigue llevando prompt, semilla(s), `preset_id`, **todos** los parámetros
de generación, la ruta de salida y el timestamp —que es exactamente lo que el corte 1 pedía— y el
historial sigue siendo la superficie de "volver a esa".

`Draft` es **versionado** (`draft_version: u32`) y contiene cada parámetro que la UI expone:
`prompt`, `negative_prompt`, `sampler`, `scheduler`, `steps`, `cfg`, `clip_skip`, `width`,
`height`, `batch_count`, `seed_base`, `loras[]`, `hires{}`, `init_image_id`, `mask_image_id`,
`strength`, `denoise`, `taesd`, `vae_tiling`. **Más** los `model_args` de solo lectura del preset,
para que el borrador siga siendo reproducible si el preset cambia.

### 7.4 Orden de escritura — esta es la parte que no se puede hacer mal

```
1. next_seeds(seed_base, batch_count)          -> local, determinista, pura
2. ledger.begin_generation(...)               -> ESCRIBE A DISCO   ◄── ANTES de la llamada
3. POST /sdcpp/v1/img_gen  { seed: seeds[0] ... }   (nunca -1)
4. ledger.attach_job(gen_id, job_id)
5. GET  /sdcpp/v1/jobs/{id}  hasta completed | failed | cancelled
6. blob -> images/<image_id>.<ext>
7. ledger.complete_generation(gen_id, image_ids)
```

**El paso 2 va antes del 3, deliberadamente.** Si el proceso de LocalMind se muere durante un
render de 15 minutos, el ledger ya tiene la semilla y el borrador completo: al reabrir, la
generación se puede relanzar con `seed` idéntico. Si se escribiera después, un crash perdería
justo la información cuyo motivo de existir es sobrevivir a un crash. Esta es la propiedad que
convierte el ledger en un taller y no en un panel de demostraciones.

### 7.5 Relación con lo ya persistido

Nada: hoy LocalMind **no persiste ninguna imagen generada**, así que no hay migración ni
reconciliación. El ledger **es** el índice del almacén de imágenes; `image_id` es la clave de
unión. Lo único reutilizado del catálogo LLM es `enrich_model_entry` (`models.rs:832`) para
componentes **de modelo**, no para imágenes — deliberadamente, porque su forma
(`filename`/`sha256`/`verified`/`source`) es de fichero de modelo y forzarle imágenes sería una
mentira de esquema.

Servir la imagen: `GET /api/diffusion/image/{id}` (autenticado, mismo origen que la WebView, la
cookie `lm_key` ya está puesta por `auth.rs:78`).

---

## 8. Plan de pruebas

`strict_tdd: false`. **Nada de TDD.** Pero sí tests unitarios de las **funciones puras**, con el
precedente de `build_engine_cmd` (`process.rs:2960`).

### 8.1 Unitarios (`cargo test`, base 142 tests, nombres en español)

| # | Función pura | Qué prueba |
|---|---|---|
| 1 | `build_sd_cmd` | argv: exe en `bin/sd-cpp/`, cwd en `bin/sd-cpp/`, `-l 127.0.0.1` presente, puerto ≠ 1234, **todos** los componentes declarados presentes, todas las rutas absolutas, `memory_args` presentes **una** vez |
| 2 | `filter_denied_flags` | argv limpio intacto; `--diffusion-fa` fuera + un único log; `-fa on` fuera; **ambas** grafías de `int8_convrot` fuera, incluso dentro de `--model-args` y como nombre de fichero; `=f32` fuera; y tras aplicar los `memory_args` inmutables, `q8_0` aparece **exactamente una vez** |
| 3 | `parse_capabilities` | cuerpo completo realista; `{}`; claves extra/desconocidas; `limits` ausente. **Ninguno** puede panicar; todos devuelven `Defaults` degradados |
| 4 | `validate_preset` | ruta declarada ausente → error nombrando clave, buscados y dónde; `.safetensors` **aceptado** (cierra el hueco); `mmproj-*.gguf` **aceptado** sin exclusión; **VAE de otra familia → error** (T2) |
| 4b | catálogo embebido | **ningún** preset declara una ruta o flag con una subcadena de `DENIED` (§6.2). Invariante de datos, no de ejecución |
| 5 | ledger | `next_seeds(base, n)` = consecutivos; append/prune/rehydrate; **orden**: `begin_generation` escribe antes del `POST`; el historial sale **plano** y ordenado |
| 6 | `list_gguf_files` | **no contaminación**: con `models/diffusion/presets/x/y.gguf` en el fixture, el resultado es **idéntico** al del mismo árbol sin ese subárbol |
| 7 | `pick_mmproj` | con `models/diffusion/.../mmproj-F16.gguf` presente → **no** lo elige; `models/mmproj-BF16.gguf` → sí; el orden de candidatos se respeta |
| 8 | selección de puerto sd | nunca devuelve 1234; respeta `find_free_port` |

### 8.2 Harness Node

| Harness | Estado | Qué |
|---|---|---|
| `tests/ui-render.mjs` | Modificar | Controles nuevos de la pestaña (stub, offline). **Explícitamente insuficiente** para estructura. **Sin checks de A/B** (revisión 2). |
| `tests/ui-tabs.mjs` | **Modificar — y esto se suele olvidar** | La lista `TABS` de `tests/ui-tabs.mjs:25` está **hardcodeada a 7**. Sin añadir `"generate"`, **la pestaña nueva no se cubre en absoluto** y el harness seguirá en verde. **Tarea explícita.** Puerta **dura**: `proposal.md` C4, `config.yaml` §testing (`conditional_gates: structural HTML edit`) y el P0 documentado en `tests/ui-tabs.README.md`. |
| `tests/smoke.mjs` | **Sin cambios** | **Revisión 2: no se toca.** Un aserto de forma contra el motor real no añade señal al arnés live, y la forma de la superficie LLM publicada la cubre `tests/e2e.mjs`, que consume la API como un cliente externo de verdad. |
| `tests/diffusion.mjs` | **Nuevo, live-only** | **El arnés único de difusión, con dos secciones.** *Superficie* (`--mode surface`, **no** necesita motor): 401 sin credencial, denylist, CORS, 404 con `enabled = false`. *Iteración* (motor real): ledger ↔ semilla, cancelar sin perder la cola, barrido con `max_batch_count` respetado, rehidratación, los dos upscales, arranque sin OOM, handshake, cambio de preset. **Sin caso A/B** (revisión 2). **No puede ser offline**: necesita el motor para la sección de iteración. |
| `tests/diffusion-vram-probe.mjs` | **Nuevo, live-only** | **Instrumento de la medición de VRAM** (§14): pico de VRAM y RAM del proceso `sd-server` con reloj de pared, salida a JSON. Es el número que nadie tiene. |
| `tests/diffusion-bench.mjs` | **Nuevo, live-only** | Arnés de medición del **Nivel 2**: reloj de pared + RAM pico por trabajo, varias combinaciones de resolución/frames/pasos, salida a JSON. Es el instrumento del informe. |
| `node --check` sobre `tests/*.mjs` | Extender | Los tres ficheros nuevos entran en el gate de sintaxis. |
| `tests/bench.mjs --self-test` | Sin cambio | No toca este dominio. |

**Lo que NO se propone:** un test que "verifique" la iteración offline. No se puede: la
reproducibilidad es una propiedad del par (semilla enviada, semilla guardada) más el resultado del
motor, y sin motor no hay resultado. Un test que simule el motor daría verde sobre
comportamiento no ejercido — precisamente lo que `config.yaml` §`strict_tdd_rationale` dice
evitar.

---

## 9. Reversión — verificada, y corregida respecto a la propuesta

### 9.1 Lo que la propuesta afirma y lo que se verifica

`proposal.md:396` pide verificar con `git status` que `bin/` solo tiene lo añadido. **Imposible**
(§0.1: `.gitignore:13`, 0 ficheros versionados). Se sustituye por:

**(a) Garantía estructural — la colisión es imposible por construcción.** Tras añadir
`bin/sd-cpp/`, exactamente **una** entrada nueva puede existir en `bin/`. Un test lo verifica:
el conjunto de ficheros de `bin/` raíz es idéntico al previo, y `bin/sd-cpp/` **no** contiene
ningún nombre de fichero que ya exista en `bin/` raíz con tamaño distinto. Esto no es una
comprobación de "no toqué nada": es que **el diseño hace que no se pueda tocar nada**.

**(b) Manifiesto de hashes — la prueba real.** Antes de añadir `bin/sd-cpp/`, capturar
`Get-FileHash SHA256` de los **70 ficheros preexistentes** de `bin/` en
`%APPDATA%\LocalMind\bin-manifest.json`. Una tarea de verificación re-hashea y diffea. **Esa** es
la evidencia, y es la que git no puede dar. (Opcionalmente versionar el manifiesto en
`docs/` — son datos del repositorio, no secretos.)

**(c) `models/`.** Igual: git no puede. Se verifica por convención y por test: solo se crea
`models/diffusion/`; nada bajo `models/` se escribe, mueve ni borra. La prueba 6 de §8.1 demuestra
que el catálogo LLM no cambia **por Presence** del subárbol nuevo.

### 9.2 Reversión por nivel (corregida)

| Nivel | Reversión | Efecto residual |
|---|---|---|
| **Nivel 1 completo** | Apagar `[generation_diffusion]` en `config.rs` deja LocalMind exactamente como está. Borrar `bin/sd-cpp/` y `models/diffusion/`. | Ninguno en el LLM. Ficheros sin usar en disco. |
| **Incremental** | Cada piece se retira por separado: UI → revertir `ui.html`; proxy → revertir `server.rs`; motor → revertir `diffusion.rs` + `presets.rs`; ledger → revertir `ledger.rs`. | Igual que el estado previo de esa piece. |
| **A mitad** | Abandonar la rama y volver al último commit. El estado del LLM nunca estuvo en riesgo: **su ruta de arranque no se toca** (`build_engine_cmd` no se modifica; solo se le añade una rama hermana para difusión). | Ninguno. |

**Punto de no retorno: ninguno para el LLM.** La reversión no requiere reinstalar, recompilar ni
redescargar el motor LLM. Verificado: `build_engine_cmd` (`process.rs:634`) recibe **una función
hermana**, no una modificación; el único cambio en el camino del LLM es `find_mmproj`
(§5.5), que solo **estrecha** lo que ya se aceptaba.

**Puertas tras revertir:** `cargo test --manifest-path src-rust/Cargo.toml` (142 base),
`node tests/ui-render.mjs`, `node tests/bench.mjs --self-test`, `node --check` sobre
`tests/*.mjs`, y **`node tests/ui-tabs.mjs`** si hubo edición estructural de `ui.html` — más
`node tests/diffusion.mjs` y `node tests/e2e.mjs` si se añadieron rutas `/api/diffusion/*`.
(`node tests/smoke.mjs` sigue siendo una puerta viva del proyecto por `config.yaml`, pero **no**
lleva aserciones de difusión desde la revisión 2.)

---

## 10. Interfaz de `EngineCoordinator`

```rust
/// Quién tiene la GPU ahora. El estado no se adivina: es explícito y consultable.
pub enum EngineHolder { Llm, Diffusion }

pub enum VramPolicy {
    /// Cede automáticamente al titular inactivo; restituye el LLM al soltar.
    Auto,
    /// Nunca expulsa al otro motor. El usuario para y arranca a mano.
    Manual,
}

/// Concede/cede la VRAM. La ÚNICA fuente de verdad de quién la tiene.
pub struct EngineCoordinator {
    holder: RwLock<EngineHolder>,
    policy: RwLock<VramPolicy>,
    llm_busy: Box<dyn Fn() -> bool + Send + Sync>,
    stop_llm:  Box<dyn Fn() + Send + Sync>,
    start_llm: Box<dyn Fn() -> Result<u32, String> + Send + Sync>,
    stop_sd:   Box<dyn Fn() + Send + Sync>,
}

/// `Err` con motivo ESPAÑOL y `blocked_by` cuando el titular está ocupado.
pub enum AcquireError {
    Busy { blocked_by: EngineHolder, retry_in_secs: u64 },
    Guardrail { message: String },     // p. ej. cooldown de energía (LM-NF-3)
}
```

`llm_busy` se implementa con `check_slots_busy` (`process.rs:865`), que ya habla `/slots` con la
clave del gateway. Las dependencias por *closure* en vez de por referencia a `ProcessManager` son
lo que hace testeable el coordinador **sin matar procesos**, siguiendo el criterio que ya está
escrito en `new_with_log_file` (`process.rs:176-182`): "la env global se resuelve una vez y el
núcleo puro recibe el path, porque los tests corren en paralelo".

---

## 11. Migración / rollout

**Sin migración de datos.** No hay capacidad publicada que modificar (`openspec/specs/` vacío) y
`localmind.toml` existente carga sin tocarlo: todas las secciones nuevas llevan
`#[serde(default = …)]`, exactamente como `MmprojConfig` (`config.rs:161-178`).

**Rollout en dos incrementos**, coherente con los dos niveles del `proposal`:

1. **Motor + catálogo + libro + UI de imagen.** Sin nada de vídeo. **Un** preset de Nivel 1:
   `qwen-image-2.1` (default, licencia no comercial marcada).
2. **Spike de H3.** Preset `h3-fl2va` + `/api/diffusion/vid_gen` + `tests/diffusion-bench.mjs`.
   **Sin pestaña de vídeo.** Un informe negativo es un entregable válido.

El interruptor `[generation_diffusion] enabled` (default **false** en el primer arranque tras
actualizar, **true** después del primer arranque con el flag) permite que la reversión del §9 sea
un cambio de configuración, no un `git revert`.

---

## 12. Matriz de amenazas

`references/threat-matrix.md` está orientada a repositorios y automatización de VCS. Se
transcribe con aplicabilidad explícita y **sin inventar filas**:

| Frontera | Casos adversarios mínimos | Aplicabilidad | Respuesta | Pruebas |
|---|---|---|---|---|
| Rutas tipo documentación | `requirements.txt`, `CMakeLists.txt`, README ejecutable | **N/A** — este cambio no clasifica ficheros de repositorio ni ejecuta rutasDocumentation | — | — |
| Selección de repositorio git | `git -C`, relativas, absolutas | **N/A** — este cambio no invoca git en runtime | — | — |
| Estado de commit | staged, `commit -a`, índice vacío | **N/A** — sin automation de VCS en runtime | — | — |
| Estado de push | tracking, primer push, refspec | **N/A** — sin push en runtime | — | — |
| Comandos de PR | `--head`, prefijo de env, compuestos | **N/A** — sin PR en runtime | — | — |

**Matriz local** (las fronteras que este cambio **sí** cruza). Es la que debe propagarse a `tasks`:

| Frontera | Casos adversarios | Respuesta de diseño | Prueba |
|---|---|---|---|
| **Clasificación de ficheros** | `mmproj-F16.gguf` de difusión suelto en `models/` raíz; `.safetensors` descartado por el filtro `.gguf`; `.gguf` de difusión colado en el catálogo LLM | Separación por **directorio** (§5.1-5.3); escáner de difusión con extensiones propias y **sin** exclusión `mmproj` | #4, #6, #7 |
| **Integración de proceso** | `ggml-vulkan.dll` de sd.cpp resuelta a la build de llama.cpp; cwd incorrecto; binario en ruta plana | `bin/sd-cpp/` (§2.3), cwd en el propio directorio, rutas absolutas | #1 |
| **Flags antes del `spawn()`** | `--diffusion-fa` en Vulkan; `int8_convrot` cuelga la GPU en RDNA2; `prefix_cache_type=f32` → 8 GiB de RAM | Filtro **duro** (rechaza) + filtro **blando** (descarta) (§6.2); rechazo de `f32`/`f16`/`auto` (§6.3) | #2 |
| **Ciclo de vida / concurrencia** | Dos motores residiendo a la vez; `\"`agente lanzado sin LLM; auto-restitución saltándose el guardarraíl de energía | Exclusividad conmutada, cesión solo ante inactividad, restitución **por `start()`** (§3) | #1, #8 |
| **Ruteo HTTP** | Hostile page → puerto crudo; `Origin` no-loopback con ACAO; ruta sd-server fuera de la allowlist | `-l 127.0.0.1` + puerto no default + auto-stop; CORS loopback-only y sin credenciales; **denylist** explícita de `/v1/images/*` y `/sdapi/v1/*` (§4) | live `tests/diffusion.mjs` |
| **Persistencia** | Crash a mitad de un render; semilla perdida; mezcla de rutas | Libro escrito **antes** del `POST` (§7.4); JSON atómico (tmp + rename, patrón `config.rs:788-791`) | #5 |

---

## 13. Cambios de ficheros

| Fichero | Acción | Qué |
|---|---|---|
| `bin/sd-cpp/` | Crear | Contenido del zip prebuilt Vulkan de sd.cpp. **Aditivo**; `bin/` raíz intacto. |
| `models/diffusion/` | Crear | Layout de presets + directorios escaneados. **Datos de usuario**; vacío hasta que el dueño descargue pesos. |
| `src-rust/src/presets.rs` | Crear | `ComponentSpec`, `PresetSpec`, `validate_preset` (puro), catálogo embebido `qwen-image-2.1` / `h3-fl2va`, `filter_denied_flags` + `DENIED`. |
| `src-rust/src/diffusion.rs` | Crear | `DiffusionManager` (hijo, estado, logs), `build_sd_cmd`, `EngineCoordinator`, `VramPolicy`, `proxy_sd`, `parse_capabilities`. |
| `src-rust/src/ledger.rs` | Crear | `Generation` / `Image` / `Draft` (**plano, sin `Series`**), `next_seeds`, almacén de blobs, poda, rehidratación. |
| `src-rust/src/process.rs` | Modificar | `new_with_log_file` deriva `sd_bin_dir`; `find_mmproj` → `pick_mmproj` + sonda `llm-mmproj` + guarda `diffusion_owned`; `pub` para `check_slots_busy` si el coordinador lo necesita. **No se toca `build_engine_cmd`.** |
| `src-rust/src/models.rs` | Modificar | `list_diffusion_components` (`rel`/`name`/`ext`/`bytes`, **sin `role` ni `family`**). **`list_gguf_files` y `dest_for` intactos.** |
| `src-rust/src/config.rs` | Modificar | `GenerationDiffusionConfig` (`sd_port`, `sd_idle_timeout_secs`, `max_vram_reserve_gib`, `vram_policy`, `enabled`, `models_subdir`, `ledger_max_images`) + **`[generation_diffusion.preset.*]`** (rutas declaradas) + `default_sd_port() = 17861`. `MmprojConfig` **intacta**. |
| `src-rust/src/server.rs` | Modificar | Rama de prefijo `/api/diffusion/` **después** del auth de la línea 532 y antes del 404; 12 handlers; rama `409` en `handle_launch_generic` cuando el LLM no tiene la lease; `acquire_cors_for` restrictivo. **Ninguna ruta existente cambia de forma.** |
| `src-rust/src/launcher.rs` | Modificar | `AGENT_ORDER` **sin cambios**; los cinco pasan por la guarda de la lease. |
| `src-rust/src/main.rs` | Modificar | Construir `DiffusionManager` + `EngineCoordinator` y pasarlos a `HttpServer::start`. |
| `ui.html` | Modificar | Pestaña `generate` en `switchTab` (`ui.html:2734`), panel `#tab-generate`, aviso de lease en Chat. **Cambio estructural → `ui-tabs.mjs` obligatorio.** UI copy en inglés. |
| `tests/ui-tabs.mjs` | Modificar | **Añadir `"generate"` a `TABS` (`tests/ui-tabs.mjs:25`)**, o la pestaña no se cubre. |
| `tests/ui-render.mjs` | Modificar | Checks stub de los controles nuevos. |
| `tests/smoke.mjs` | **Sin cambios** | **Revisión 2: el test de contrato HTTP se retira.** La superficie la cubre `tests/diffusion.mjs` (sección de superficie) y la forma publicada la cubre `tests/e2e.mjs`. |
| `tests/diffusion.mjs` | Crear | Arnés live **único**: superficie (sin motor) + iteración (motor real). Ledger ↔ semilla, cancelar, barrido, rehidratación, upscales. **Sin A/B.** |
| `tests/diffusion-vram-probe.mjs` | Crear | Instrumento live de la medición del pico de VRAM (§14). |
| `tests/diffusion-bench.mjs` | Crear | Arnés live de medición del Nivel 2. |
| `docs/models/diffusion.md` | Crear | Presets, layout, licencias (C1), el porqué de `q8_0`. |
| `openspec/changes/diffusion-generation/design.md` | Crear | Este documento. |

---

## 14. Preguntas abiertas

- [ ] **¿Existe alguna flag en el `sd-server` vendorizado para desactivar el reflejo de CORS o
      limitar `Origin`?** Si existe, es la **capa F** y sube mucho el listón de §4.4. No está en
      las strings verificadas en `exploration.md:62-64` y **no se ha ejecutado** para
      confirmarlo. Si `tasks` la encuentra, se aplica **encima** de A-E, no en lugar de ellas.
- [ ] **¿Qué pico de VRAM real da Qwen-Image-2.1 en esta máquina?** El valor del preset es
      *declarado* (~13 GB de la documentación). El Nivel 1 lo **mide**, con
      `tests/diffusion-vram-probe.mjs`, y lo corrige. Si resultara ≤ 6 GB, su `VramClass` podría
      bajar a `Shared` y convivir con el LLM — es el único camino hacia la convivencia real.
      **Expectativa honesta: > 6 GB**, y por eso el único modelo de Nivel 1 es el grande
      (§3.1). La **alternancia** que entrega `EngineCoordinator` no depende de este número.
- [ ] **Espacio en disco para los pesos** (R12): `models/` está en C: (54 GB libres) y H3 pide
      ~28.25 GiB. ¿Se mueve `models/` a E:, o se deja que el Nivel 2 se quede sin espacio y se
      reporte eso como el hallazgo? **El `proposal` no lo decide; lo decide quien lo ejecute.**
- [ ] **Retención del ledger**: el tope de 2000 imágenes es un default. Si el dueño quiere el
      archivo de por vida, ¿subimos el tope o desactivamos la poda? Afecta solo a disco, pero es
      una decisión del dueño, no una última línea de configuración.

---

## 15. Riesgo residual del diseño

- **El LLM y la difusión no conviven.** Los presets del cambio son `Exclusive`. Generar una imagen
  **apaga el chat**, y lanzar un agente **apaga el generador**. Es la consecuencia honesta de
  16 GB de VRAM con los picos reales, no un descuido. La mitigación futura (bajar `qwen-image-2.1`
  a `Shared` si la medición lo permite, §14) depende de un número que todavía no existe, y su
  resultado esperado es negativo: con **un solo** preset de Nivel 1 y siendo el grande, la
  convivencia no es un objetivo alcanzable de este cambio. Lo que sí lo es, y lo que `apply` entrega,
  es la **alternancia**.
- **La postura de C2 no es completa en Windows.** §4.5: un proceso con los privilegios del
  usuario alcanza el puerto de loopback mientras el motor esté arriba. El proxy, el CORS
  loopback-only, `SameSite=Strict` y el auto-stop reducen mucho el riesgo desde el **navegador**
  —que es el向量 de ataque de un usuario normal— pero no lo eliminan frente a software local.
  La única mitigación completa es la capa E (firewall, opt-in, requiere elevación), y debería
  ser la recomendación por defecto en la UI, no un extra.
- **La simbolización del proceso sigue siendo TOCTOU.** `find_free_port` liga, cierra y devuelve.
  Se acepta por coherencia con el motor LLM; la verificación de identidad de §4.5 acota el
  impacto a un `502`.
- **`strict_tdd: false` sobre superficie asíncrona y concurrente.** Mitigado por tests de
  funciones puras (§8.1) y gates live, pero el bucle de iteración completo —el requisito
  central— **solo se ejercita contra la app viva**. Es un riesgo asumido y declarado, no
 _one mitigated.

---

*Fase design, **revisión 2** (2026-09-30). Cambios: §3.1 (nota sobre el pico de Qwen-Image-2.1),
§5.2 (layout sin `z-image-turbo` ni `manifest.json`), §5.4 (componentes **declarados**, sin
autodetección), §5.6 (preset declarado), §6.2 (**una** lista, **un** comportamiento) y §6.3
(invariante de `q8_0` por **orden**), §7.3 (esquema **plano** del ledger), §8 (plan de pruebas),
§11, §13, §14 y §15. No se modificó código del producto. Artefacto en
`openspec/changes/diffusion-generation/design.md`; misma clave en Engram
(`sdd/diffusion-generation/design`). Siguiente: `spec` y `tasks`.*
