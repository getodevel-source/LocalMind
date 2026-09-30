# Exploración: `diffusion-generation`

Fase: **explore**. Fecha: 2026-09-30. Cambio: `diffusion-generation`.
Artefacto: `exploration.md`. Store: híbrido (OpenSpec + Engram, clave `sdd/diffusion-generation/explore`).

> Esta fase **no diseña**. Las formas de integración se comparan; la elección es de `design`.
> Los identificadores, rutas de API y flags van en inglés porque son del runtime.

---

## 0. Resumen ejecutivo: el dato que cambia la premisa

La premisa de partida de esta exploración era: *"`sd-cli` es one-shot, no hay servidor, y esa es
la mayor brecha arquitectónica con LocalMind"*. **Esa premisa es incorrecta a día de hoy.**

`leejet/stable-diffusion.cpp` **ya shippea un servidor persistente de larga duración** en el mismo
zip prebuilt de Windows x64 Vulkan que LocalMind ya vendoriza para llama.cpp:

```
sd-master-3f8527a-bin-win-vulkan-x64.zip   (28.7 MB, release master-929-3f8527a, 2026-09-27)
  ggml-vulkan.dll      42.7 MB
  stable-diffusion.dll 35.3 MB
  sd-cli.exe            0.66 MB
  sd-server.exe         1.15 MB   <-- servidor persistente
  webm.dll / libwebp.dll / libwebpmux.dll   (salida WebP y WebM)
```

**Verificado ejecutando el binario extraído en esta máquina**, no leyendo un README:

```
> sd-server.exe
[ERROR] common.cpp:846 - error: the following arguments are required: model_path/diffusion_model
stable-diffusion.cpp version unknown, commit 3f8527a
Svr Options:
  -l, --listen-ip <string>      server listen ip (default: 127.0.0.1)
  --serve-html-path <string>    path to HTML file to serve at root (optional)
  --listen-port <int>           server listen port (default: 1234)
```

Consecuencia directa: **la integración no tiene que construir un shim**. La opción (b) de la lista
de shapes no es "escribir un shim en Rust", es "usar el servidor que upstream ya escribió, con la
misma relación de hermandad que `llama-server` ↔ LocalMind". Eso reduce el alcance de
implementación y, más importante, reduce el riesgo: el motor de generación pasa a ser un
**binario vendorizado más** — exactamente el patrón que `openspec/config.yaml` ya declara
("vendored prebuilt binaries in `bin/`, NOT built from this repo's source").

---

## 1. La pregunta de arquitectura, en profundidad

### 1.1 Lo que `sd-server` expone realmente (verificado en el código fuente y en las strings del binario)

Tres familias de API, registradas en `examples/server/routes.h`:

| Familia | Rutas | Propósito |
|---|---|---|
| **OpenAI** | `POST /v1/images/generations`, `POST /v1/images/edits`, `GET /v1/models` | Compatibilidad de clientes |
| **A1111 / sdapi** | `POST /sdapi/v1/txt2img`, `/img2img`, `GET /sdapi/v1/upscalers`, `/latent-upscale-modes` | Compatibilidad con el ecosistema WebUI |
| **sdcpp nativa** | `GET /sdcpp/v1/capabilities`, `POST /sdcpp/v1/img_gen`, `POST /sdcpp/v1/vid_gen`, `POST /sdcpp/v1/upscale`, `GET /sdcpp/v1/jobs/{id}`, `POST /sdcpp/v1/jobs/{id}/cancel` | Nativo, asíncrono, con cancelación |

Verificado por strings en el `.exe` descargado: `sdcpp/v1/capabilities`, `v1/images/generations`,
`sdapi/v1/txt2img`, `/sdcpp/v1/vid_gen`, `upscale`, `listen-ip`, `listen-port`, `offload-to-cpu`,
`params-backend`, `auto-fit`, `max-vram`, `audio-vae`, `llm_vision`, `hires-upscalers-dir`,
`serve-html-path`, `video-frames`, `moe-boundary`, `temporal-tiling` — **todas presentes**.

**Esto es exactamente el contrato que LocalMind ya consume.** `server.rs` ya proxea
`/v1/chat/completions` hacia `llama-server` y ya reescribe el campo `model` hacia el servido
(`served_model_id`, `rewrite_model_to_served`). El proxy de `/sdcpp/v1/*` es la misma idea con otro
destino.

**Jobs asíncronos con cancelación** (`examples/server/async_jobs.cpp`):

```json
{ "id": "job_01HTXYZVID", "kind": "vid_gen", "status": "queued|generating|completed|failed|cancelled",
  "created": 1775401200, "started": …, "completed": …, "queue_position": 0,
  "result": { "output_format": "webm", "mime_type": "video/webm", "fps": 16,
              "frame_count": 33, "b64_json": "GkXfo59ChoEBQveBAULygQRC84EIQo..." } }
```

Esto es crítico y no estaba en el encargo: con 8–15 min por clip de H3, **`POST /sdcpp/v1/jobs/{id}/cancel`
es la diferencia entre una app usable y una app frustrante**. Un one-shot `sd-cli` habría hecho
imposible cancelar sin matar el proceso (y perder la cola).

### 1.2 Las cuatro formas, pesadas contra el código real de LocalMind

#### (a) `sd-cli` por generación, capturando el fichero de salida

- **A favor**: cero estado; si el proceso muere, no hay que recuperarse. El output va a disco
  directamente (`-o out.png`).
- **En contra, y es lo decisivo**: **destruye la UX de iteración que el dueño pidió explícitamente**.
  Un one-shot paga el coste de carga de pesos en *cada* generación. En Qwen-Image-2.1 eso es el
  codificador Qwen3-VL-8B (5.15 GB) + DiT (5.39 GB) entrando a VRAM una y otra vez. Con el VRAM
  compartido que ya es el cuello de botella, y con el LLM de LocalMind ocupando ~6 GB, cada render
  se convierte en un ciclo de descarga/eviction/recarga de segundos que se paga N veces en un
  barrido de semillas. Un `sd-cli` no puede mantener un KV/prefix cache entre peticiones ni
  devolver un job consultable. Y no hay cancelación: matar el proceso es la única forma de parar
  un render de 15 minutos.
- **Veredicto**:descartado. No por dificultad, sino porque ataca directamente el requisito
  central del dueño.

#### (b) `sd-server` como proceso de larga duración, con LocalMind como proxy ← **la que encaja**

Mapeo directo contra el código existente:

| LocalMind hoy | Extensión natural a difusión |
|---|---|
| `process.rs::build_engine_cmd` (`llama-server` + flags) | `sd-server` + flags de contexto |
| `find_free_port(cfg.engine.llama_port)` (`process.rs:1400`) | `find_free_port` para `--listen-port` (default upstream 1234) |
| `ServerStatus { status, model, port, is_healthy }` | Segundo `ServerStatus` de difusión, o campo hermano |
| `server.rs` proxy `/v1/*` → `llama-server` | proxy `/sdcpp/v1/*` → `sd-server` |
| `start_banner(context, threads, ubatch, port)` | banner de difusión con el plan de memoria |
| `strip-action` / `stripAction()` en `ui.html` | acción de arranque/parada de difusión |
| `AuthRequest`/`gateway_key` (D-7) | ver §1.4 — hueco real |

- **A favor**: cero código de motor nuevo. Conserva la forma "proceso de larga duración + API" que
  es la identidad arquitectónica de LocalMind. Da jobs asíncronos, cola, cancelación, `capabilities`
  para descubrimiento de samplers/LoRAs/upscalers, y video en WebM directo (reproducible en WebView2
  sin códecs extra).
- **En contra**: no hay endpoint de recarga de modelo (ver §1.3), no hay autenticación (ver §1.4), y
  no se puede descubrir la taxonomía de modelos desde la API.
- **Veredicto**: **recomendada**.

#### (c) "RPC mode" de sd.cpp

- **Corrección de premisa**: el modo RPC de sd.cpp **no es un plano de control de trabajos**. Es el
  `GGML_RPC` de llama.cpp — cómputo ggml distribuido (`docs/rpc.md`: compilar `rpc-server` aparte,
  luego `sd-cli --rpc-servers host:porta,...` para repartir el cómputo entre GPUs remotas). **No**
  sirve para enviar una job de generación.
- Además requiere un binario extra compilado a mano, lo que contradice la regla de
  `openspec/config.yaml` ("no propose building `bin/` from source").
- **Veredicto**: descartado. Es la opción que el nombre sugiere y no sirve para nada de lo que se
  necesita aquí.

#### (d) Reutilizar una UI/binding de terceros en vez de construir

Investigado y **descartado como camino principal**, con reservas:

- **El frontend embebido de sd.cpp no está en el binario prebuilt.** El CI oficial
  (`.github/workflows/build.yml`, job `windows-latest-cmake`) compila sin
  `-DSD_SERVER_BUILD_FRONTEND=ON`; sin `HAVE_INDEX_HTML`, el server sirve la cadena
  "Stable Diffusion Server is running". Verificado: las strings `gen_index_html` y `pnpm`
  están **ausentes** del `sd-server.exe` de 1.15 MB. La UI web upstream vive en
  `github.com/leejet/sdcpp-webui` (Vue, **MIT**), que se sincroniza cada 1–2 semanas y se puede
  pasar con `--serve-html-path`. Es un `index.html` servido por sd-server, **no** una UI
  integrable en el WebView2 de LocalMind sin reescribirla.
- **KoboldCpp** (AGPL-3.0) **sí** es un frontend de sd.cpp —confirmed por el issue
  `LostRuins/koboldcpp#2314`, donde el autor reporta usar `sd-server.exe` como backend. Es decir:
  **KoboldCpp y `sd-server` no son alternativas, son la misma pila con distinto shell**. KoboldCpp
  no puede ser el motor; como *UI* es una app web AGPL que habría que re-tematizar dentro del
  WebView2, encima del cual LocalMind ya tiene su propio diseño. AGPL además desaconseja
  copiar código a un proyecto con licencia incompatible; invocar el binario por HTTP sí es limpio.
- **sd.cpp-webui** (`daniandtheweb`, AGPL-3.0, Gradio/Python) y **stable-diffusion-cpp-python**
  (MIT, bindings de Python) reintroducen un runtime de Python — justo lo que la elección de
  sd.cpp evita. LocalMind ya es Rust de una sola dependencia, sin Python.
- Ver el detalle por candidato en §3.

- **Veredicto**: no construir un binding; sí **reusar el modelo de API y el vocabulario de
  parámetros** de sd.cpp, que ya están hechos y probados.

### 1.3 Sin recarga de modelo: `new_sd_ctx` ocurre una vez, antes de `listen`

`examples/server/main.cpp` construye el contexto **antes** de `svr.listen(...)`, y `routes.h` no
declara ningún endpoint de recarga. `routes_sdcpp.cpp` registra exactamente seis rutas y ninguna
gestiona el ciclo de vida del modelo.

**Consecuencia de producto**: cambiar de modelo de difusión = **reiniciar el proceso de difusión**,
igual que en LocalMind cambiar de LLM es reiniciar el motor. La diferencia arquitectónica es
que LocalMind tiene **dos motores** que compiten por el mismo VRAM, no uno. Eso obliga a que la
política de convivencia LLM↔difusión sea una decisión de producto explícita (§5, pregunta 3), no
un detalle de implementación.

### 1.4 `sd-server` no tiene autenticación — hueco real contra el modelo de seguridad de LocalMind

`sd-server -h` no expone `--api-key`. `server.rs:532` exige Bearer/x-api-key/cookie en `/api/*` y
`/v1/*` (decisión D-7, clave en `%APPDATA%\LocalMind\gateway.key`). Si LocalMind abre el puerto de
`sd-server` tal cual, queda un endpoint **sin autenticar** en `127.0.0.1` que cualquiera con una
sesión en la máquina puede usar para generar (y consumir disco/RAM). El CORS de sd-server además
refleja cualquier `Origin` con `Allow-Credentials: true`.

Esto es un **requisito de diseño, no una nota**: o LocalMind proxea y nunca expone el puerto crudo
(probablemente insuficiente, porque el bind es a `0.0.0.0` o `127.0.0.1` y cualquier proceso local
alcanza `127.0.0.1`), o hay que añadir una capa de red. Queda como pregunta abierta (§5, 4).

### 1.5 La gestión de memoria es el verdadero diseño, y upstream ya la resuelve

`docs/backend.md` describe un sistema de *auto-fit* que no estaba considerado:

- `--auto-fit` (**activo por defecto** si no se pasa `--params-backend`): deduce la colocación de
  parámetros desde metadatos del modelo, dispositivos de cómputo y presupuesto de memoria restante.
- `--max-vram` acepta **valores negativos** (= memoria libre menos N GiB) y budgets por dispositivo.
- `--params-backend …=disk` deja los pesos **en disco y los recarga bajo demanda**, por rangos de
  bloques contigüos.
- Reparto multi-dispositivo: "parameters are assigned to devices in contiguous ranges sized
  proportionally to each device's free memory".
- CPU: reserva "the larger of 2 GiB or 10% of available RAM for other work".

Esto **degrada la objeción de RAM**. La documentación también es honesta sobre el límite:
"Offloading weights does not guarantee that every resolution or frame count will fit, and auto-fit
does not change a component to CPU computation solely because its full weights exceed VRAM."

---

## 2. Correcciones a los hallazgos de investigación previos

Tres, todas verificadas contra fuente/binario. La primera invalida un presupuesto; la segunda
invalida un comando documentado; la tercera corrige un nombre de proyecto.

### 2.1 H3 en 15.1 GB de RAM: el presupuesto anterior se apoyaba en un modelo que no hace texto-a-video

Inventario real y medido de `huggingface.co/leejet/MiniMax-H3-GGUF` (la referencia que la
propia doc de H3 usa):

```
minimax_h3_fl2va-Q4_K_M.gguf                17.49 GiB   (fl2va, no podado)
minimax_h3_fl2va_pruned-Q4_K_M.gguf        10.64 GiB   <-- único capable de T2AV razonable
minimax_h3_ref2va_pruned-Q2_K_M.gguf        6.26 GiB   <-- Ref2AV SOLO, sin texto-a-video
minimax_h3_ref2va_pruned-Q4_K_M.gguf       10.64 GiB
qwen3vl_32b_minimax_h3-Q2_K_M.gguf          12.20 GiB
qwen3vl_32b_minimax_h3-Q4_K_M.gguf          16.97 GiB
```

El único fichero de 6.26 GiB es `ref2va_pruned` **con Q2_K**, y según `docs/minimax_h3.md` Ref2VA
**exige** material de referencia (imagen/video/audio) y no se puede combinar con `--init-img`/`--end-img`.
**No genera texto-a-video.** El presupuesto de "~8.4 GiB de RAM de 15.1 → cabe" se calculó sobre
ese fichero y, por tanto, **no aplica a H3 como texto-a-video**.

Suma realista para T2AV en esta máquina:
`10.64 (DiT) + 12.20 (TE, con `params-backend te=disk`) + 4.85 (VAE video fp16) + 0.56 (VAE audio fp32) ≈ 28.25 GiB en disco`,
con el auto-fit repartiendo DiT entre VRAM libre (~11 GiB) y RAM. **Es un caso de que el auto-fit
resuelva o no**, y el propio doc advierte que no está garantizado. La afirmación honesta es:
**H3 es experimental en esta máquina, no un pilar.** Ref2VA con el Q2_K de 6.26 GiB sí es un objetivo
más alcanzable (imagen de referencia → clip), y es un primer objetivo de iteración coherente.

### 2.2 Flash Attention NO es usable en esta máquina — y los ejemplos de la doc lo usan

`docs/performance.md`: "At the moment, it is only supported for some models and some backends
(like cpu, cuda/rocm, metal)". **Vulkan no está.** Confirmado también en el encargo (medido).
Pero `docs/minimax_h3.md` y `docs/qwen_image_2.1.md` muestran sus comandos de ejemplo **con
`--diffusion-fa` / `--fa`**. **Esos comandos son los que no hay que copiar.**

Y hay un coste de memoria oculto y grande derivado de esto, en
`qwen_image_2_1_prefix_cache_type`:

> `auto` (default): use FP16 only when Flash Attention is enabled …

**Sin FA, `auto` cae a FP32.** La doc cuantifica la caché de prefijo para el modelo de 32 capas a
4096 tokens **por condición**: `f32` 4 GiB, `f16` 2 GiB, `q8_0` 1.0625 GiB, `q4_0` 0.5625 GiB.
Como las condiciones positiva y negativa usan cachés separadas, el peor caso en Vulkan es
**~8 GiB de RAM solo en caché de prefijo**, en un equipo con 15.1 GB. Mitigación documentada:
`--model-args qwen_image_2_1_prefix_cache_type=q8_0`, o desactivar la caché. **Esto debe entrar en
el diseño como decisión consciente**, no descubrirse en el primer render.

### 2.3 "Jellybox" no existe como proyecto de este ecosistema

Busqueda en la API de GitHub y web: no hay ningún proyecto de generación de imágenes/vídeo con
difusión llamado Jellybox. Los resultados son un reproductor de música Jellybox, una app Xbox de
Jellyfin y un grabador G-code. **No hay software que reusar con ese nombre.**

### 2.4 Menores, pero reales

- El ejemplo de Qwen-Image-2.1 usa `qwen_image_2.1_int8_convrot.safetensors`. `int8_convrot`
  **cuelga la GPU en RDNA1/2** (necesita WMMA/MFMA). La propia doc ofrece la alternativa GGUF
  (`qwen_image_2.1-Q4_K.gguf`) — esa es la ruta. Nota: el flag de búsqueda es `int8_convrot`
  (sin guion bajo), el doc en la otra fuente lo escribe `int8_convrot` vs `int8_convrot`; verificar
  el spelling exacto al construir la lista de modelos.
- El repo correcto es `leejet/`, no `andangel/`. Confirmado.
- Las VAE de Qwen-Image-2.1 **no son intercambiables** con las de Qwen-Image y Wan 2.2. Un error
  silencioso de catálogo aquí produce basura, no un error.
- Las dims de imagen deben ser divisibles por 32; H3 alinea W/H hacia arriba a múltiplo de 32 y
  `video_frames` a la rejilla `17k+5` (mínimo 5). La API normaliza `video_frames` al mayor `4n+1`
  que no exceda lo pedido, y devuelve el `frame_count` real en el resultado.
- `sd-server` no está en los skills; tampoco hace falta. `bin/` queda como está.

---

## 3. Software de terceros: qué reusar y qué no

| Candidato | Licencia | ¿Headless / estilo librería? | ¿Reusar? |
|---|---|---|---|
| `leejet/stable-diffusion.cpp` (`sd-server`) | **MIT** | Sí, HTTP en `127.0.0.1` | **Sí — el motor.** Vendorizar el zip Vulkan en `bin/`, como ya se hace con llama.cpp |
| `leejet/sdcpp-webui` | **MIT** | Es un `index.html` servible (`--serve-html-path`) | **Parcial.** Referencia de parámetros y UX a estudiar; no integrable en WebView2 tal cual |
| `LostRuins/koboldcpp` | **AGPL-3.0** | Servidor HTTP propio, pero **es un frontend de `sd-server`** | **No como motor** (misma pila, distinto shell). **No para copiar código** (AGPL). Sí como *referencia de diseño de UX* |
| `daniandtheweb/sd.cpp-webui` | AGPL-3.0 | Gradio/Python | **No.** Python + AGPL, y superado por el Vue oficial |
| `rmatif/Local-Diffusion` | Apache-2.0 | Flutter, **Android**, ONNX | **No.** Otra plataforma y otro runtime; ONNX reintroduce el problema de kernels quantized en AMD |
| `william-murray1204/stable-diffusion-cpp-python` | MIT | bindings Python | **No.** Reintroduce Python |
| "Jellybox" | — | — | **No existe** (§2.3) |

**Conclusión de la pregunta "software de terceros"**: el reuso correcto **es el propio motor
upstream**, y el segundo reuso es **el vocabulario de parámetros y el modelo de capabilities**
(`/sdcpp/v1/capabilities` ya devuelve samplers, schedulers, LoRAs, upscalers, límites y defaults
por modo). Lo que **no** hay que reusar es una UI de terceros: LocalMind ya tiene su diseño
(`ui.html`, i18n, tema, 7 pestañas) y KoboldCpp/Gradio/WebUI obligarían a reconstruirlo.

---

## 4. Gestión de modelos: el catálogo actual no puede representar difusión

### 4.1 Qué existe hoy

`models.rs::list_gguf_files` (`models.rs:737`) recorre `models/` raíz + un nivel de subdirectorio,
filtra `.gguf` y **excluye cualquier nombre que contenga `mmproj`**. Devuelve `(nombre, path, bytes)`.
`server.rs:537` (`GET /api/models`) lo serializa como `{filename, name, size_gb, path, rel}` más
`enrich_model_entry` (sha256 / verificado). La tabla de `ui.html:1739` tiene cuatro columnas:
Modelo, Tamaño, Estado, Acción.

Todo eso asume **un modelo = un fichero = un motor que lo sirve**. Para difusión eso es falso: un
modelo es un **conjunto de componentes tipados**, y cada uno va a una flag distinta.

### 4.2 Lo que sd.cpp necesita (de la lista real de flags de `sd-server -h`)

| Componente | Flag | Nota |
|---|---|---|
| DiT / transformer de difusión | `--diffusion-model` | obligatorio; o `--model` para un pack único |
| DiT de alto ruido (H3/wan hybrid) | `--high-noise-diffusion-model` | |
| DiT incondicional (Ideogram4 CFG) | `--uncond-diffusion-model` | |
| VAE | `--vae` (+ `--vae-format`) | |
| **VAE de audio** | `--audio-vae` | H3 lo necesita; fp32 obligatorio |
| VAE diminuto (rápido, baja calidad) | `--taesd` | |
| **Encoder de texto** | `--llm` (+ `--llm_vision` para visión) | el de H3 es 12.20 GiB |
| Encoders CLIP | `--clip_l`, `--clip_g`, `--clip_vision` | |
| T5/XXL | `--t5xxl` | |
| Tokenizer (PiD, Lens) | `--tokenizer` | |
| ControlNet | `--control-net` | |
| IP-Adapter | `--ip-adapter` | requiere `--clip_vision` |
| Módulo de movimiento (AnimateDiff) | `--motion-module` | habilita vídeo con `--video-frames > 1` |
| Encoder de audio (Wan2.2 S2V) | `--audio-encoder` | |
| Conector de embeddings (LTXAV) | `--embeddings-connectors` | |
| **LoRAs** | `--lora-model-dir` | directorio, escaneado |
| **Upscalers** | `--hires-upscalers-dir` | directorio, escaneado |
| Embeddings | `--embd-dir` | |

### 4.3 La tensión concreta con el layout actual

`dest_for` (`models.rs:402`) pone **un solo `.gguf` plano en `models/`** y multi-fichero en
`models/<repo>/`. Funciona para LLM. Para difusión, los componentes se organizan por
**familia de modelo**, no por repositorio: la doc upstream asume
`../models/diffusion_models/…`, `../models/vae/…`, `../models/text_encoders/…`. Además
`list_gguf_files` excluye por nombre `mmproj`, y para Qwen-Image-2.1 la edición **requiere**
`mmproj-Qwen3VL-8B-Instruct-F16.gguf` — o sea, un componente obligatorio quedaría **invisible** en
el catálogo actual. Y los VAE son a menudo `.safetensors`, que el filtro `.gguf` descarta entero.

Además, el selector de la UI (`model-select`, `ui.html:1756`) alimenta `POST /api/start` con
`model`, y `rewrite_model_to_served` fuerza **un** nombre servido. El modelo de "un motor, un
modelo" es correcto para LLM e insuficiente para difusión: hacen falta **presets** (un preset =
un coherent set de rutas de componentes), no un `filename`.

Nota: `GET /v1/models` existe en sd-server, pero devuelve el **modelo ya cargado**, no un catálogo
de lo disponible en disco. Descubrimiento de LoRAs/upscalers/samplers sí viene en
`/sdcpp/v1/capabilities` (por proceso, con lo que cargó ese proceso).

---

## 5. La UX de iteración: qué significa "muy bien implementada" en concreto

Este es el requisito central del dueño, así que se explicita antes de que `design` lo converts en
especificación. Ninguna de estas decisiones se toma aquí.

### 5.1 Hallazgo que condiciona todo el diseño de iteración

**Ni `img_gen` ni `vid_gen` devuelven la semilla usada.** El resultado es
`{output_format, images:[{index, b64_json}]}` — sin `seed`. Y `seed` es un entero único
(`-1` = aleatorio); **no existe rango ni incremento** en la API.

`batch_count` existe (`limits.max_batch_count`), pero un lote de N imágenes **no reporta qué semilla
produjo cada una**, solo `index`. Por lo tanto:

> **LocalMind debe ser el dueño del libro de semillas.** Cualquier iteración que el dueño quiera
> poder reproducir o retomar más tarde —"esa me gusta, cambiarle el texto y volver a esa"— exige
> que LocalMind envíe las semillas explícitamente y las persista junto al resultado. Depender de
> `batch_count` o de `seed: -1` produce resultados **irreproducibles**.

Esto es un argumento, no un detalle: es la diferencia entre un panel de demostraciones y un taller.

### 5.2 Lo que el motor ya da (no hay que inventarlo)

- **Jobs asíncronos + cola + `queue_position`** → se pueden encolar varias Variations y ver el
  progreso individual, en vez de un botón bloqueado.
- **Cancelación** → indispensable con 8–15 min por clip.
- **`GET /sdcpp/v1/capabilities`** → samplers, schedulers, LoRAs disponibles, upscalers,
  `output_formats_by_mode`, `features_by_mode`, y `limits` (min/max width/height, max_batch_count,
  max_queue_size, max_upscale_width/height). La UI puede **descubrir** qué es válido en vez de
  codificar números a ciegas.
- **`POST /sdcpp/v1/upscale`** → ESRGAN **sin modelo de difusión**, sin generación, con `repeats`
  1–4 y `tile_size`. `capabilities.upscale` indica si hay un modelo RGB ESRGAN compatible
  (requiere uno en `--hires-upscalers-dir`; los upscalers latentes **no** valen aquí).
- **`hires.*` dentro de la generación** → highres fix latente (`enabled`, `upscaler`, `scale`,
  `target_width/height`, `steps`, `denoising_strength`, `custom_sigmas`, `upscale_tile_size`).
- **`--vae-tiling` / `vae_tiling_params`** (incl. `temporal_tiling`) → baja la memoria de
  encode/decode a cambio de tiempo y algo de calidad.
- **WebM directo** → el video llega como `video/webm` reproducible en WebView2 sin instalar códecs.
- **Edición de imagen**: `POST /v1/images/edits`, `-r` (refs múltiples), `init_image`, `mask_image`,
  `control_image`, `ip_adapter_*`; y para H3, `end_image` + `control_frames`.

### 5.3 Bucle de iteración de imagen — superficie de UI que esto implica

1. **Barrido de semillas (seed sweep)**: N imágenes con semillas explícitas y consecutivas.
   Limitado por `limits.max_batch_count` y por el tiempo. Requiere el libro de semillas (§5.1).
2. **Variación / metamorfosis**: mismo prompt y misma semilla, un parámetro cambiado (prompt,
   `cfg`, `steps`, sampler, LoRA, `strength`); o `init_image` + `strength` para img2img. La UI tiene
   que poder **volver a un resultado anterior y edit sus parámetros**, no solo generar de cero.
3. **Upscale**: dos caminos distintos y la UI no debe confundirlos — (a) *hires fix* latente dentro
   de la generación (`hires.*`), que regenera; (b) `POST /sdcpp/v1/upscale`, que **no** regenera y
   solo requiere un ESRGAN RGB cargado. El segundo es barato y no toca el DiT.
4. **Comparación de prompts**: ver A/B lado a lado con los mismos parámetros y **la misma semilla**,
   para que la única variable sea el texto.
5. **Cambio de checkpoint**: reinicio de proceso (§1.3). La UI debe decirlo explícitamente — es la
   operación más lenta y la más fácil de lanzar por error.
6. **Borrador de prompt con negative prompt**, sampler, scheduler, steps, cfg, `clip_skip`,
   resolución, seed — y **recordar el borrador junto a cada resultado**, para poder reproducir.

### 5.4 Bucle de iteración de vídeo — una categoría distinta

El coste por render es de minutos, así que iterar sobre el render final es una mala estrategia. El
bucle tiene que **escalonarse**:

- **Previsualización barata**: baja resolución (~0.4 MP), pocos pasos, `video_frames` en el mínimo
  de la rejilla `17k+5` (5 frames), sin audio VAE. Sirve para iterar **prompt, composición y
  movimiento**, no calidad. Aquí es donde se gasta el tiempo de iteración.
- **Escalonado**: subir resolución y/o frames solo cuando el prompt está aprobado. Guardar las
  semillas ganadoras de la previsualización y **reusarlas** en el render final (de nuevo §5.1:
  quien guarda la semilla es LocalMind).
- **Fijar el target de producción**: decidir pronto si el objetivo es Qwen-Image-2.1 / Z-Image-Turbo
  (imagen, segundos) o H3 (vídeo, minutos). Son dos productos con perfiles de interacción distintos.
- **Cancelar es una función, no un extra**: con 15 min por clip, la cancelación es lo que hace
  aceptable el bucle.
- **Nota de alcance de audio**: H3 emite **audio estéreo 32 kHz nativo en la misma pasada**. Es un
  diferencial enorme frente a la competencia y hay que exponerlo (silenciar la pista, guardar
  solo video, etc.). También es la razón del `--audio-vae` fp32 obligatorio y de su coste de RAM.

### 5.5 Lo que debería quedar **fuera de alcance** (propuesta, no decisión)

- Cualquier sistema de nodos/ grafos tipo ComfyUI.
- Fine-tuning, LoRA **entrenado** (cargar LoRA sí, entrenarla no).
- Model sharing/red multi-GPU (`--rpc-servers`): una sola GPU.
- Face-swap / inpainting avanzado más allá de lo que ya dan `mask_image` e IP-Adapter.
- Batch masivo desatendido: la RAM es el cuello (§2.1), no la GPU.
- Timeline/editor de vídeo: LocalMind **renderiza**, no **monta**.

---

## 6. Preguntas abiertas: decisiones que son del dueño

Ninguna de estas se resuelve en esta fase. Están ordenadas por impacto.

1. **¿Cuál es el objetivo primario, imagen o vídeo?** Define el producto. Imagen (Qwen-Image-2.1 /
   Z-Image-Turbo) es usable en segundos y permite iterar de verdad; H3 es de 8–15 min por clip y en
   esta máquina depende de que el auto-fit resuelva (ver §2.1). Son dos apps distintas, no dos
   pestañas de la misma.
2. **Si H3 entra, ¿en qué modo?** T2AV necesita el DiT de 10.64 GiB; Ref2AV admite el Q2_K de
   6.26 GiB pero **exige imagen de referencia** y no genera desde texto. La segunda es la primera
   meta alcanzable. ¿Se acepta "H3 = imagen de referencia → clip"?
3. **¿Cómo conviven el LLM y la difusión en 16 GB de VRAM?** Hoy LocalMind mantiene ~6 GB con el
   LLM residente. Qwen-Image-2.1 (~13 GB pico) y H3 (~11 GB pico) no conviven con él. ¿Política de
   *eviction* conmutada (parar LLM → generar → volver a arrancar el LLM, con su coste de recarga) o
   reserva dedicada? Esto toca el panel y la UX del chat, no es un detalle interno.
4. **¿Se proxy-ea `sd-server` y no se expone su puerto, o se acepta un puerto sin autenticar en
   127.0.0.1?** (§1.4) LocalMind ya tiene una postura de seguridad (D-7) y `sd-server` no la
   respeta. Hay que decidir explícitamente.
5. **¿Se vendoriza el zip de sd.cpp en `bin/`, se descarga en primera ejecución, o se deja elegir
   el build (Vulkan / ROCm)?** La regla actual del repo es vendorizar prebuilt. C: tiene 54 GB
   libres y E: tiene 412 GB — pero H3 necesita ~28 GiB solo en pesos, así que la ubicación de
   `models/` y de la caché cuenta.
6. **¿Qwen-Image-2.1 entra pese a su licencia no comercial?** Es el #1 en Elo y el mejor en
   tipografía, pero su licencia **no permite uso comercial**. Z-Image-Turbo y FLUX.2 klein son
   Apache 2.0. Si LocalMind es solo de uso personal esto es una decisión del dueño, no técnica.
   (Para H3: la restricción es solo UE, Reino Unido, Corea del Sur y EE. UU. — Argentina queda
   fuera; pero la cláusula V.4 prohíbe *mostrar* las salidas fuera del territorio, así que publicar
   un clip es una zona gris contractual. Generar en privado es inequívoco.)
7. **¿Límite del catálogo: qué familias se soporta de primera?** Cubrir todos los modelos de la
   lista de sd.cpp (40+ familias) no es alcanzable. Sugerencia para decidir: Z-Image-Turbo +
   Qwen-Image-2.1 y, si entra, H3.
8. **¿Se expone la generación por la API pública de LocalMind (`/v1/*`)?** Hoy `/v1/*` es un proxy
   OpenAI *publicado* que consumen CLIs externos. Añadir `/v1/images/generations` publicaría
   difusión a cualquier CLI con la clave del gateway. Es una superficie de ataque nueva.

---

## 7. Registro de riesgos

| # | Riesgo | Impacto | Señal temprana | Mitigación posible |
|---|---|---|---|---|
| R1 | **H3 no cabe en 15.1 GB de RAM** para T2AV (§2.1). El auto-fit puede no resolverlo. | Alto — el pilar de vídeo no arranca | `sd-server` falla al cargar el DiT, o el sistema hace swap y se clava | Empezar por Ref2AV Q2_K (6.26 GiB); o fijar un presupuesto explícito con `--max-vram` negativo y medir |
| R2 | **Caché de prefijo de Qwen-Image-2.1 cae a FP32 sin FA** → ~8 GiB de RAM (§2.2) | Alto — OOM en el equipo con menos RAM | Fallo de asignación al generar | `--model-args qwen_image_2_1_prefix_cache_type=q8_0`, o desactivar la caché |
| R3 | **Conflicto de VRAM LLM↔difusión** (§6.3) | Alto — rompe la función existente de LocalMind | El chat deja de responder cuando empieza un render | Política de eviction explícita, no implícita |
| R4 | **`sd-server` sin autenticación** en loopback (§1.4) | Medio-alto — exposure local no intencionada | — | Proxy LocalMind; no exponer el puerto crudo |
| R5 | **Recargar pesos cuesta segundos** y cambiar de modelo = reiniciar el proceso | Medio — fricción en el bucle de iteración | — | Auto-fit; `params-backend te=disk`; caché del sistema de ficheros |
| R6 | **Render de vídeo de 8–15 min sin progreso visible** → parece colgado | Medio — el dueño cancela antes de ver nada | — | `queue_position` + `started`/`completed` del job; cancelar; y previsualización escalonada (§5.4) |
| R7 | **Catálogo de modelos insuficiente** (§4): `.safetensors` filtrado, `mmproj` excluido, un `filename` por modelo | Medio-alto — bloquea la corrección en la UI | — | Taxonomía de componentes + presets |
| R8 | **Comandos copiados de la documentación upstream fallan en Vulkan** (`--diffusion-fa`, `int8_convrot`) (§2.2, §2.4) | Medio — pérdida de tiempo y cuelgues del driver | — | Los presets de LocalMind deben filtrar flags no soportados en el backend activo |
| R9 | **Cambio estructural en `ui.html`** sin pasar `ui-tabs.mjs`: el precedente documentado es un `<div>` sin cerrar que **entró con las 55 comprobaciones stub en verde** | Alto — P0 en producción | — | `ui-tabs.mjs` es obligatorio tras cualquier cambio estructural (ya está en `conditional_gates`) |
| R10 | **Cambio en el contrato HTTP `/api/*`** sin avisar: CLIs externos lo consumen | Medio | — | Lo exige `openspec/config.yaml` §rules.specs |
| R11 | **Sin TDD** (`strict_tdd: false` deliberado) + superficie nueva grande y asíncrona | Medio | — | Las offline gates no cubren nada de este dominio; las funciones puras (construcción de argv, parsing de `capabilities`, libro de semillas) sí son testeables unitariamente, como ya se hace con `build_engine_cmd` |
| R12 | **C: con 54 GB libres** y H3 pidiendo ~28 GiB de pesos | Bajo-medio | — | Modelos en E:, o limpieza |
| R13 | **`int8_convrot` cuelga la GPU** si algún preset lo incluye por error | Alto (requiere reinicio) | — | Presets solo GGUF/safetensors compatibles con Vulkan; nunca convrot |

---

## 8. Áreas afectadas (si el cambio avanza)

- `src-rust/src/process.rs` — `build_engine_cmd` es la fuente única del argv; hay un precedente
  exacto para un segundo motor (`ProcessManager`, `ServerStatus`, `find_free_port`,Stop, logs,
  presión). El candado `psu_unsafe_flag` es el precedente de "validar flags antes del `spawn()`".
- `src-rust/src/server.rs` — proxy y rutas; `handle_request` (~línea 442) es el despachador.
  `is_public_path` y el chequeo de auth de la línea 532 son donde se decide R4.
- `src-rust/src/models.rs` — `list_gguf_files` (737), `dest_for` (402), `enrich_model_entry` (832).
- `src-rust/src/config.rs` — perfil/puerto/flags de un segundo motor; `psu_unsafe_flag` y el
  precedente de flags)).
- `src-rust/src/agents.rs` / `launcher.rs` — sólo si los agentes deben poder invocar generación.
- `ui.html` — 7 pestañas (`switchTab`, `ui.html:2736`); una pestaña de generación es
  **cambio estructural** → R9.
- `bin/` — el zip de sd.cpp. `openspec/config.yaml` prohíbe regenerar `bin/`; hay que **añadir**,
  no reconstruir.
- `tests/` — `ui-render.mjs` (stub) y `ui-tabs.mjs` (DOM real). Los tests de Rust son
  `#[cfg(test)]` en el propio módulo, en español.

---

## 9. Listo para propuesta

**Sí, con una salvedad de encuadre.** La exploración貧 closed la pregunta de arquitectura con
evidencia verificada: el motor persistente ya existe, ya viene en el zip que LocalMind ya vendoriza,
y expone un contrato asíncrono con cancelación que encaja con la forma "proceso + API" del proyecto.
Las formas (a) y (c) quedan descartadas por razones concretas, y (d) por licencia y shape.

**Lo que `propose` debe llevar y esta fase NO decide**: el objetivo primario (imagen o vídeo), la
política de convivencia LLM↔difusión en 16 GB, la postura de seguridad sobre el puerto de
`sd-server`, y si Qwen-Image-2.1 entra pese a su licencia no comercial. Las cuatro son del dueño y
todas cambian el alcance de forma material.

**Recomendación de encuadre para `propose`**: atacar esto como **dos incrementos con puertas
distintas**, no como una sola entrega. El primero —imagen, con Z-Image-Turbo (Apache 2.0) y/o
Qwen-Image-2.1, sobre `sd-server` + proxy + presets + UI de iteración con libro de semillas— es
donde se puede de verdad cumplir "muy bien implementada", porque el bucle de segundos permite
iterar. El segundo —vídeo/H3— es un problema de memoria y de tiempo, y conviene tratarlo como
**exploración de factibilidad medible** (cargar, medir, reportar) antes que comprometer una UX.

---

*Fase explore. No se modificó código del producto. Artefacto en
`openspec/changes/diffusion-generation/exploration.md`; misma clave en Engram
(`sdd/diffusion-generation/explore`).*
