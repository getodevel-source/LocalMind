# Propuesta: `diffusion-generation`

Fase: **propose**. Fecha: 2026-09-30. Cambio: `diffusion-generation`.
Artefacto: `proposal.md`. Store: híbrido (OpenSpec + Engram, clave `sdd/diffusion-generation/proposal`).
Entrada: `exploration.md` (fase `explore`, id Engram 1204).

> Esta fase propone **alcance y enfoque**. No diseña, no especifica requisitos, no lista tareas.
> Los identificadores, rutas de API y flags van en inglés porque son del runtime.

---

## Reducción de alcance aprobada por el dueño (2026-09-30)

**Esta revisión recorta; no rediseña.** El dueño revisó el plan, lo juzgó sobre-construido, y
aprobó cuatro cortes más la retirada de `Z-Image-Turbo` del catálogo. Se aplican en `design.md`, en
las 5 specs y en `tasks.md`. Lo que **no** se toca, porque es el alcance aprobado y la razón de que
el plan sea defendible:

| Se conserva | Por qué |
|---|---|
| La **sonda de VRAM** (`tests/diffusion-vram-probe.mjs`) y el **spike medido de H3** con su informe | No son ceremonia de test: son los instrumentos que producen los dos números que nadie tiene. Un informe negativo de H3 es un entregable válido. |
| El **`EngineCoordinator`** y la lease de VRAM | Dos motores en una tarjeta de 16 GB. Es específico de LocalMind e inevitable. |
| La **postura de seguridad** del proxy: denylist, CORS loopback-only, puerto crudo no publicado | `sd-server` no tiene autenticación y refleja cualquier `Origin`. |
| **`bin/sd-cpp/`** con `argv` de rutas absolutas | sd.cpp vendoriza su propio `leejet/ggml`; su `ggml-vulkan.dll` es ABI-incompatible con la de llama.cpp que vive en `bin/`. No pueden compartir directorio. |

Los cuatro cortes, y lo que cuesta cada uno:

1. **Ledger plano**: un registro por generación, sin nivel `series`. Se conservan intactos la
   escritura de la semilla **antes** del `POST` (el motor nunca la devuelve), la rehidratación y la
   reproducibilidad por parámetros registrados.
2. **Sin comparación A/B** en el bucle de iteración. Dos prompts se comparan generando dos
   generaciones con la misma semilla y mirándolas en el historial.
3. **Sin autodetección de roles ni filtro de flags a dos severidades**: cada preset **declara** sus
   rutas en el TOML, y hay **una** lista de flags denegadas con **un** comportamiento, aplicada antes
   del `spawn()`. Se conservan las dos correcciones que no son preferencias: **ningún `int8*` puede
   existir** (cuelga la GPU en RDNA2) y **`--diffusion-fa` no llega al motor** en Vulkan.
4. **Sin test de contrato HTTP genérico**: `tests/diffusion.mjs` lo ejercita contra el motor real.

**Consecuencias que se asumen, dichas con claridad:**

- **C1 queda concentrado.** Sin un Tier-1 permisivo, toda la generación de imagen depende de la
  licencia no comercial de `Qwen-Image-2.1`. Se acepta, y la mitigación pasa a ser la visibilidad
  de la licencia antes de generar.
- **La coexistencia se vuelve más difícil.** Z-Image-Turbo era el preset **pequeño** (~9 GB);
  Qwen-Image-2.1 es el **grande** (~13 GB). Retirarlo endurece la aritmética de la reserva de VRAM
  de "mala" a "imposible", y hace **improbable** la promoción a `Shared`. La **alternancia** —no la
  convivencia— es lo que este cambio entrega.
- **Un `family` declarado puede mentir.** Al no leer metadatos, LocalMind comprueba que el `family`
  declarado por un componente coincida con el de su preset, pero no que sea el real. Declarar la
  familia correcta pasa a ser contrato del usuario.

---

## Intención

LocalMind hoy es un host de **LLM local**: un binario vendorizado (`llama-server`) residentemente
arrancado, un proxy HTTP y una UI de chat. El dueño quiere que la **generación de imagen** sea una
capacidad de primera clase con la misma calidad de producto, y que la **iteración** sobre esas
imágenes esté *muy bien implementada* — esa es la necesidad declarada, no una consecuencia.

Por qué ahora, y no antes:

1. **El bloqueo técnico que existía ya no existe.** La exploración partió de la premisa de que no
   había servidor de difusión y había que escribir un shim. Falso: `leejet/stable-diffusion.cpp`
   shippea `sd-server.exe` (1.15 MB) **en el mismo zip prebuilt Windows x64 Vulkan** que LocalMind
   ya vendoriza para llama.cpp. La integración pasa a ser *un binario vendorizado más* — el patrón
   que `openspec/config.yaml` ya declara.
2. **El contrato encaja 1:1 con lo que LocalMind ya hace.** Jobs asíncronos, cola con
   `queue_position`, y cancelación. La cancelación es la que convierte "8–15 min por clip" en algo
   usable, y solo existe porque hay proceso persistente.
3. **La arquitectura de LocalMind es exactamente la que este motor necesita.** "Proceso de larga
   duración + API HTTP + proxy" no es un patrón que haya que inventar para difusión: ya es la
   identidad del proyecto.

Qué problema concreto resuelve: hoy no hay **ninguna** generación de imagen. No es una mejora
incremental, es una pestaña nueva. Y el diferenciador no es "generar una imagen" (eso lo hace
cualquier web), es **iterar sobre ella**: volver a un resultado, cambiar un parámetro, reusar la
semilla, comparar dos prompts con la misma semilla.

### El problema que casi no se ve: la semilla

`POST /sdcpp/v1/img_gen` **no devuelve la semilla usada**. `seed` es un entero único (`-1` =
aleatorio), no hay rango ni incremento, y `batch_count` solo informa `index`, no qué semilla
produjo cada imagen de un lote.

Consecuencia directa e inevitable: **depender de `seed: -1` produce resultados irreproducibles.**
"Esa me gusta, cambiarle el texto y volver a esa" es imposible si nadie guardó la semilla.
Por eso LocalMind debe ser el **dueño del libro de semillas**: envía semillas explícitas,
persiste la semilla junto al borrador completo del resultado, y rehidrata al reabrirlo.

No es una mejora de la UX. Es la condición que hace que exista la UX.

---

## Alcance

El cambio tiene **dos niveles con niveles de compromiso deliberadamente distintos**. No es una
sola entrega: es un incremento con puerta y un spike con informe.

### Nivel 1 — Imágenes: compromiso de producto completo

**Dentro de alcance:**

- **Vendorizar y pilotar `sd-server`** como motor de difusión de larga duración, con su propio
  puerto, su propio estado de salud y su ciclo de vida (arranque/parada), siguiendo el patrón ya
  establecido para `llama-server`.
- **Proxy LocalMind hacia `sd-server`** sobre las rutas nativas `/sdcpp/v1/*`, exponiendo solo la
  superficie autenticada de LocalMind.
- **Un concepto de catálogo para difusión** distinto del de LLM: un *preset* es un conjunto
  coherente de componentes tipados (DiT, VAE, VAE de audio, encoder de texto, encoders CLIP,
  tokenizador…), no un `filename`. Descubrimiento de lo que hay en disco, con tipos.
- **Libro de semillas** persistido por LocalMind, con el borrador completo de parámetros de cada
  resultado, de modo que cualquier imagen generada sea reproducible y reanudable.
- **Bucle de iteración de imagen bien construido** en `ui.html`: barrido de semillas, variación /
  metamorfosis, upscale (los dos caminos, sin confundirlos), y memoria del borrador.
- **Presets de modelo de primera**: **Qwen-Image-2.1** (único preset de Nivel 1, y por tanto el
  default) con su configuración de memoria correcta para el backend activo.
  *(Revisión 2, 2026-09-30: `Z-Image-Turbo` se retiró del alcance. Ver §Reducción de alcance.)*
- **Informe de factibilidad en vídeo** (Nivel 2) como entregable de primera clase.

**Fuera de alcance del Nivel 1** (explícito):

- Cualquier sistema de nodos o grafos tipo ComfyUI.
- Fine-tuning y **entrenar** LoRAs (cargar LoRAs sí; entrenarlas no).
- Model sharing / multi-GPU vía `--rpc-servers`: una sola GPU.
- Face-swap, inpainting avanzado, más allá de lo que ya dan `mask_image` e IP-Adapter.
- Batch masivo desatendido.
- Publicar generación en la API pública `/v1/*` de LocalMind. Hoy `/v1/*` es un proxy OpenAI
  *publicado* que consumen CLIs externos; añadir `/v1/images/generations` expondría generación a
  cualquier CLI con la clave del gateway. Fuera de alcance en este cambio (queda como decisión
  abierta posterior).
- Cualquier UX de iteración de vídeo.

### Nivel 2 — Vídeo: **spike medido**. NO es un compromiso de UX

- **Se cablea el motor** lo suficiente para cargar MiniMax-H3 y lanzar un trabajo real.
- **Se mide empíricamente** si H3 entra en 15.1 GB de RAM, y a qué resolución, cuántos frames y
  cuántos pasos. Se mide el **tiempo real de pared**, no una estimación.
- **Se entrega un informe con los números.**
- **No se promete ninguna experiencia de iteración de vídeo en este cambio.**

Por qué así: la estimación vigente de "8–15 min por clip" se calculó sobre el **GGUF equivocado**
(el único fichero de 6.26 GiB es `ref2va_pruned-Q2_K`, que exige imagen de referencia y **no**
hace texto-a-video). El único T2AV razonable es `fl2va_pruned-Q4_K_M` de 10.64 GiB, lo que lleva la
suma realista a ~28.25 GiB en disco. Esa cifra es una extrapolación, no un hecho. El entregable de
este nivel es **sustituir la extrapolación por un hecho medido**, para que el dueño decida después
qué construir. El dueño eligió esto explícitamente antes que comprometer ahora una UX Ref2AV o T2AV.

**Fuera de alcance del Nivel 2** (explícito): previsualización escalonada, audio, `end_image`,
`control_frames`,_TIMELINE/editor de vídeo. LocalMind **renderiza**, no **monta**.

### Fuera de alcance transversal

- **Upscaling por encima del techo de VRAM.** El techo es ~1 megapixel: por encima, la atención
  MATH SDPA es O(n²) y revienta. Cualquier ampliación del límite es otro cambio.
- **Rutas int8.** `int8_convrot` **cuelga la GPU en RDNA2** (necesita WMMA/MFMA). Ningún preset
  puede incluirlo. El flag de búsqueda upstream es `int8_convrot` (sin guion bajo) y la doc lo
  escribe de dos formas; el selector de LocalMind filtra por capacidad del backend, no por nombre.
- **ComfyUI, ROCm, backends alternativos.** Vulkan es el único backend viable aquí.
- **Modelos de edición** tipo Qwen-Image-Edit como familia propia.
- **Reconstruir o regenerar `bin/`.** Se **añade**, nunca se recompila.

---

## Capacidades

> Contrato entre `propose` y `spec`. `openspec/specs/` está **vacío** en este repositorio: no existe
> ninguna capacidad publicada todavía.

### Capacidades nuevas

Todas son nuevas. Cada una recibe su spec completo en
`openspec/changes/diffusion-generation/specs/<nombre>/spec.md` en la fase `spec`.

- **`diffusion-engine-lifecycle`** — `sd-server` como motor vendorizado de larga duración:
  elección y colocación del build prebuilt, construcción del argv de arranque (con filtrado de
  flags no soportados por el backend activo), asignación de puerto, sonda de disponibilidad,
  estado de salud, arranque/parada/reinicio, y lectura de logs y presión. Incluye el hecho de que
  **no existe endpoint de recarga**: cambiar de preset exige reiniciar el proceso, y eso es
  contrato, no efecto secundario.

- **`diffusion-model-catalog`** — un modelo de difusión como **preset**: conjunto ordenado de
  componentes tipados con sus rutas (DiT, VAE, VAE de audio, encoder de texto, CLIP L/G/V, T5/XXL,
  tokenizador, ControlNet, IP-Adapter, módulos de movimiento), con directorios escaneados para
  LoRAs, upscalers y embeddings. Descubrimiento de lo que hay en disco con tipos y tamaños, y
  descubrimiento de lo que el proceso cargado admite vía `/sdcpp/v1/capabilities`. Debe convivir
  con el catálogo LLM existente **sin alterarlo**: `GET /api/models` no cambia de forma.

- **`generation-seed-ledger`** — LocalMind es el dueño de la semilla. Asignación explícita de
  semillas por LocalMind (nunca `-1` para trabajo que deba ser reproducible), persistencia de la
  semilla junto al **borrador completo** que la produjo (prompt, negative prompt, sampler,
  scheduler, steps, cfg, clip_skip, resolución, LoRAs, hires), y rehidratación del borrador para
  continuar una iteración más tarde. Cubre también el caso del lote: qué semilla produjo qué
  imagen, que la API no reporta.

- **`image-generation-iteration`** — la UX de iteración sobre imágenes, en la UI y en el proxy:
  barrido de semillas con progreso por imagen, variación/metamorfosis (mismo prompt + misma
  semilla, un parámetro cambiado, o `init_image` + `strength`), los dos caminos de upscale
  distinguidos (hires fix
  latente dentro de la generación, que regenera, frente a `POST /sdcpp/v1/upscale`, que no
  regenera y no toca el DiT), encolado y cancelación de trabajos, y advertencia explícita de que
  cambiar de preset reinicia el proceso.

- **`video-generation-feasibility`** — spike **medido**, sin compromiso de UX. Carga de
  MiniMax-H3 con presupuesto de memoria explícito, ejecución de trabajos **reales** (no
  simulados) en varias combinaciones de resolución / frames / pasos, medición del tiempo real de
  pared y del pico de RAM en cada una, y **publicación de un informe con los números medidos**,
  incluido el caso de fallo cuando el que hay es que no carga.

### Capacidades modificadas

**Ninguna.** No existe ninguna capacidad publicada (`openspec/specs/` vacío), así que no hay
requisitos previos que cambien. Lo que sí hay es un **contrato HTTP publicado** que este cambio
extiende: añadir rutas bajo `/api/*` es superficie nueva, y alterar la forma de las existentes
sería un riesgo de ruptura (regla `config.yaml` §rules.specs). Eso se trata en Riesgos, no como
capacidad modificada.

---

## Enfoque

### Superficie del cambio (regla `config.yaml` §rules.proposal)

- **Backend Rust** (`src-rust/src/`): sí, es la parte mayor.
- **UI** (`ui.html`): sí, y es **cambio estructural**, no solo de lógica. Una pestaña nueva en el
  `switchTab` existente es exactamente el caso que el stub DOM no puede validar.
  → **`tests/ui-tabs.mjs` es puerta obligatoria**, no condicional. Ver R9 abajo.
- **Configuración de motor** (`bin/`, `config.rs`): sí, y es **adición de binario vendorizado**.

### 1. Cómo se vendoriza `sd-server`

Se **añade** el contenido del zip prebuilt `sd-master-3f8527a-bin-win-vulkan-x64.zip`
(`sd-server.exe`, `stable-diffusion.dll`, las DLL de WebM/WebP, y las que el binario resuelva) al
directorio `bin/`, exactamente igual que ya se hizo con los binarios de llama.cpp. **No se compila
nada desde fuente** — regla dura de `config.yaml`.

> **Descubrimiento que hay que resolver en `design`:** `bin/` ya contiene un `ggml-vulkan.dll` del
> build de llama.cpp, y el zip de sd.cpp **trae su propio `ggml-vulkan.dll`**, que es otra build de
> la misma biblioteca. Los dos no pueden convivir en el mismo directorio plano sin colisión de
> nombre. Esto condiciona la colocación (subdirectorio dedicado y resolución de DLL por proceso,
> frente a aplanado) y por tanto el argv y el `cwd` del `spawn()`.

Los binarios de llama.cpp que ya existen **no se tocan**. Si algo falla, la reversión es borrar lo
añadido.

### 2. Cómo se lanza y se descubre

`process.rs` ya tiene el precedente exacto para un segundo motor, y lo reutiliza sin inventar:

| Precedente LLM ya existente | Extensión a difusión |
|---|---|
| `build_engine_cmd` (fuente única del argv) | `sd-server` + flags de contexto de difusión |
| `find_free_port` (preferido, hasta +50 intentos) | puerto de `sd-server` (default upstream `1234`) |
| `ServerStatus { status, model, port, is_healthy }` | estado hermano de difusión |
| `psu_unsafe_flag` | precedente de **validar flags antes del `spawn()`** |
| `start_banner(context, threads, ubatch, port)` | banner de difusión con el plan de memoria |
| logs, historial de fallos, presión | los mismos, por motor |

La sonda de disponibilidad usa `GET /sdcpp/v1/capabilities`: no es un health-check genérico, es el
mismo endpoint que aporta samplers, schedulers, LoRAs, upscalers, límites y valores por defecto por
modo. **Discovery no significa codificar números a ciegas**: la UI debe descubrir qué es válido.

El `argv` de cada preset pasa por un **filtro de flags por backend**. Los ejemplos de la
documentación upstream (`docs/minimax_h3.md`, `docs/qwen_image_2.1.md`) usan `--diffusion-fa` /
`--fa`, y **Vulkan no soporta Flash Attention** (solo cpu, cuda/rocm, metal). Copiar esos comandos
produce un arranque que falla o un motor que corre sin la aceleración esperada. El filtro es la
defensa.

### 3. Cómo se maneja la memoria — dos decisiones que son configuración, no accidents

- **Qwen-Image-2.1 sin FA hace que `qwen_image_2_1_prefix_cache_type=auto` caiga a FP32**:
  ~4 GiB por condición × 2 condiciones ≈ **8 GiB de RAM solo en caché de prefijo**, en un equipo
  con 15.1 GB. La mitigación documentada es `q8_0` (~1.06 GiB). **El preset de Qwen-Image-2.1 lleva
  `q8_0` por defecto.** Esto no se descubre en el primer render.
- **`--auto-fit`** (activo por defecto si no se pasa `--params-backend`) reparte los parámetros
  entre dispositivos según memoria libre, y `--max-vram` admite valores negativos. Es lo que hace
  tratable el problema. La propia documentación upstream es honesta: offload no garantiza que toda
  resolución o frame count quepa, y auto-fit **no** mueve un componente a CPU solo porque sus pesos
  completos excedan el VRAM. Por eso el Nivel 2 es una medición y no una promesa.

### 4. Cómo se conduce

LocalMind proxea `/sdcpp/v1/*` hacia `sd-server` y **no expone el puerto crudo**. La
implementación concreta de esa frontera es de `design` (§"Quedado para `design`"), pero la
**postura sí se decide aquí**: el puerto de `sd-server` **no** se publica fuera del proceso de
LocalMind.

El trabajo asíncrono se conduce con lo que el motor ya da: `POST` devuelve `id`, `GET
/sdcpp/v1/jobs/{id}` da `status`/`queue_position`/`result`, `POST /sdcpp/v1/jobs/{id}/cancel`
cancela. No hay que inventar ni un modelo de trabajos ni una cola en LocalMind.

### 5. Cómo se distingue un modelo de difusión de un modelo LLM

Hoy el catálogo asume *un modelo = un fichero = un motor que lo sirve*: `list_gguf_files`
(`models.rs:737`) recorre `models/` raíz + un nivel de subdirectorio, filtra `.gguf` y **excluye
cualquier nombre que contenga `mmproj`**; `dest_for` (`models.rs:402`) coloca un `.gguf` plano en
`models/`.

Para difusión eso es falso por partida doble:

1. **Un preset es un conjunto de componentes tipados**, cada uno con su flag. No hay un `filename`.
2. **El filtro actual esconde componentes obligatorios.** Para Qwen-Image-2.1 la edición requiere
   `mmproj-Qwen3VL-8B-Instruct-F16.gguf`, que hoy sería invisible; y los VAE son a menudo
   `.safetensors`, que el filtro `.gguf` descarta entero. Un componente que falta aquí produce
   **basura silenciosa, no un error** — el peor modo de fallo posible.

De ahí sale el concepto: **un `diffusion_preset` es un bundle tipado y validado**, no una entrada
de la lista de LLM. El escaneo de difusión es un escaneo distinto, con su propio conjunto de tipos
y extensiones, y **no hereda la exclusión por `mmproj`**. Los componentes se organizan por familia
de modelo, no por repositorio.

Regla dura de compatibilidad: **`GET /api/models` y `POST /api/start` mantienen su forma actual
para LLM.** La superficie nueva se añade como campos o rutas nuevas. Cambiar la forma de un
contrato publicado que consumen CLIs externos es una ruptura, y `config.yaml` lo prohíbe sin
llamada explícita.

### 6. El libro de semillas

Es la pieza que convierte un panel de demostraciones en un taller, así que el enfoque es explícito
aunque el modelo de datos sea de `design`:

- LocalMind **asigna y envía** la semilla. Nunca `-1` en nada que deba reproducirse.
- Cada resultado se persiste **con su semilla y su borrador completo**, no solo con la imagen.
- Al reabrir un resultado, el borrador se rehidrata editable: cambiar un parámetro y regenerar
  parte de la variación.
- En un lote, la correspondencia imagen↔semilla la conoce LocalMind, porque la API solo devuelve
  `index`.
- Las semillas ganadoras de una previsualización se **reusan** en el render de mayor calidad, que
  es el mismo principio aplicado al Nivel 2.

### 7. Cómo se resuelve la contención con el LLM residente

Restricted a lo que esta fase puede afirmar: LocalMind pasa de **un** motor a **dos motores
compitiendo por el mismo VRAM**. Hoy el LLM residente ocupa ~6 GB; los picos de Qwen-Image-2.1
(~13 GB) y de H3 (~11 GB) no conviven con eso.

Lo que queda **establecido** aquí:

- Los dos motores pasan a tener un **coordinador único en LocalMind**, porque la serialización tiene
  que ser una decisión consciente y no el resultado de dos `spawn()` independientes que compiten.
- El conflicto **debe ser visible** en la UI antes de que ocurra. Con el reinicio de proceso como
  única vía para cambiar de preset, esa es la operación más cara y la más fácil de lanzar por
  error.
- El reinicio del LLM tiene un **coste real de recarga de segundos**. Ignorarlo produce una
  experiencia que parece un cuelgue.

Lo que **queda para `design`** y no se decide aquí: la política concreta (eviction conmutada
parar→generar→arrancar, reserva dedicada, o conmutación manual), y cómo se expresa en el panel y
en la UX del chat, que es donde se nota.

---

## Superficies afectadas

| Ruta | Cambio | Qué cambia |
|---|---|---|
| `bin/` (subdirectorio a decidir) | **Añadir** | Contenido del zip prebuilt Vulkan de sd.cpp. Binarios de llama.cpp **intactos**. Colisión de `ggml-vulkan.dll` a resolver en `design`. |
| `src-rust/src/process.rs` | Modificar | `build_engine_cmd` es la fuente única del argv: añadir la rama de difusión. Reutilizar `find_free_port`, `ServerStatus`, logs, presión. Aplicar el precedente `psu_unsafe_flag` (validar flags **antes** del `spawn()`). |
| `src-rust/src/server.rs` | Modificar | `handle_request` (~442) como despachador de las rutas nuevas; proxy `/sdcpp/v1/*`; `is_public_path` y el chequeo de auth de **532** (D-7) donde se decide la frontera de seguridad. **No cambia la forma de las rutas existentes.** |
| `src-rust/src/models.rs` | Modificar | `list_gguf_files` (737) y `dest_for` (402) **se quedan como están** para LLM. Añadir el escaneo de difusión tipado, con sus extensiones y **sin** la exclusión `mmproj`. `enrich_model_entry` (832) se reutiliza. |
| `src-rust/src/config.rs` | Modificar | Puerto, flags y perfil del segundo motor, siguiendo el precedente de flags existente. |
| `src-rust/src/agents.rs` / `launcher.rs` | **No** en este cambio | Solo si los agentes deben poder invocar generación. Fuera de alcance. |
| `ui.html` | Modificar | Pestaña de generación junto a las 7 existentes (`switchTab`, ~2736). **Cambio estructural → `ui-tabs.mjs` obligatorio.** La tabla de modelos, con sus cuatro columnas actuales, no alcanza para representar un preset. |
| `tests/ui-render.mjs` | Modificar | Cobertura stub de los controles nuevos. **No** es suficiente para la estructura (R9). |
| `tests/ui-tabs.mjs` | Sin cambio | **Es la puerta**, no el trabajo. Debe correr contra la app viva. |
| `tests/smoke.mjs` | **Sin cambios** *(revisión 2)* | La superficie de difusión la cubre `tests/diffusion.mjs`, que habla con el motor real; un aserto de forma no añade señal. La forma de la superficie LLM publicada la cubre `tests/e2e.mjs`, que consume la API como un cliente externo. |
| `tests/` (nuevo) | Añadir | Arnés de medición del Nivel 2 (reloj de pared, RAM) y tests de las funciones puras: construcción de argv, filtrado de flags por backend, parseo de `capabilities`, libro de semillas. En español, como el resto. |
| `openspec/changes/diffusion-generation/` | Añadir | Artefactos del cambio, incluido el informe de medición del Nivel 2. |

---

## Riesgos y restricciones aceptadas

| # | Riesgo / restricción | Impacto | Mitigación en esta propuesta |
|---|---|---|---|
| **C1** | **Licencia no comercial de Qwen-Image-2.1.** No permite uso comercial. Es el #1 en Elo y el mejor en tipografía, y el dueño **la acepta explícitamente** para uso personal. | **Alto y ahora concentrado**: con `Z-Image-Turbo` retirado, **todo** el Nivel 1 depende de esta única licencia | **Riesgo documentado y aceptado, no pregunta abierta.** Registrado en el preset, en la ficha de la UI **antes de generar** y en el `docs/` del modelo. **La mitigación que existía antes —que el default fuese un modelo permisivo— ya no es posible**: no hay Tier-1 alternativo. Lo que la sustituye es visibilidad, no un default distinto. |
| **C2** | **`sd-server` no tiene autenticación** (no existe `--api-key`), bindea a loopback y su CORS refleja cualquier `Origin` con `Allow-Credentials: true`. Choca con D-7 (`server.rs:532`). | Medio-alto: endpoint sin autenticar en `127.0.0.1` alcanzable por cualquier proceso local | **Restricción aceptada con frontera:** el puerto crudo **no** se publica; todo pasa por el proxy autenticado de LocalMind. La implementación de esa frontera es de `design`. |
| **C3** | **Dos motores, un VRAM.** LLM residente ~6 GB; picos de Qwen-Image-2.1 ~13 GB y H3 ~11 GB. | Alto: rompe una función que hoy funciona | coordinador único y precondición visible. **Política concreta → `design`.** |
| **C4** | **Cambio estructural en `ui.html`.** El precedente documentado es un `<div>` sin cerrar que **entró a producción con las 55 comprobaciones stub en verde**. | **Alto — P0 ya ocurrido una vez** | **`tests/ui-tabs.mjs` es puerta dura, no condicional**, para cualquier cambio estructural. Las 4 gates offline **no** cubren este dominio. |
| **C5** | **No existe ningún número medido de vídeo.** El único dato actual ("8–15 min por clip") es una extrapolación calculada sobre el GGUF equivocado. | Alto: si el Nivel 2 falla, el vídeo queda descartado y hay que re-plantearlo | Declarado como riesgo, no escondido: el entregable del Nivel 2 es **el informe, incluido el caso de fallo**. Un informe que dice "no cabe" también es un éxito. |
| **R2** | Caché de prefijo de Qwen-Image-2.1 cae a FP32 sin FA → ~8 GiB de RAM. | Alto — OOM en el equipo con menos RAM | `q8_0` (~1.06 GiB) **en el preset por defecto**, no como ajuste opcional. |
| **R8/R13** | Comandos copiados de la doc upstream fallan en Vulkan (`--diffusion-fa`); `int8_convrot` **cuelga la GPU** en RDNA2. | Medio-alto: cuelgue del driver que **requiere reinicio** | Filtro de flags por backend antes del `spawn()` (`psu_unsafe_flag` como precedente). Ningún preset incluye rutas int8. |
| **R10** | `/api/*` es contrato publicado que consumen CLIs externos. Añadir rutas es superficie nueva; cambiar formas es ruptura. | Medio | Superficie nueva **aditiva**. `GET /api/models` y `POST /api/start` conservan su forma para LLM, y `tests/e2e.mjs` lo comprueba como cliente externo real. |
| **R5** | Recargar pesos cuesta segundos; cambiar de preset = **reiniciar el proceso** (`new_sd_ctx` se construye una vez, antes de `listen`, y no hay endpoint de recarga). | Medio: fricción constante en la iteración | Es **contrato explícito**, no efecto colateral: la UI lo dice. `auto-fit` y `params-backend …=disk` para aliviar la recarga de pesos. |
| **R11** | Sin TDD (`strict_tdd: false` deliberado) y superficie nueva grande, asíncrona y concurrente con el chat. Las gates offline no cubren nada de este dominio. | Medio | Las funciones **puras** (argv, filtrado de flags, parseo de `capabilities`, libro de semillas) sí se testean unitariamente, con el precedente de `build_engine_cmd`. Nada de TDD ritual. |
| **R12** | Espacio en disco: H3 pide ~28.25 GiB solo en pesos; C: tiene 54 GB libres, E: tiene 412 GB. | Bajo-medio | Los presets apuntan a la unidad con espacio; la instalación de modelos es una acción explícita, no automática. |
| **T1** | **Colisión de `ggml-vulkan.dll`**: `bin/` ya tiene la build de llama.cpp y el zip de sd.cpp trae la suya. | **Bloqueante de la colocación** → `design` | Señalado explícitamente. Afecta argv y `cwd`, así que no se puede decidir en `apply`. |
| **T2** | **VAE no intercambiables** entre familias (Qwen-Image-2.1 vs Qwen-Image vs Wan 2.2). Un emparejamiento erróneo en el preset produce **basura, no error**. | Medio | El catálogo valida el bundle por familia antes de arrancar. Un preset no es una lista de rutas: es un conjunto tipado y validado. |
| **N1** | `exploration.md:520` contiene un carácter corrupto (`exploración貧 closed`) — defecto cosmético del artefacto de `explore`. | Ninguno funcional | Anotado para que `apply`/`archive` lo limpien; no afecta a ninguna decisión. |

---

## Fuera de alcance: explícitamente **NO** en este cambio

Enumerado para que `spec` no lo infiera y `apply` no lo construya:

- **UX de iteración de vídeo.** Sin pestaña de vídeo, sin previsualización, sin escalonado.
- **Upscaling por encima del techo de ~1 megapixel.** La atención MATH SDPA es O(n²) y revienta.
- **Rutas int8 / `int8_convrot`.**
- **ComfyUI, ROCm, backends alternativos a Vulkan.**
- **Modelos de edición** (Qwen-Image-Edit) como familia de primera.
- **img2img más allá del bucle de variación**: `init_image` + `strength` y `mask_image` sí, como
  parte de la iteración; inpainting avanzado, face-swap y workflow de nodos no.
- **Entrenamiento de LoRAs.** Cargar LoRAs sí.
- **Exposición de generación en la API pública `/v1/*`.**
- **Invocación de generación desde agentes** (`agents.rs` / `launcher.rs`).
- **Reconstruir o regenerar `bin/`.**

---

## Quedado explícitamente para la fase `design`

Estas tres están **dentro** del cambio y son decisiones de diseño, no de alcance. No se toman aquí:

1. **La política de convivencia LLM↔difusión** en el VRAM compartido: eviction conmutada con su
   coste de recarga, reserva dedicada, o conmutación manual; y cómo se expresa en el panel y en la
   UX del chat.
2. **La implementación de la postura de seguridad** de `sd-server`: qué se expone, cómo se
   autentica el puerto crudo, y qué se hace con el CORS que refleja cualquier `Origin`.
3. **El modelo de datos del libro de semillas**: esquema, ubicación, granularidad y cómo se
   relaciona con los resultados ya persistidos.

Y una cuarta, que es una **pregunta abierta para el dueño**, no para `design`:

4. **¿Se expone la generación por la API pública `/v1/*`?** Queda fuera de este cambio, pero es
   una decisión que conviene tomar antes de que la superficie crezca.

---

## Plan de reversión

Específico, porque toca arranque/parada de motor, selección de modelo y estructura de la UI.

**Antes de empezar:**

- `bin/` **solo se añade**. Los binarios de llama.cpp no se tocan, no se renombran y no se
  sustituyen. `git status` sobre `bin/` debe mostrar unicamente lo añadido, y un cambio de
  cualquier binario existente es motivo de parada inmediata.
- Ningún modelo se descarga ni se borra automáticamente. `models/` y sus subdirectorios quedan
  intactos: son datos del usuario, no artefactos del cambio.

**Reversión por nivel:**

| Nivel | Reversión | Efecto residual |
|---|---|---|
| **Nivel 1 completo** | Desactivar la generación de difusión desde `config.rs` deja LocalMind exactamente como está: sin motor de difusión, sin pestaña, sin rutas. Los ficheros añadidos en `bin/` y `models/` se pueden borrar sin tocar nada más. | Ninguno en el LLM. Ficheros sin usar en disco. |
| **Incremental** | Cada piece (motor, catálogo, libro de semillas, UI) es retirable por separado: retirar la UI es revertir `ui.html`; retirar el proxy es revertir `server.rs`; retirar el motor es revertir `process.rs`. | Igual que el estado previo de esa piece. |
| **Si algo sale mal a mitad** | Abandonar la rama y volver al último commit. El estado del LLM nunca estuvo en riesgo porque **no se toca su ruta de arranque**. | Ninguno. |

**Punto de no retorno:** ninguno para el LLM. El único estado que este cambio puede dañar es el
del propio motor de difusión, que es nuevo. La reversión nunca requiere reinstalar, recompilar ni
redescargar el motor LLM.

**Verificación tras revertir:** `cargo test --manifest-path src-rust/Cargo.toml` +
`node tests/ui-render.mjs` + `node tests/bench.mjs --self-test` + `node --check` sobre
`tests/*.mjs`. Si hubo cambio estructural en `ui.html`, también `node tests/ui-tabs.mjs` contra la
app viva.

---

## Dependencias

**Binario vendorizado (se añade, no se compila):**

- `sd-master-3f8527a-bin-win-vulkan-x64.zip` (release `master-929-3f8527a`, 2026-09-27):
  `sd-server.exe` (1.15 MB), `stable-diffusion.dll` (35.3 MB), `ggml-vulkan.dll`,
  `webm.dll` / `libwebp.dll` / `libwebpmux.dll`. Licencia **MIT**.
- El build de llama.cpp existente **no se modifica**.

**Modelos (descarga explícita, nunca automática):**

| Nivel | Modelo | Licencia | Nota |
|---|---|---|---|
| 1 (default y **único**) | Qwen-Image-2.1 GGUF + VAE + `mmproj-Qwen3VL-8B-Instruct-F16.gguf` | **No comercial** | Aceptada por el dueño (C1). Requiere `qwen_image_2_1_prefix_cache_type=q8_0`. Solo la ruta GGUF — **nunca** `int8_convrot`. |
| 2 (spike) | `minimax_h3_fl2va_pruned-Q4_K_M.gguf` (10.64 GiB) + encoder + `--audio-vae` | Restringida (UE/Reino Unido/Corea del Sur/EE. UU.; **Argentina fuera**; la cláusula V.4 prohíbe *mostrar* salidas fuera del territorio) | ~28.25 GiB en disco. Medición únicamente. |

**Infraestructura:**

- Espacio en disco en la unidad que aloje `models/` (E: tiene 412 GB libres; C: tiene 54 GB — ver
  R12).
- Descarga de Hugging Face para los pesos.
- Red local en `127.0.0.1` para el bucle LocalMind ↔ `sd-server`.

**Ninguna dependencia nueva en el crate.** Sin Python, sin runtime nuevo. `bin/` sigue siendo
"vendored prebuilt binaries, NOT built from this repo's source".

---

## Criterios de éxito

### Nivel 1 — Imágenes (compromiso de producto)

- [ ] `sd-server` arranca como proceso de larga duración con un preset, en su propio puerto, con
      estado de salud observable desde la UI, y se para y reinicia limpiamente.
- [ ] LocalMind genera una imagen desde la UI **sin pegar rutas de fichero ni flags a mano**: se
      elige preset, prompt, resolución, semilla.
- [ ] **Reproducibilidad verificable**: cualquier imagen generada se reabre con su semilla y su
      borrador completo; cambiar un parámetro y regenerar con la misma semilla produce la variación
      esperada.
- [ ] El barrido de semillas genera N imágenes y se sabe **qué semilla produjo cada una**.
- [ ] Un trabajo se puede **encolar y cancelar**; cancelar unajob en curso no mata el proceso ni
      pierde la cola.
- [ ] Los dos caminos de upscale están disponibles y **distinguibles** (hires fix que regenera vs
      upscale que no toca el DiT).
- [ ] Cambiar de preset **avisa** de que reinicia el proceso, y lo hace.
- [ ] Los presets filtran flags no soportados por el backend activo: ningún arranque usa
      `--diffusion-fa` ni ninguna ruta int8. **Ningún arranque puede colgar la GPU.**
- [ ] Qwen-Image-2.1 arranca y genera en un equipo de 15.1 GB de RAM **sin OOM**, con la caché de
      prefijo en `q8_0`.
- [ ] `GET /api/models` y `POST /api/start` **conservan su forma** para LLM. CLIs externos siguen
      funcionando sin cambios.
- [ ] Los presets de Qwen-Image-2.1 documentan su restricción de licencia no comercial de forma
      visible.

### Nivel 2 — Vídeo (spike medido)

- [ ] MiniMax-H3 **carga** en esta máquina, o el fallo está capturado con su firma exacta.
- [ ] Se ejecutan **trabajos reales** de `vid_gen` — no extrapolaciones, no simulaciones.
- [ ] El informe registra, medidos: tiempo real de pared por trabajo, RAM pico, y **la
      combinación de resolución / frames / pasos que cabe** en 15.1 GB de RAM.
- [ ] El informe **sustituye explícitamente** la estimación previa de "8–15 min por clip", dizendo
      cuál era el error de la estimación y cuál es el dato real.
- [ ] El informe da un **veredicto accionable**: con estos números, el objetivo de producción
      razonable es T2AV, Ref2AV, o ninguno todavía — y qué haría falta para cada caso.
- [ ] Si no cabe, el informe lo dice con la misma claridad que si hubiera cabido. **Un informe
      negativo es un entregable válido.**

### Puertas (todas, en ambos niveles)

- [ ] `cargo test --manifest-path src-rust/Cargo.toml` — verde (base: 142 tests).
- [ ] `node tests/ui-render.mjs` — verde.
- [ ] `node tests/bench.mjs --self-test` — verde.
- [ ] `node --check` sobre `tests/*.mjs` — verde.
- [ ] **`node tests/ui-tabs.mjs` contra la app viva — puerta DURA y obligatoria**, porque la UI
      cambia estructuralmente. No negociable (C4).
- [ ] `node tests/diffusion.mjs` si se añaden rutas bajo `/api/diffusion/*` *(revisión 2: en lugar de
      extender `node tests/smoke.mjs`)*, y `node tests/e2e.mjs` si se toca la superficie LLM.
- [ ] Sin TDD ritual (`strict_tdd: false`), pero **con** tests unitarios de las funciones puras:
      construcción de argv, filtrado de flags por backend, parseo de `capabilities`, libro de
      semillas. Precedente: `build_engine_cmd`.

---

*Fase propose, **revisión 2** (2026-09-30): reducción de alcance aprobada por el dueño —
`Z-Image-Turbo` fuera del catálogo, ledger plano, sin A/B, sin autodetección de roles ni filtro a
dos severidades, y sin test de contrato HTTP. Ver §Reducción de alcance. No se modificó código del
producto. Artefacto en `openspec/changes/diffusion-generation/proposal.md`; misma clave en Engram
(`sdd/diffusion-generation/proposal`). Siguiente: `design`, `spec` y `tasks`.*
