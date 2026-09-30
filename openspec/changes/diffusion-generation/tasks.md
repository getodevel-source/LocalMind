# Tareas: `diffusion-generation`

Fase: **tasks**. Fecha: 2026-09-30. Cambio: `diffusion-generation`.
Artefacto: `tasks.md`. Store: híbrido (OpenSpec + Engram, clave `sdd/diffusion-generation/tasks`).
Entradas: `proposal.md`, `design.md`, `specs/diffusion-engine-lifecycle/spec.md`,
`specs/diffusion-model-catalog/spec.md`, `specs/generation-seed-ledger/spec.md`,
`specs/image-generation-iteration/spec.md`, `specs/video-generation-feasibility/spec.md`.

> La prosa va en **español**, como `docs/` y `tests/`. Los identificadores, rutas, flags, nombres de
> fichero y los mensajes de commit van en **inglés** porque son del runtime.
> `strict_tdd: false`: nada de ritual TDD. Pero **toda función pura lleva test unitario**, con el
> precedente de `build_engine_cmd`.

## Revisión 2 — reducción de alcance aprobada por el dueño (2026-09-30)

**Esta revisión no rediseña: recorta.** El dueño revisó el plan, lo juzgó sobre-construido y aprobó
**cuatro cortes**. Se aplican aquí y en el resto del conjunto de artefactos (`specs/`, `design.md`,
`proposal.md`). La numeración de tareas y las fronteras de porción **se conservan**: una tarea
retirada mantiene su número y queda marcada, para que el historial siga siendo auditable.

| # | Corte | Qué se va | Ahorro real (líneas de autor) | Lo que **no** se va |
|---|---|---|---|---|
| 1 | **Ledger plano** | El nivel `series`. Un registro por generación con prompt, seed, preset, parámetros, ruta de salida y timestamp. | **−100** | Persistir la semilla **antes** del `POST`; rehidratación; reproducibilidad por parámetros registrados. |
| 2 | **Sin comparación A/B** | La vista de dos imágenes lado a lado en el bucle de iteración. | **−100** | Historial, randomizador de semilla, rehidratación del prompt, disparo del upscaler. |
| 3 | **Sin autodetección de rol, sin dos severidades** | El escáner que adivinaba el rol desde el nombre y el `manifest.json`, y el filtro de flags con severidad blanda + dura. | **−215** | La prohibición de `int8*` (cuelga la GPU en RDNA2) y el descarte de `--diffusion-fa` en Vulkan. |
| 4 | **Sin test de contrato HTTP genérico** | El test de forma de rutas, que `tests/diffusion.mjs` ya ejercita contra el motor real. | **−90** | `tests/diffusion.mjs` como arnés live. `tests/e2e.mjs` sigue siendo la puerta de la superficie LLM publicada. |
| | | **Total** | **−505** | |

**Consecuencia que hay que decir en voz alta:** el aterrizaje real estimado es **~4.758 líneas de
autor** (128 ya aterrizadas), no los ~3.200 previstos. El motivo está desglosado en §Forecast y es
una corrección, no una excusa: **la tabla de forecast anterior no sumaba** (sus filas dan 4.785, su
total declarado era 3.985, y la suma de las estimaciones por tarea de la revisión 1 da 5.195). Los
cuatro cortes retiran el **9,7%** del alcance, no el 23%. Llegar a 3.200 exigiría un **quinto corte de
una capacidad entera**, y ese no se ha inventado porque no está aprobado.

**Cambio de catálogo aprobado en la misma revisión:** **Z-Image-Turbo queda fuera**. El único modelo
de imagen de Nivel 1 es **`qwen-image-2.1`**. Ver §0.1 para lo que eso cuesta en la política de
coexistencia.

### 0.1 Lo que se pierde al dejar Z-Image-Turbo, dicho sin adornos

Z-Image-Turbo era el preset **pequeño** (~9 GB de pico declarado). Qwen-Image-2.1 es **más grande**
(~13 GB). Con el modelo pequeño fuera:

- La regla de promoción `Shared` (§3.3) sigue existiendo como mecanismo, pero su **resultado
  esperado pasa de "posible" a "improbable"**: medir ~13 GB contra los ~6 GB de LLM residente en
  16 GB deja poco margen. El objetivo de medición es ahora **Qwen-Image-2.1**, no Z-Image.
- La mitigación de C1 —"el preset por defecto no depende de la licencia restringida"— **deja de
  existir**: no queda un Tier-1 permisivo. El riesgo de licencia queda concentrado en un único
  modelo y la UI MUST mostrar su licencia **antes** de generar. Se registra como riesgo concentrado,
  no como pregunta abierta: el dueño ya lo aceptó.
- El `VRAM lease / EngineCoordinator` **no se toca**. Sigue siendo la pieza que hace que esto sea
  LocalMind y no un panel de demostraciones.

---

## 0. Cómo se lee este documento

Cada tarea es una **unidad de trabajo Autonomous y commiteable por separado**. La sintaxis es:

```
- [ ] <id> <acción concreta>  ·  PR <n>  ·  ~<N> líneas  ·  PUERTA: <gates>
```

- **PR** → a qué porción (slice) del encadenamiento pertenece. Ver §Forecast.
- **~N líneas** → líneas de autor estimadas (`additions + deletions`). La suma da el total.
- **PUERTA** → los comandos que deben quedar en verde para poder marcar la tarea hecha.

**Gates offline** (los 4 de `openspec/config.yaml` §testing, obligatorios en TODAS las tareas):

```
cargo test --manifest-path src-rust/Cargo.toml
node tests/ui-render.mjs
node tests/bench.mjs --self-test
Get-ChildItem tests/*.mjs | ForEach-Object { node --check $_.FullName }
```

**Gates live** (NO son CI; requieren la app viva en `127.0.0.1:17860`):

```
node tests/smoke.mjs     (existe; tras el corte 4 NO lleva aserciones de difusión)
node tests/e2e.mjs       (existe; puerta de la superficie LLM publicada)
node tests/ui-tabs.mjs   (existe; se le añade "generate" en 5.1)
node tests/diffusion.mjs (nuevo, se crea en 6.1)   <- arnés live de TODO lo verificable de difusión
node tests/diffusion-bench.mjs (nuevo, se crea en 7.2)
```

> **Corte 4 y sus consecuencias.** No hay test de contrato HTTP de forma para las rutas de difusión:
> `tests/diffusion.mjs` las ejercita contra el motor real, y un aserto de forma contra el motor real
> no añade señal. Para que borrar un test **no** abra un agujero, `tests/diffusion.mjs` tiene **dos
> secciones**: la de **superficie** —que **no** necesita motor: 401 sin credencial, denylist, CORS,
> 404 de `/api/diffusion/*` con `enabled = false`— y la de **iteración** —que **sí** lo necesita.
> La primera se puede correr sola con `--mode surface`. Esa es la puerta que **reemplaza** a
> `tests/smoke.mjs` en todos los escenarios de las specs que lo nombraban. La **forma de
> `/api/models` y `POST /api/start`** la sigue cubriendo `tests/e2e.mjs`, que no se toca.

### Reglas duras que condicionan la secuenciación

| # | Regla | Dónde se aplica |
|---|---|---|
| H1 | **`tests/ui-tabs.mjs:25` tiene `TABS` hardcodeado a 7.** La entrada `"generate"` MUST añadirse **en la misma unidad de trabajo** que el cambio estructural de `ui.html`. Si no, el arnés sigue verde sin cubrir la pestaña. | Fase 5, tareas 5.1 y 5.5 |
| H2 | **Un test que simule el motor NO es cobertura** de la iteración. El bucle de iteración solo se ejercita contra la app viva. | Fase 6, y §1 "Cobertura no verificable" |
| H3 | Las **4 gates offline** van en verde antes de cerrar cualquier unidad. | Todas |
| H4 | **Ningún preset declara una ruta `int8*` ni una flag `int8_convrot`** en ninguna de sus dos grafías: cuelga la GPU en RDNA2. Invariante de catálogo **y** filtro antes del `spawn()`. No es descartable. | 1.3, 1.4, 7.1 |
| H5 | `q8_0` es **invariante**, no preferencia: el filtro elimina cualquier token que fije `prefix_cache_type` en `f32`/`f16`/`auto`, y los `memory_args` **inmutables** del preset se aplican **después** del filtro, de modo que el `argv` final lleva `=q8_0` exactamente una vez. | 1.3, 2.3 |
| H6 | **`bin/` no está en git** (`.gitignore:13`, `git ls-files bin` = 0 ficheros). `git status` **no es evidencia**. La prueba es el manifiesto SHA256 de los **71** ficheros preexistentes (medido: 71 ficheros, 0 subdirectorios) + el test estructural. | 0.1, 0.2 |
| H7 | **La ruta de arranque del LLM no se toca.** `build_engine_cmd` (`process.rs:634`) recibe una función hermana, no una modificación. | Todas |
| H8 | `models/` es **dato de usuario**: solo se **crea** `models/diffusion/`. Nada bajo `models/` se escribe, mueve ni borra. | 1.2, 1.4 |
| H9 | **Corte 3 — nada se adivina.** LocalMind **no** deduce el rol de un componente ni por su nombre, ni por los metadatos GGUF, ni por un `manifest.json`. Cada preset **declara** sus rutas; lo declarado se valida contra el disco; lo que no se declara no existe. | 1.1, 1.2, 1.5 |

### La medición va deliberadamente antes que cualquier `VramClass` firme

El pico de VRAM de **`qwen-image-2.1`** es un **número abierto** (`design.md` §14). Decide si su
`VramClass` es `Exclusive` o puede promoverse a `Shared`, y esa decisión cambia la política de
coexistencia. Antes el objetivo era el pico de `z-image-turbo` (~9 GB declarados, el modelo
**pequeño**); con Z-Image-Turbo fuera, **el objetivo es el modelo grande**: ~13 GB declarados, que es
justo el caso duro. Por eso:

- **Tarea 2.4** (tarea pura, offline) declara `Exclusive` como valor **conservador y explícitamente
  provisional**. Nadie lo lee como verdad.
- **Tarea 3.3** produce el número **medido** y corrige el valor. *(Corrección: la revisión 1 decía
  «tarea 3.2»; 3.2 construye el instrumento, 3.3 lo ejecuta. La referencia estaba mal y se corrige
  aquí, no en 3.2.)*
- **Tarea 4.5** (coordinador) **NO** implementa ninguna rama que ramifique sobre
  `vram_class == Shared`. Ramificar sobre un número no medido es hardcodear una suposición en la
  política. La promoción a `Shared` queda **condicionada** al resultado de 3.3.

**Honestidad sobre el resultado esperado:** con ~13 GB medidos contra los ~6 GB de LLM residente en
16 GB, la promoción a `Shared` es **improbable**. El mecanismo se mantiene porque es barato y porque
el número medido manda, pero la **coexistencia no es objetivo de este cambio**: lo es la
**alternancia**, que es lo que `EngineCoordinator` entrega.

---

## 1. Cobertura no verificable — declarado, no inventado

Estas afirmaciones son honestas y las dejo escritas **antes** de que aparezcan como "pendientes" en
`verify`:

| Afirmación | Por qué no hay gate | Qué se hace |
|---|---|---|
| Toda la capacidad `image-generation-iteration` (20 escenarios) | Ninguna gate offline puede ejercitar iteración: no hay motor, ni render, ni cola real. H2 lo prohíbe explícitamente. | El arnés live `tests/diffusion.mjs` se crea en **6.1** y es la única prueba admisible. Hasta 6.1, esos 20 escenarios **no tienen puerta** y se marcan *pendiente de puerta*. |
| **"Mismo draft, misma semilla, misma imagen"** (`generation-seed-ledger`) | La reproducibilidad es una propiedad del par (semilla enviada, semilla guardada) **más** el resultado del motor. Pero sd.cpp sobre Vulkan **no garantiza** bit-identidad entre dos ejecuciones con la misma semilla (planificación no determinista del scheduler). Un assert de hash SHA256 sobre los PNG **fallaría aunque el sistema sea correcto**. | La puerta verifica lo verificable: *misma semilla enviada + borrador rehidratado idéntico + el motor devuelve una imagen*. La igualdad **bit a bit** se registra como **no verificada** en el informe de `verify`, y si el dueño la exige es una decisión de producto (tolerancia perceptual o hash), no una tarea de test. |
| **"La cookie no viaja cross-site"** / la página hostil | Un arnés Node **no puede** ejercitar la política de SameSite del navegador. Solo puede afirmar sus **precondiciones**. | `cargo test` afirma: la cookie lleva `SameSite=Strict` (`auth.rs:78-80`) y `/api/diffusion/*` nunca emite `Access-Control-Allow-Credentials`. El comportamiento de navegador se declara **no ejercido**. |
| **Capa E (firewall `netsh`)** (`design.md` §4.4) | Requiere **elevación de administrador**. No es ejecutable desde `apply`. | No es tarea. Es **acción del dueño**, expuesta en la UI como opt-in (`hardened: true/false`). Se anota en `docs/models/diffusion.md`. |
| **Pico de VRAM medido** (§14 de `design.md`) | No existe instrumento. La sonda que lo produce (`tests/diffusion-vram-probe.mjs`) se construye en **3.2**; la spec lo nombraba `tests/diffusion-bench.mjs`, que es el arnés de **vídeo** y no mide VRAM de imagen. Corregido aquí y en la spec. | Se construye en **3.2** y se usa en **3.3**. Hasta 3.3, el escenario de promoción a `Shared` queda **pendiente de medición** y `Exclusive` sigue vigente, tal y como dice la spec. |
| **Test estructural de `bin/`** en `cargo test` | Depende del estado de la máquina (que el zip esté descargado y descomprimido). Un test hermético de CI **no** puede exigir un artefacto vendorizado de 40 MB en `bin/`. | Se marca `#[ignore]` y se ejecuta explícitamente como paso de verificación, no como parte del `cargo test` normal. El manifiesto SHA256 (0.1) es la prueba de reversión; el test estructural es la garantía de que el diseño hace imposible la colisión. |
| **Que el `family` declarado sea el real** (consecuencia del corte 3) | Sin autodetección, LocalMind **no puede** deducir la familia de un componente: no lee metadatos GGUF ni `manifest.json`. La familia es **declaración del usuario** en el TOML. | `validate_preset` rechaza que un componente declarado tenga `family` distinta de la del preset —esto **sí** se comprueba, y es lo que atrapa copiar una línea de `wan2.2` dentro de un preset de `qwen-image-2.1`—. Que el `family` *declarado* sea el *real* **no es verificable por el código** y se documenta como contrato de configuración en `docs/models/diffusion.md`. Es el precio honesto de no adivinar. |

---

## Fase 0 — Baseline y reversibilidad verifiable

> Precondición de **todo** el cambio. Sin el manifiesto, ninguna de las 14 unidades siguientes tiene
> forma de demostrar que no rompió el motor LLM.

- [x] **0.1** Capturar el manifiesto **SHA256** de los **71** ficheros preexistentes de `bin/`
  (read-only sobre `bin/`; 0 subdirectorios) en `%APPDATA%\LocalMind\bin-manifest.json`, más el
  recuento `{files: 71, subdirs: 0}`. Escribir un script **versionado** que re-hashea y diffea
  (`tools/verify-bin-manifest.ps1`). **Ojo**: `design.md` §9.1(b) decía 70; la spec lo corrigió a
  71 y el recuento medido hoy es 71. **PR 1 · ~60 líneas · PUERTA:** el script re-hashea y reporta
  `0 diferencias` contra sí mismo; las 4 gates offline en verde.
      - Requisitos: `diffusion-engine-lifecycle` / *Reversión verificable por manifiesto de hashes*
        → escenario *«`git status` no es evidencia; el manifiesto sí»*.
  - [x] **0.2** Registrar el **baseline** de las 4 gates offline en el log de la unidad (142 tests
    Rust) y anotar explícitamente que `git status` **no** es evidencia sobre `bin/`
    (`.gitignore:13`, `git ls-files bin` = 0). **PR 1 · ~10 líneas · PUERTA:** las 4 gates
    offline en verde, con el número de tests anotado.

## Fase 1 — Fundaciones puras (tipos, flags, catálogo, configuración)

> Todo aquí es lógica **pura** sin I/O de proceso, y es la fase que más tests unitarios produce.
> Precedente: `build_engine_cmd` (`process.rs:634`).

- [ ] **1.1** Crear `src-rust/src/presets.rs` con las **taxonomías puras**: `ComponentSpec` (la
  declaración de un componente: `key`, `flag` literal de sd.cpp, `path`, `family` opcional,
  `required`), `PresetSpec`, `VramClass` (`Exclusive` | `Shared`), `VramPolicy` (`Auto` | `Manual`),
  `LicenseNote`, `ScanDir` y `MemoryNote`. **Corte 3: no hay `DiffusionRole` de 18 variantes** —el
  rol *es* la flag que el preset declara, y el mapa slot→flag es lo que se valida. Registrar el
  módulo en `src-rust/src/main.rs`.
  **PR 1 · ~70 líneas · PUERTA:** `cargo test` (compila) + las otras 3 gates offline.
      - Requisitos: `diffusion-model-catalog` / *Un preset es un bundle declarado y validado*.
  - [ ] **1.2** En el mismo `presets.rs`, `validate_preset(preset, disk) ->
  Result<ResolvedPreset, Vec<MissingComponent>>` **puro**: sin I/O, sin `unwrap`. Cada
  `MissingComponent` lleva **clave de componente, ruta esperada, dónde buscó y qué encontró** (§5.6
  del design). Comprobaciones, y **solo** estas tres: (a) la ruta declarada existe; (b) la extensión
  es de la aceptada para esa flag (`.gguf`, `.safetensors`, `.ckpt`, `.pt`); (c) **guardia
  anti-VAE-cruzado (T2)**: `family` declarada ≠ `family` del preset ⇒ error, nombrando que los VAE de
  Qwen-Image-2.1 no son intercambiables con los de Qwen-Image ni Wan 2.2. **Corte 3: se eliminan**
  la resolución por lista de nombres candidatos, la lectura del `manifest.json` y la heurística de
  nombre — LocalMind ya no busca, ya no adivina (H9). Un componente **no declarado** por el preset no
  existe para ese preset y no puede llegar al `argv`. **PR 1 · ~110 líneas · PUERTA:** `cargo test`
  con los 4 casos de la prueba 4 de `design.md` §8.1 (ruta declarada ausente → error nombrando
  clave, buscados y dónde; `.safetensors` **aceptado**; `mmproj-*.gguf` **aceptado** sin exclusión;
  VAE de otra familia → error) + las otras 3 gates offline.
      - Requisitos: `diffusion-model-catalog` / *Un preset es un bundle declarado* → escenarios
        *«Un VAE de otra familia es error, no basura»*, *«Un componente ausente se nombra»*;
        `diffusion-engine-lifecycle` / *Filtro de flags* → escenario *«Ningún preset declara un
        componente `int8*`»*.
  - [ ] **1.3** En `presets.rs`, **`filter_denied_flags(argv, backend) -> (Vec<String>,
  Vec<String>)`**: **una** lista de subcadenas denegadas, **un** comportamiento —el token que
  contenga cualquier subcadena de la lista **se elimina del `argv` y se registra una vez**—, aplicada
  **antes** del `spawn()`. La lista, en este orden y sin más reglas:
      1. `int8_convrot` y `int8-convrot` (las **dos** grafías) — **cuelga la GPU en RDNA2**;
      2. `--diffusion-fa`, `--flash-attn`, `-fa` — Vulkan no soporta Flash Attention;
      3. `qwen_image_2_1_prefix_cache_type=f32` / `=f16` / `=auto` — sin FA, `auto` cae a FP32
         (~4 GiB × 2 condiciones ≈ 8 GiB en 15.1 GB de RAM).
      Detección por **subcadena sobre todo el `argv`**, no por lista exacta, para capturar
      `--model-args key=…int8_convrot…` y un nombre de fichero como
      `qwen_image_2.1_int8_convrot.safetensors`. Precedente de ubicación: `psu_unsafe_flag`
      (`config.rs:306`).
      **Cómo sobrevive la invariante de `q8_0` con un solo comportamiento** (H5): los
      `memory_args` del preset se aplican **después** del filtro, y son **inmutables**, así que un
      `extra_flags` con `=f32` desaparece por la regla 3 y el `argv` final lleva `=q8_0`
      **exactamente una vez**. Orden: construir `argv` → `filter_denied_flags` → aplicar
      `memory_args` inmutables → `spawn()`. Sin rechazo condicional, sin dos severidades, sin
      doble logging.
      **PR 1 · ~90 líneas · PUERTA:** `cargo test` con la prueba 2 completa de `design.md` §8.1
      (argv limpio intacto; `--diffusion-fa` fuera; `-fa on` fuera; `=f32` fuera; **ambas** grafías
      de `int8_convrot` fuera, incluso dentro de `--model-args` y como nombre de fichero; el
      `argv` resultante lleva `qwen_image_2_1_prefix_cache_type=q8_0` una sola vez tras aplicar los
      `memory_args`; el descarte se registra **una** vez) + las otras 3 gates offline.
      - Requisitos: `diffusion-engine-lifecycle` / *Filtro de flags* → escenarios *«Flash Attention
        se descarta antes de arrancar»*, *«`int8_convrot` nunca llega al `spawn()`»*; /
        *`q8_0` como invariante* → escenario *«Un `extra_flags` mal puesto no deshace la
        invariante»*.
  - [ ] **1.4** Catálogo embebido de presets en `presets.rs`. **Después del cambio de catálogo solo
  hay un preset de Nivel 1**: `qwen-image-2.1` (**único default**, licencia **no comercial**
  marcada, `memory_args: ["qwen_image_2_1_prefix_cache_type=q8_0"]` inmutables, `Exclusive`
  provisional hasta 3.3, `max_resolution_mp: 1.0`), y `h3-fl2va` (Nivel 2, `AudioVae` fp32
  obligatorio). **`z-image-turbo` se elimina del catálogo** (decisión del dueño, 2026-09-30): no
  queda como default, ni como alternativa, ni como opción documentada. **Ningún preset declara
  ninguna ruta ni flag `int8*`** (H4), verificado con un test que recorre el catálogo embebido
  entero. **PR 2 · ~100 líneas · PUERTA:** `cargo test` (licencias, default único,
  `vram_class`, `max_resolution_mp`, `memory_args` de `q8_0`, **cero** apariciones de cualquier
  subcadena denegada en cualquier componente o flag de cualquier preset) + las otras 3 gates
  offline.
      - Requisitos: `diffusion-model-catalog` → escenario *«El preset de Nivel 1 está declarado con
        su licencia»*; `image-generation-iteration` → *«La licencia no comercial es visible (C1)»*
        → escenario *«La licencia no comercial se ve antes de generar»*.
  - [ ] **1.5** En `src-rust/src/models.rs`, añadir `list_diffusion_components(models_dir) ->
  Vec<DiffusionComponent>` como **enumeración de lo que hay en disco**, con extensiones
  **`.gguf`, `.safetensors`, `.ckpt`, `.pt`** y **sin** exclusión por `mmproj` (deliberado: para
  Qwen-Image-2.1 el `mmproj-*.gguf` es obligatorio). Reporta `rel`, `name`, `ext`, `bytes`.
  **Corte 3: `DiffusionComponent` ya NO lleva `role` ni `family`, y no se deduce nada.** No hay
  heurística de nombre, no hay `manifest.json`, no hay metadatos GGUF. Su único consumidor es la
  vista de "qué hay instalado" y el diagnóstico de rutas; **la validación de un preset no pasa por
  aquí** (1.2 valida contra las rutas declaradas).
  **`list_gguf_files` (`models.rs:737`) y `dest_for` (`models.rs:402`) quedan INTACTOS** (H7).
  **PR 2 · ~55 líneas · PUERTA:** `cargo test` (`.safetensors` listado; `mmproj-*.gguf` listado sin
  exclusión; `bytes` reportado; ningún campo de rol en el tipo) + las otras 3 gates offline.
      - Requisitos: `diffusion-model-catalog` / *Componentes declarados, enumerados sin adivinar* →
        escenarios *«Componentes que el catálogo LLM descartaría, aquí se encuentran»*.
  - [ ] **1.6** **Test de regresión de no contaminación**: sobre un árbol de **fixture** temporal,
  `list_gguf_files` con `models/diffusion/presets/x/y.gguf` + `vae.safetensors` presentes MUST dar
  un resultado **idéntico** al del mismo árbol sin ese subárbol. Y el catálogo LLM **no** acepta
  `.safetensors`. **PR 2 · ~50 líneas · PUERTA:** `cargo test` (prueba 6 de `design.md` §8.1) + las
  otras 3 gates offline.
      - Requisitos: `diffusion-model-catalog` / *Invariante de no contaminación* → escenarios
        *«El subárbol de difusión es invisible al catálogo LLM»*, *«El catálogo LLM no se ensancha con
        `.safetensors»*.
  - [ ] **1.7** En `src-rust/src/config.rs`, añadir `GenerationDiffusionConfig` con
  `#[serde(default = …)]` para que un `localmind.toml` existente cargue **sin migración**:
  `enabled`, `sd_port`, `sd_idle_timeout_secs` (900), `max_vram_reserve_gib` (2),
  `vram_policy` (`Auto` | `Manual`), `backend` (`Vulkan`), `models_subdir` (`diffusion`),
  `ledger_max_images` (2000), y la **tabla de presets** (§1.2 del corte 3). Añadir
  `default_sd_port() -> u16 = 17861` junto a `default_http_port()` y `default_llama_port()`.
  **`MmprojConfig` intacta.** **PR 2 · ~110 líneas · PUERTA:** `cargo test` (TOML viejo carga sin
  migración; defaults presentes; 17861; la tabla de presets deserializa con rutas relativas) + las
  otras 3 gates offline.

      La tabla de presets —**corte 3: esto es lo que sustituye a la autodetección de roles**— se
      declara así (rutas relativas a `models/diffusion/`, valor simple o tabla con `family`):

      ```toml
      [generation_diffusion.preset.qwen-image-2.1]
      family = "qwen-image-2.1"
      diffusion_model = "presets/qwen-image-2.1/qwen_image_2.1-Q4_K.gguf"
      text_encoder     = "presets/qwen-image-2.1/qwen3vl_8b-Q4_K.gguf"
      vae              = { path = "presets/qwen-image-2.1/qwen_image_2_1_vae.safetensors", family = "qwen-image-2.1" }
      vision_encoder   = "presets/qwen-image-2.1/mmproj-F16.gguf"
      ```

      Un valor puede ser una ruta simple o una tabla `{ path, family }`; `family` es **obligatorio**
      en los slots donde el cruce importa (VAE, T2). Un slot omitido **no existe** para ese preset: no
      se busca, no se deduce, no se inventa.
  - [ ] **1.8** En `src-rust/src/process.rs`, extraer `pub fn pick_mmproj(candidates,
  diffusion_owned, exists) -> Option<PathBuf>` como **función pura testeable** (§5.5 del design) y
  reescribir `find_mmproj` (`process.rs:1222`) para usarla, sondeando por candidato y en este orden:
  `models/llm-mmproj/<file>` (nueva, opcional) → `models/<file>` (histórico, **se conserva** porque
  `mmproj-BF16.gguf` ya vive ahí y moverlo sería tocar dato de usuario). Log `warn` **una vez** en el
  arranque si hay una torre de visión de difusión con nombre `mmproj-*`. **PR 2 · ~110 líneas ·
  PUERTA:** `cargo test` (prueba 7 de `design.md` §8.1: con
  `models/diffusion/.../mmproj-F16.gguf` presente → **no** lo elige; `models/mmproj-BF16.gguf` → sí;
  `llm-mmproj/` gana; orden de candidatos respetado) + las otras 3 gates offline.
      - Requisitos: `diffusion-model-catalog` / *Separación por directorio* → escenarios
        *«La torre de visión de difusión no puede ser el proyector del LLM»*, *«La ruta explícita del
        LLM tiene preferencia»*.

## Fase 2 — Motor: colocación, `argv` puro y arranque mínimo

- [ ] **2.1** Descomprimir el zip prebuilt Vulkan de sd.cpp en **`bin/sd-cpp/`** (read-only sobre
  `bin/` raíz: **no** se aplana, **no** se renombra, **no** se recompila). Contenido esperado:
  `sd-server.exe`, `stable-diffusion.dll`, `ggml-vulkan.dll` (la de sd.cpp, ~40.7 MB),
  `webm.dll` / `libwebp.dll` / `libwebpmux.dll`. **PR 3 · ~5 líneas (artefacto, no código) ·
  PUERTA:** re-hashea `tools/verify-bin-manifest.ps1` → `0 diferencias` en los 71 ficheros de `bin/`
  raíz, y `bin/ggml-vulkan.dll` conserva **57 547 264 B** (H6, H7).
      - Requisitos: `diffusion-engine-lifecycle` / *Colocación del motor en `bin/sd-cpp/`*.
  - [ ] **2.2** Test estructural `#[ignore]` **de colocación**, en el módulo de diffused
  (o en `tests/` de Rust): el conjunto de ficheros de `bin/` raíz es **idéntico** al previo, y
  `bin/sd-cpp/` **no** contiene ningún nombre de fichero que ya exista en `bin/` raíz con **tamaño
  distinto`. No es "comprobar que no toqué nada": es que **el diseño hace que no se pueda tocar
  nada**. **PR 3 · ~70 líneas · PUERTA:** `cargo test -- --ignored` explícito (verde) + manifiesto
  SHA256 sin diferencias + las 4 gates offline.
      - Requisitos: `diffusion-engine-lifecycle` / *Reversión verificable* → escenarios
        *«Colocación y resolución de la DLL de Vulkan»*, *«La colisión de nombres es imposible por
        construcción»*.
  - [ ] **2.3** En `src-rust/src/diffusion.rs` (nuevo), `build_sd_cmd(preset, resolved, port,
  cfg) -> Command` como **función pura y testeable**, junto a la resolución de puerto con
  `find_free_port` (`process.rs:897`, preferred + 0..50) y `CREATE_NO_WINDOW` (`process.rs:16`).
  Reglas: programa en `<base>/bin/sd-cpp/sd-server.exe`, **`cwd` = `bin/sd-cpp/`**, **todas** las
  rutas de componentes **absolutas** (resueltas desde las rutas **declaradas** del preset, 1.7),
  `-l 127.0.0.1` explícito, **nunca** `0.0.0.0`, `--max-vram -2` (negativo = VRAM libre − 2 GiB),
  `--auto-fit`. El puerto **nunca** es `1234` (default público de sd.cpp). El **orden** es
  explícito y es contrato: construir `argv` → `filter_denied_flags` (1.3) → aplicar los
  `memory_args` **inmutables** del preset → `spawn()`. **PR 3 · ~180 líneas · PUERTA:** `cargo test`
  con la prueba 1 de `design.md` §8.1 + la prueba 8 (selección de puerto: nunca 1234; respeta
  `find_free_port`; 17861 ocupado → 17862, 17863…) + las otras 3 gates offline.
      - Requisitos: `diffusion-engine-lifecycle` / *Colocación* → escenario *«Rutas relativas no
        pueden colarse en el argv»*; / *Construcción del `argv`* → escenarios *«`argv` completo con
        loopback explícito»*, *«El puerto upstream por defecto está excluido por construcción»*.
  - [ ] **2.4** En el mismo `diffusion.rs`, `parse_capabilities(body) -> Capabilities` **tolerante**:
  `Option`/default en todo, campos desconocidos ignorados, **nunca** entra en pánico con `{}` ni con
  `limits` ausente. `features_by_mode`, `output_formats_by_mode`, `samplers`, `schedulers`,
  `loras`, `upscalers` (con `kind: Latent | EsrganRgb`), `limits` (`max_batch_count`,
  `max_queue_size`, `min/max w/h`, `max_upscale_*`). **PR 3 · ~100 líneas · PUERTA:** `cargo test`
  con la prueba 3 de `design.md` §8.1 (cuerpo completo realista; `{}`; claves extra; `limits`
  ausente; **ninguno** entra en pánico) + las otras 3 gates offline.
      - Requisitos: `diffusion-engine-lifecycle` / *Handshake de salud* → escenario *«Puerto abierto
        pero sin parseo no es sano»*; `diffusion-model-catalog` / *Descubrimiento desde
        `capabilities`* → escenarios *«La UI no inventa máximos»*, *«Los dos tipos de upscaler se
        distinguen»*.
  - [ ] **2.5** En `diffusion.rs`, `DiffusionManager`: `spawn()` con `stderr` por
  `attach_reader_threads` (`process.rs:804`) hacia SSE `/api/events` y log, `stop()` limpio, y el
  **handshake** de salud: sondear `GET http://127.0.0.1:{p}/sdcpp/v1/capabilities` **cada 250 ms**
  hasta un techo derivado del tamaño del bundle (estilo `eta_secs`, `process.rs:1855`).
  **`is_healthy` solo es `true` si el JSON parseó** — "puerto abierto = sano" no se acepta.
  `diffusion_banner(preset, port, plan)` nombrando **lo que se PASA**, no lo que se configuró
  (lección D-44, `process.rs:849-854`). Estado con `starting_for_secs`, `eta_secs`, `verifying`,
  `last_error` — **los mismos campos que la UI ya sabe pintar** (`ServerStatus`,
  `process.rs:26-62`), sin inventar UX de progreso. **PR 4 · ~200 líneas · PUERTA:** `cargo test`
  (reloj de techo, transición de estados) + las otras 3 gates offline.
      - Requisitos: `diffusion-engine-lifecycle` / *Handshake de salud* → escenarios *«Handshake
        correcto activa la salud»*, *«El progreso de carga se ve real»*.
  - [ ] **2.6** Cablear en `src-rust/src/main.rs`: derivar `sd_bin_dir = base_dir.join("bin").join("sd-cpp")`
  (**sin** tocar ninguna derivación existente: `meta::engine_bin_path` sigue resolviendo
  `base_dir/bin/llama-server.exe` y `detect_hardware` sigue usando `bin_dir/llama-server.exe`), y
  construir el `DiffusionManager`. **PR 4 · ~50 líneas · PUERTA:** `cargo test` + las 4 gates
  offline; **el arranque del LLM no cambia** (H7).

## Fase 3 — Router mínimo y **MEDICIÓN del pico de VRAM**

> **Fase intacta por decisión del dueño.** No es ceremonia de test: es el instrumento que produce un
> número que **nadie tiene**. El pico real de VRAM de **`qwen-image-2.1`** (~13 GB declarados) decide
> si su `VramClass` puede promoverse a `Shared`. Se mide **antes** de que nada ramifique sobre él.
> Con Z-Image-Turbo fuera, este es el **caso duro** de la tarjeta: el modelo que queda es el grande.

- [ ] **3.1** En `src-rust/src/server.rs`, rama de prefijo `/api/diffusion/` en `handle_request`
  (`server.rs:442`) **después** del chequeo de auth de la línea **532** y **antes** del `404` final
  — así cae automáticamente bajo D-7 sin abrir un segundo camino de autenticación. Implementar
  **solo** el router mínimo que la medición necesita: `POST /api/diffusion/start`,
  `POST /api/diffusion/stop`, `GET /api/diffusion/status`, `GET /api/diffusion/presets`
  (con `{id, name, license, ready, missing[], vram_class, memory_args, argv_preview}`).
  `start` con un preset `ready: false` → **400** nombrando el componente. `is_public_path` NO incluye
  `/api/diffusion/*`. **PR 5 · ~130 líneas · PUERTA:** `cargo test` (`is_public_path` sigue
  excluyendo las rutas de difusión; `pick` de la rama por allowlist explícita) + las 4 gates offline.
      - Requisitos: `diffusion-model-catalog` → escenario *«Un componente ausente se nombra, no se
        ignora»*; `diffusion-model-catalog` / *Descubrimiento* (superficie `argv_preview`).
  - [ ] **3.2** Crear **`tests/diffusion-vram-probe.mjs`** (live, en español): arranca el motor vía
  `POST /api/diffusion/start` con `qwen-image-2.1`, espera el handshake, lanza **trabajos reales** de
  `img_gen` a la resolución máxima del preset, y muestrea el **pico de VRAM** (y RAM comprometida)
  del proceso `sd-server` con reloj de pared. Salida a **JSON**
  (`tests/fixtures/vram-qwen-image-2.1.json`).
  **Este es el instrumento, no el dato**: se escribe en la unidad anterior a 3.3 para que 3.3 tenga
  con qué trabajar. **PR 5 · ~130 líneas · PUERTA:** `node --check` sobre `tests/*.mjs` (gate offline
  obligatorio) + las otras 3 gates offline. **Ejecución live**: app viva + motor arrancado.
      - Requisitos: `diffusion-engine-lifecycle` / *Correctitud de `VramClass`* (el instrumento que
        la spec nombra, leyendo el pico del proceso `sd-server`).
  - [ ] **3.3** **TAREA QUE PRODUCE EL NÚMERO ABIERTO.** Ejecutar `tests/diffusion-vram-probe.mjs`
  en la máquina del dueño y **corregir `vram_peak_gb`** del preset `qwen-image-2.1` con el valor
  **medido**, no el declarado (~13 GB de la documentación). Si el pico medido es **≤ 6 GB**, anotar
  el cambio a `vram_class: Shared` **pendiente de aplicar en 4.5**; si es **> 6 GB** —**el resultado
  esperado**— `Exclusive` se confirma y se documenta por qué, con el número al lado. El JSON medido
  se versiona. **PR 5 · ~30 líneas · PUERTA:** `cargo test` (el valor declarado del preset = el
  medido) + las 4 gates offline. El **número** lo produce el gate live
  `tests/diffusion-vram-probe.mjs`, y queda **registrado como artefacto**, no como aserto.
      - Requisitos: `diffusion-engine-lifecycle` / *Correctitud de `VramClass` y su regla de
        promoción* → escenario *«La promoción a `Shared` depende de una medición»* — **queda
        `pendiente de medición` hasta que 3.3 se ejecute**, que es exactamente lo que dice la spec.
      - ⚠ **Flag**: esta tarea **no puede completarse** en un entorno sin GPU/pesos. Si el dueño no
        puede descargar `qwen-image-2.1`, 3.3 se cierra como *"número no medido"* y **todo lo demás
        sigue con `Exclusive`**, que es el valor conservador. No se inventa el número. Y si no se
        puede medir, el informe de `archive` lo dice con esas palabras.

## Fase 4 — Libro de semillas (`ledger.rs`)

> La pieza que convierte un panel de demostraciones en un taller. Replica el par de convención de
> `usage.rs` (`usage_path()` con override `LOCALMIND_USAGE_PATH`, `usage.rs:16`).
>
> **Corte 1 — esquema plano.** Un registro por generación. Desaparecen `Series`, `series_id`,
> `parent_id` y `generation_ids`. Cada registro guarda prompt, semilla(s), `preset_id`, **todos** los
> parámetros de generación, ruta de salida y timestamp. El historial es una lista plana ordenada por
> fecha; "volver a esa y cambiarle el texto" se resuelve **rehidratando ese registro**, sin un hilo
> que lo agrupe. **Lo que no se toca:** la escritura **antes** del `POST` (el motor nunca devuelve
> la semilla), la rehidratación y la reproducibilidad por parámetros registrados.

- [ ] **4.1** Crear `src-rust/src/ledger.rs`: esquemas **`Generation` / `Image` / `Draft`**
  (verificados: `draft_version: u32`), `GenerationKind`, `JobStatus`, e IDs prefijados por tipo
  (`g_<unix>-<rand6>`, `i_<unix>-<rand6>`). **Corte 1: sin `Series`.** `Generation` = `id`,
  `created_ts`, `job_id` que devolvió sd-server, `preset_id`, `kind` `ImgGen | Upscale`,
  `seeds: Vec<i64>`, `draft` —**copia exacta de lo enviado**—, `status`
  `Queued|Generating|Completed|Cancelled|Failed`, `image_ids`, `error`. `Draft` contiene **cada**
  parámetro que la UI expone (`prompt`, `negative_prompt`, `sampler`, `scheduler`, `steps`, `cfg`,
  `clip_skip`, `width`, `height`, `batch_count`, `seed_base`, `loras[]`, `hires{}`, `init_image_id`,
  `mask_image_id`, `strength`, `denoise`, `taesd`, `vae_tiling`) **más** los `model_args` de solo
  lectura del preset, para que el borrador siga siendo reproducible si el preset cambia.
  `generation_dir()` con override `LOCALMIND_GENERATION_DIR`, apuntando a
  `%APPDATA%\LocalMind\generation\`. `list_generations()` devuelve el historial **plano** ordenado por
  `created_ts` descendente. **PR 6 · ~120 líneas · PUERTA:** `cargo test` (ledger vacío es un
  estado válido; override de env; round-trip del esquema plano; el historial sale ordenado sin
  ninguna noción de serie) + las 4 gates offline.
      - Requisitos: `generation-seed-ledger` / *Ubicación, forma, esquema y retención* → escenario
        *«Cada generación es un registro plano e independiente»*; / *Sin migración* → escenario
        *«No hay nada que migrar»*.
  - [ ] **4.2** En el mismo módulo, **`next_seeds(seed_base, batch_count)` pura** (consecutivas, sin
  I/O, sin estado global, sin `unwrap`) y la **orden de escritura** §7.4 del design, con el paso 2
  **deliberadamente antes** del 3. **Esto es lo que no se toca y lo que no se negocia**:
  `next_seeds` → `begin_generation` (**escribe a disco**) → `POST` (nunca `-1`) → `attach_job` →
  polling → blob → `complete_generation`. Escritura **atómica** (tmp + `rename`, patrón
  `config.rs:788-791`). `events.jsonl` append-only con rotación 1 MiB → 5000 líneas (criterio de
  `usage.rs:37-38`). **PR 6 · ~165 líneas · PUERTA:** `cargo test` (prueba 5 de `design.md` §8.1:
  `next_seeds` consecutivos; **`begin_generation` escribe antes del `POST`**; transiciones
  `Queued|Generating|Completed|Cancelled|Failed` conservan `draft` y `seeds`) + las 4 gates offline.
      - Requisitos: `generation-seed-ledger` / *LocalMind es el dueño de la semilla* → escenario
        *«El motor nunca recibe `-1`»*; / *Escritura del ledger ANTES del `POST`* → escenarios
        *«Un crash a mitad de render conserva semilla y borrador»*, *«El estado terminal se escribe
        al terminar»*, *«Un fallo se registra, no se pierde»*.
  - [ ] **4.3** En el mismo módulo, **poda** por edad con el tope configurable
  (`ledger_max_images`, default **2000**): borra los blobs huérfanos de las podadas y **nunca** poda
  generaciones con `status` no terminal. Con el esquema plano la poda es una operación **directa**
  sobre la lista ordenada: no hay cascada de serie que recorrer. **PR 7 · ~105 líneas · PUERTA:**
  `cargo test` (poda respeta el tope; borrado de huérfanos; las activas nunca se podan) + las 4 gates
  offline.
      - Requisitos: `generation-seed-ledger` / *Ubicación, forma, esquema y retención* → escenario
        *«La poda respeta el tope configurado»*.
  - [ ] **4.4** Rehidratación: `rehydrate(generation_id) -> Draft` devuelve el borrador **tal como
  se envió**, no una relectura del preset actual. Servir el blob por
  `GET /api/diffusion/image/{id}` **autenticado** (mismo origen que la WebView; la cookie `lm_key`
  ya la emite `auth.rs:78`), con su `mime_type`. **PR 7 · ~110 líneas · PUERTA:** `cargo test`
  (rehidratación exacta tras cambiar el preset en disco; `GET` de imagen sin credencial → 401)
  + las 4 gates offline.
      - Requisitos: `generation-seed-ledger` / *Reproducción exacta desde la semilla registrada*
        (la parte verificable offline) y / *Ubicación, forma, esquema y retención* → escenario
        *«La imagen se sirve autenticada»*.
  - [ ] **4.5** `EngineCoordinator` en `diffusion.rs`: `EngineHolder { Llm, Diffusion }`, tres
  estados (`llm_resident`, `diffusion_resident`, `reloading`), y `VramPolicy { Auto, Manual }`.
  Dependencias **inyectadas por closure** (`llm_busy`, `stop_llm`, `start_llm`, `stop_sd`), con
  `llm_busy` implementado sobre `check_slots_busy` (`process.rs:865`) combinado con la ventana de
  actividad reciente (`last_activity` / `touch_activity()`, `server.rs:1828`) — el criterio ya
  escrito en `new_with_log_file` (`process.rs:176-182`). `AcquireError { Busy { blocked_by,
  retry_in_secs }, Guardrail { message } }`, motivo en **español**.
  **NO** se implementa ninguna rama que ramifique sobre `vram_class == Shared` (§0, arriba).
  **PR 8 · ~170 líneas · PUERTA:** `cargo test` (coordinador con dependencias inyectadas:
  cesión automática ante inactividad; cesión **rechazada** ante ocupación → `Busy`; turno de
  agente en curso nunca se interrumpe) + las 4 gates offline.
      - Requisitos: `diffusion-engine-lifecycle` / *Exclusividad conmutada* → escenarios *«Cesión
        automática ante inactividad»*, *«Cesión rechazada ante ocupación»*, *«Un turno de agente en
        curso no se interrumpe nunca»*.
  - [ ] **4.6** En el mismo coordinador, la **restitución del LLM por `mgr.start()`** al parar
  difusión (a mano o por auto-stop de `sd_idle_timeout_secs`), con `cfg.last.model` +
  `cfg.last.profile` + `cfg.last.context`. La restitución **pasa por los guardarraíles de energía**
  (`start_cooldown_secs` = 120, `max_starts_per_hour` = 4, `check_start_guard`,
  `process.rs:1296`): si el guardarraíl bloquea, el LLM **se queda apagado** y el estado es
  `Guardrail` con el motivo y los segundos restantes. **No se abre puerta trasera.** El ciclo
  stop/start + restart **cuenta** contra el tope de 4 arranques/hora, y eso se avisa. **PR 8 · ~130
  líneas · PUERTA:** `cargo test` (restitución normal; guardarraíl bloquea → `Guardrail` con
  segundos restantes) + las 4 gates offline.
      - Requisitos: `diffusion-engine-lifecycle` / *Restitución del LLM* → escenarios *«Restitución
        normal tras parar difusión»*, *«El guardarraíl de energía no se esquiva»*.
  - [ ] **4.7** En `diffusion.rs`, `proxy_sd(method, path, body) -> Result<Response, String>` hacia
  `127.0.0.1:{sd_port}/sdcpp/v1/*`, con **verificación de identidad** antes de reenviar (el JSON de
  `/sdcpp/v1/capabilities` debe parsear **y** traer el campo propio del build vendorizado; un
  ocupante del puerto recibe `502`, no tráfico — acota la ventana TOCTOU de `find_free_port`).
  **PR 9 · ~150 líneas · PUERTA:** `cargo test` (ocupante del puerto → 502) + las 4 gates offline.
  - [ ] **4.8** En `server.rs`, completar las **6 rutas de proxy nativas** y las **6 rutas de
  LocalMind** de la tabla §4.3 del design. **Denylist explícita** (una decisión, no una omisión):
  LocalMind **NO** reenvía `/v1/images/generations`, `/v1/images/edits` (familia OpenAI de sd-server:
  publicaría generación a cualquier CLI con la clave del gateway), ni `/sdapi/v1/txt2img`,
  `/sdapi/v1/img2img`, `/sdapi/v1/upscalers`, `/sdapi/v1/latent-upscale-modes` (familia A1111).
  La denylist convierte la prohibición en **propiedad del código**. `/api/diffusion/*` **nunca**
  emite `Access-Control-Allow-Credentials` (a diferencia de `sd-server`); el ACAO usa
  `cors_origin_header` (`server.rs:51`) + `is_loopback_origin` (`meta.rs:177`), o sea un `Origin`
  no-loopback no recibe ACAO. **PR 9 · ~230 líneas · PUERTA:** `cargo test` (ruta de la denylist no
  alcanza el motor; ninguna respuesta de difusión lleva `Allow-Credentials`;
  `Origin: https://evil.example` no recibe ACAO; cookie lleva `SameSite=Strict`) + las 4 gates
  offline.
      - Requisitos: `diffusion-engine-lifecycle` / *Superficie HTTP autenticada* → escenarios
        *«La generación no aparece en la superficie publicada»*, *«Sin credencial, `/api/diffusion/*`
        no hace nada»*, *«La cookie de sesión no viaja cross-site»*, *«Un `Origin` no-loopback no
        recibe ACAO»*.
  - [ ] **4.9** En `launcher.rs` (`AGENT_ORDER` en `launcher.rs:22` **sin cambios**) y en
  `handle_launch_generic` (`server.rs:1388`), la guarda de la lease: los **cinco** agentes piden la
  lease para el LLM y, si la tiene la difusión, responden **409** con `blocked_by: diffusion` **sin
  lanzar ningún proceso**. `/v1/images/*` y `/sdapi/v1/*` siguen ausentes del router público.
  **PR 9 · ~90 líneas · PUERTA:** `cargo test` (`is_public_path` y el 409) + las 4 gates offline.
      - Requisitos: `diffusion-engine-lifecycle` → escenario *«Ningún agente queda fuera de la
        guarda»*.

## Fase 5 — UI: pestaña `generate` (**cambio estructural → `ui-tabs.mjs` obligatorio**)

> H1: **la entrada `"generate"` en `TABS` (`tests/ui-tabs.mjs:25`) va en la MISMA unidad de trabajo
> que el cambio estructural de `ui.html`.** Sin ella, el arnés sigue verde sin cubrir la pestaña
> nueva — misma clase de fallo que el P0 de `tests/ui-tabs.README.md`, pero por otra causa.
> `ui.html` son **4843 líneas**: un cambio de pestaña **no** es pequeño y va en el recuento.
> UI copy en **inglés**, por `openspec/config.yaml` §context.

- [ ] **5.1** `ui.html`: botón de navegación `data-tab="generate"` junto a los 7 existentes
  (`ui.html:1436-1456`), registro en `switchTab` (`ui.html:2734`), panel `#tab-generate` en el nivel
  de los paneles existentes (`ui.html:1480` y hermanos) — **nunca anidado dentro de otro panel**.
  Controles mínimos: selector de preset (con `license` visible **antes** de generar), prompt,
  negative prompt, resolución, semilla, y botón **Generate**. Ficha del preset con `memory:
  { prefix_cache_type: "q8_0", approx_gib: 1.06, reason: ... }` y con `vram_class` **explícito**.
  **EN LA MISMA UNIDAD**: añadir `'generate'` al array `TABS` de `tests/ui-tabs.mjs:25`.
  **PR 10 · ~300 líneas · PUERTA:** `node tests/ui-render.mjs` + `node --check` + `cargo test` +
  `bench.mjs --self-test`, y **`node tests/ui-tabs.mjs` contra la app viva** — puerta **dura y no
  negociable** (H1, H3, C4 de la propuesta).
      - Requisitos: `image-generation-iteration` / *Pestaña `generate` y cobertura real* → escenarios
        *«La pestaña nueva se cubre en el arnés live»*, *«`ui-render.mjs` verde no es prueba de
        estructura»*; / *Generación sin rutas ni flags escritos a mano*; / *Licencia no comercial*.
  - [ ] **5.2** `ui.html`: **aviso persistente de lease** en la pestaña **Chat** mientras
  `active == "diffusion"` (*"The chat is paused: the GPU is in use by the image generator."* +
  acción de volver al generador), y el aviso inverso en `#tab-generate` cuando el LLM tomó la GPU.
  Ante un `409` con `blocked_by`, la UI ofrece **el botón exacto que lo resuelve**, no un error
  genérico. **PR 10 · ~70 líneas · PUERTA:** `node tests/ui-render.mjs` + las otras 3 gates offline,
  y **`node tests/ui-tabs.mjs` live** (la pestaña Chat pinta con el aviso).
      - Requisitos: `image-generation-iteration` / *Aviso de exclusividad de VRAM en ambas pestañas*
        → escenario *«El aviso de lease es visible y accionable»*.
  - [ ] **5.3** `ui.html`: **poblar los desplegables desde `capabilities`** — sampler, scheduler,
  LoRAs y formato de salida desde `GET /api/diffusion/capabilities`; **tope del barrido de semillas
  desde `limits.max_batch_count`** (default conservador documentado si el campo falta), **jamás** un
  máximo codificado en el cliente. Un preset con `ready: false` aparece marcado **no disponible**,
  con la lista de `missing`, y su botón deshabilitado. **PR 11 · ~120 líneas · PUERTA:**
  `node tests/ui-render.mjs` + las otras 3 gates offline.
      - Requisitos: `diffusion-model-catalog` → escenario *«La UI no inventa máximos»*;
        `image-generation-iteration` / *Generación sin rutas ni flags* → escenario *«Un preset
        incompleto no se puede arrancar»*.
  - [ ] **5.4** `ui.html`: **bucle de iteración**. (a) **Barrido de semillas** con progreso **por
  imagen** (no spinner de lote), etiquetando cada una con su semilla. (b) **Reusa** de la semilla
  ganadora en el render de mayor calidad. (c) **Variación**: mismo prompt + **misma semilla** + un
  parámetro cambiado, y **metamorfosis** con `init_image_id` + `strength` (+ `mask_image_id`).
  (d) **Los dos upscales, etiquetados antes de confirmar**: *hires fix* (`hires{}` dentro del
  borrador → **regenera**, pasa por el DiT) frente a `POST /api/diffusion/upscale` (**no**
  regenera, **no** toca el DiT). Mostrar el `kind` del upscaler (`Latent` / `EsrganRgb`).
  (e) Validar el techo de **~1 megapixel** antes de lanzar, con la explicación del O(n²) de la
  atención MATH SDPA. (f) **Cola**: reflejar `queue_position` y **Cancelar** sin perder la cola.
  (g) **Reabrir un resultado** desde el ledger: rehidratación editable del borrador completo, con
  la acción de relanzar conservando la semilla. (h) **Aviso de reinicio** al cambiar de preset,
  **antes** de actuar.
  **Corte 2 — lo que NO se construye: la vista A/B.** No hay panel de dos imágenes lado a lado, ni
  rama de lanzamiento con dos prompts. Comparar dos prompts se hace generando dos generaciones por
  separado con la misma semilla y mirándolas en el **historial**, que es lo que ya existe. Se
  elimina también el `(d)` original, que era exactamente esa vista.
  **PR 11 · ~225 líneas · PUERTA:** `node tests/ui-render.mjs` + las otras 3 gates offline, y
  `node tests/ui-tabs.mjs` live (el panel completo pinta).
      - Requisitos: `image-generation-iteration` → *Barrido de semillas* (3 escenarios),
        *Variación y metamorfosis* (2), *Los dos caminos de upscale* (3),
        *Encolado y cancelación* (2), *Aviso de reinicio al cambiar de preset* (1),
        *Reproducción exacta* (rehidratación).
  - [ ] **5.5** `tests/ui-render.mjs`: **checks stub offline** de los controles nuevos de
  `#tab-generate` y de los avisos de lease. Declarar explícitamente en la cabecera del fichero que
  este arnés es **insuficiente para la estructura** y que verde aquí no dice nada sobre si el panel
  pinta. **Sin checks de A/B** (corte 2: la vista no existe, un stub de algo que no existe es ruido
  que además da falsa sensación de cobertura). **PR 11 · ~35 líneas · PUERTA:**
  `node tests/ui-render.mjs` (verde) + `node --check` + las otras 2 gates offline.
      - Requisitos: `image-generation-iteration` → escenario *«`ui-render.mjs` verde no es prueba de
        estructura»*.

## Fase 6 — Gates live: el arnés único

> **H2**: desde aquí es donde la iteración se vuelve verificable. Antes de 6.1, los 20 escenarios de
> `image-generation-iteration` **no tienen puerta** (ver §1).
>
> **Corte 4**: no hay un segundo fichero de contrato HTTP. `tests/diffusion.mjs` es el arnés **único**
> de difusión, y tiene dos secciones para que borrar el test de contrato no deje un agujero.

- [ ] **6.1** Crear **`tests/diffusion.mjs`** (live, en español, sin dependencias): arnés de
  difusión contra la app viva, con **dos secciones**.
  **Sección de superficie** (`--mode surface`, **no necesita motor** — es la que reemplaza al test de
  contrato HTTP retirado): (0a) `GET /api/diffusion/status` sin credencial → **401** y sin tocar el
  motor; (0b) `POST /v1/images/generations` y `POST /v1/images/edits` **no** alcanzan el motor;
  (0c) `Origin: https://evil.example` no recibe ACAO; (0d) ninguna respuesta de difusión lleva
  `Access-Control-Allow-Credentials`; (0e) con `[generation_diffusion] enabled = false`,
  `/api/diffusion/*` devuelve **404** y `GET /api/models` conserva su forma.
  **Sección de iteración** (motor real): (1) ledger ↔ semilla (correspondencia imagen↔semilla a través
  del ledger, con `index` y `seed` consistentes con `generation.seeds`); (2) **cancelar sin perder la
  cola** (el proceso de `sd-server` sigue vivo y el segundo trabajo pasa a estar en curso);
  (3) **barrido** con `max_batch_count` respetado y semilla etiquetada por imagen; (4) rehidratación
  del borrador al reabrir; (5) los **dos upscales** distinguibles; (6) Qwen-Image-2.1 **arranca sin
  OOM** en 15.1 GB con el banner nombrando `q8_0`; (7) handshake: `capabilities` parseado,
  `is_healthy` solo si parseó; (8) cambio de preset: aviso, reinicio, `status` con el `preset_id`
  nuevo. **Corte 2: no hay caso A/B** (la vista no existe). **Un test que simule el motor NO se
  acepta como cobertura** (H2): la sección de iteración habla con el motor de verdad o no cuenta.
  **PR 12 · ~320 líneas · PUERTA:** `node --check` (gate offline obligatorio) +
  las otras 3 gates offline. **Ejecución live**: app viva + motor arrancado + `node
  tests/diffusion.mjs` verde; y `node tests/diffusion.mjs --mode surface` verde **sin** motor.
      - Requisitos: **los 20 escenarios** de `image-generation-iteration`; más
        `generation-seed-ledger` → *Reproducción exacta* (la parte verificable live), *Correspondencia
        del lote*, *Un crash a mitad de render conserva semilla y borrador*;
        `diffusion-engine-lifecycle` → *Ciclo de vida completo*, *Handshake correcto activa la salud*,
        *Superficie HTTP autenticada* (sección de superficie), `q8_0` →
        *«Qwen-Image-2.1 arranca sin OOM en 15.1 GB»*.
  - [ ] **6.2** ~~Contrato HTTP de las rutas nuevas en `tests/smoke.mjs`~~ — **RETIRADA por el corte 4
  (~90 líneas de autor).** `tests/diffusion.mjs` (6.1) cubre las mismas rutas contra el motor real, y
  un aserto de forma contra el motor real no aporta señal que el arnés live no dé ya. **El número se
  conserva** para que el historial de `apply` no tenga huecos: si esta revisión se revierte, esta es
  la tarea que hay que resucitar. **Lo que NO se pierde:** la forma de `GET /api/models` y
  `POST /api/start`, que la cubre 6.3 (`tests/e2e.mjs`, un cliente externo real). **PR 13 · ~0
  líneas · PUERTA:** ninguna; no hay diff que revisar.
      - Requisitos: los mismos, Transferidos a `tests/diffusion.mjs` (6.1) y `tests/e2e.mjs` (6.3).
  - [ ] **6.3** `node tests/e2e.mjs` (sin cambios previstos): confirma que un CLI externo que
  consume `/api/models` + `/api/start` sigue funcionando **con el catálogo de difusión activo**, y es
  ahora **la** puerta de la superficie LLM publicada (lo que 6.2 dejó de ser). **PR 13 · ~0 líneas
  (gate) · PUERTA:** las 4 gates offline sin cambios + `node tests/e2e.mjs`
  live verde.
      - Requisitos: `diffusion-model-catalog` → escenario *«Un CLI externo no nota el cambio»*;
      `diffusion-engine-lifecycle` → *«La superficie LLM publicada no cambia»*.
  - [ ] **6.4** `node tests/ui-tabs.mjs` (live) recorre **8** pestañas, incluida `generate`, y para
  ella comprueba que el panel existe, **no está anidado dentro de otro panel** y tiene
  `offsetHeight > 0`. **Ninguna pestaña de vídeo** (Nivel 2 no añade pestaña). **PR 10 · ~0 líneas
  (gate) · PUERTA:** la app viva + navegador Chromium/Edge; verde en las 8.
      - Requisitos: `image-generation-iteration` → escenario *«La pestaña nueva se cubre en el arnés
        live»*; `video-generation-feasibility` → escenario *«No existe pestaña de vídeo»*.

## Fase 7 — Nivel 2: spike medido de vídeo + documentación

- [ ] **7.1** Preset `h3-fl2va` completo en `presets.rs` (componentes `Diffusion`,
  `MotionModule`, `AudioEncoder`, `AudioVae` **fp32 obligatorio**, `TextEncoder`, `Tokenizer`,
  **declarados** explícitamente en `[generation_diffusion.preset.h3-fl2va]` como los de imagen, sin
  lista de nombres candidatos) y ruta `POST /api/diffusion/vid_gen` en el proxy, **usada por el arnés
  de medición, no por la UI**. Pasa por el **mismo** `validate_preset` y por el **mismo filtro
  único** de flags (1.3). Banner que nombra el plan de memoria que **se pasa** (`--max-vram`,
  `--auto-fit`, `--audio-vae`). Si H3 **no** carga, el fallo se captura con **código de salida y
  `stderr` literal**, no resumido como "no funcionó". **PR 14 · ~200 líneas · PUERTA:** `cargo test`
  (prueba 1 con el preset `h3-fl2va`: el argv no contiene `--diffusion-fa` —se elimina— ni
  ninguna forma de `int8_convrot`; el banner nombra el plan) + las 4 gates offline. **Sin pestaña
  de vídeo.**
      - Requisitos: `video-generation-feasibility` / *Carga de MiniMax-H3* → escenarios *«H3 carga
        con su presupuesto de memoria»*, *«Si H3 no carga, el fallo se captura con su firma»*,
        *«El filtro de flags protege también al preset de vídeo»*.
  - [ ] **7.2** Crear **`tests/diffusion-bench.mjs`** (live, en español): el **arnés de medición**
  del Nivel 2. Ejecuta la **matriz** de combinaciones de resolución / frames / pasos (al menos la
  de referencia de la documentación y al menos una de frames crecientes), con **reloj de pared** y
  **pico de RAM observado** por trabajo, volcado a **JSON**. Incluye el **caso de fallo** con su
  firma. La matriz MUST incluir al menos un trabajo **completado** y al menos un intento que **no**
  cabe, para poder decir cuál cabe y cuál no. **PR 14 · ~180 líneas · PUERTA:** `node --check`
  sobre `tests/*.mjs` (gate offline obligatorio) + las otras 3 gates offline.
      - Requisitos: `video-generation-feasibility` → *Trabajos reales*, *Medición de tiempo de pared
        y de RAM pico* (todos los escenarios).
  - [ ] **7.3** Ejecutar `tests/diffusion-bench.mjs` y escribir el **informe** en
  `openspec/changes/diffusion-generation/video-feasibility-report.md`. El informe MUST:
  (a) **sustituir explícitamente** la estimación de "8–15 min por clip", diciendo **cuál era el
  error** (se calculó sobre `ref2va_pruned-Q2_K`, el único fichero de 6.26 GiB, que exige imagen de
  referencia y **no** hace texto-a-video; el único T2AV razonable es `fl2va_pruned-Q4_K_M` de 10.64
  GiB, ~28.25 GiB en disco) y **cuál es el dato real medido**; (b) dar el **veredicto accionable**
  (T2AV / Ref2AV / ninguno todavía) y **qué haría falta** para cada caso; (c) registrar el
  **pico de RAM observado**, nunca el tamaño de los pesos en disco; (d) si H3 no cabe, decirlo
  **con la misma claridad** que si hubiera cabido, con la firma exacta — un **informe negativo es un
  entregable válido** y el estado del spike es **éxito del spike**; (e) reportar el **espacio en
  disco** si fue el hallazgo (`models/` en C: con 54 GB libres frente a los ~28.25 GiB de H3).
  **PR 14 · ~130 líneas · PUERTA:** las 4 gates offline. El dato lo produce el gate live
  `node tests/diffusion-bench.mjs`; el **artefacto** del informe se revisa en `archive`.
      - Requisitos: `video-generation-feasibility` → *El informe sustituye explícitamente la
        estimación previa* y *Veredicto accionable* (los 4 escenarios).
  - [ ] **7.4** `docs/models/diffusion.md` (en español): layout de `models/diffusion/`, la tabla de
  presets **declarados** con sus licencias (**C1**: `qwen-image-2.1` **no comercial** y **único
  Nivel 1**, `h3-fl2va` restringida y **Argentina fuera** — y la nota explícita de que **ya no hay
  un Tier-1 permisivo** tras retirar `z-image-turbo`), el **porqué** de `q8_0` (~8 GiB en FP32 vs
  ~1.06 GiB en `q8_0` en 15.1 GB), la lista única de flags denegados y **por qué existe**
  (`int8*` cuelga la GPU en RDNA2), los dos caminos de upscale, la política de exclusividad de VRAM y
  sus consecuencias declaradas, el número de VRAM **medido** de 3.3, y la **capa E** (firewall
  `netsh`, opt-in, **requiere elevación → acción del dueño, no paso de `apply`**). **PR 15 · ~130
  líneas · PUERTA:** las 4 gates offline.
      - Requisitos: `image-generation-iteration` → *Licencia visible* (registro documental);
        `video-generation-feasibility` → el resto del report.
  - [ ] **7.5** `docs/licencias.md`: **no existe hoy** (verificado). Crearlo si el dueño lo quiere, o
  registrar explícitamente en `docs/models/diffusion.md` que la política de licencias vive solo ahí.
  **PR 15 · ~20 líneas · PUERTA:** las 4 gates offline.
      - Requisitos: `image-generation-iteration` → *La restricción de licencia no comercial es
        visible (C1)*, parte documental.
  - [ ] **7.6** El interruptor `[generation_diffusion] enabled` (default **false** en el primer
  arranque tras actualizar, **true** después) hace que la reversión del cambio completo sea un
  **cambio de configuración**, no un `git revert`: apagado ⇒ sin motor de difusión, sin rutas
  `/api/diffusion/*`, sin pestaña `generate`, LLM exactamente igual. **PR 14 · ~30 líneas ·
  PUERTA:** `cargo test` (carga de configuración con `enabled = false`; `true` activa) + las 4
  gates offline. **Ejecución live**: `node tests/diffusion.mjs --mode surface` (sección de
  superficie, que es la que comprueba el 404 de `/api/diffusion/*`) + `node tests/e2e.mjs` con
  `enabled = false`.
      - Requisitos: `video-generation-feasibility` → escenario *«Apagar la generación revierte el
        cambio»*; `design.md` §11.

---

## Review Workload Forecast

| Campo | Valor |
|---|---|
| Base de cálculo | **Suma de las estimaciones por tarea** (`~N líneas` de cada checkbox), con `tools/verify-bin-manifest.ps1` a su valor **real** de 128. Es la cifra auditable: la tabla por fichero de la revisión 1 **no sumaba** y se ha sustituido por una que sí reconcilia (ver abajo). |
| Líneas de autor estimadas (total) | **~4.758** (de las cuales **128** ya aterrizaron en la porción 1) |
| Pendiente de aterrizar | **~4.630** |
| Presupuesto de revisión | 400 líneas por PR |
| Riesgo de superar el presupuesto | **High** |
| PRs encadenados recomendados | **Yes** |
| Estrategia de entrega | `auto-chain` |
| Unidades de trabajo autonomous | 15 (PR-13 queda como **gate sin diff** tras el corte 4) |
| Strategy de cadena | **`stacked-to-main`** (fijada por el orquestador al abrir `apply`) |

```text
Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: High
```

> `Decision needed before apply: No` porque `delivery_strategy` es `auto-chain`: el orquestador
> procede con la primera porción usando la estrategia de cadena elegida. Quedó fijada en
> **`stacked-to-main`**: cada porción se encadena a `main` en orden, que encaja con que las 15
> unidades ya son autonomous.

### La tabla de la revisión 1 no sumaba. Se dice antes de discutir el número.

Tres cifras distintas para el mismo objeto, y **ninguna coincidía con las filas que las sostenían**:

| Fuente | Cifra |
|---|---|
| Filas de la tabla por fichero de la revisión 1, sumadas | **4.785** |
| Total **declarado** en la revisión 1 | **~3.985** |
| Estimaciones por tarea de la revisión 1, sumadas | **5.195** |
| Atribución del dueño antes de los cortes | ~4.150 |
| **Estimaciones por tarea de la revisión 2 (esta)** | **~4.758** |

La revisión 2 sustituye la tabla por fichero por una que **sí reconcilia con las tareas**, y publica
la suma de los `~N` de cada checkbox, que se puede comprobar tarea a tarea. Publicar un agregado
que no cuadra con sus propias filas es exactamente el tipo de número que hace que un plan parezca
más barato de lo que es.

### Los cuatro cortes, medidos contra esa base

| Corte | Ahorro atribuido | **Ahorro real (líneas de autor)** | Por qué la diferencia |
|---|---|---|---|
| 1 — Ledger plano | ~250 | **100** | Se va `Series` y una dimensión de agrupamiento. **No se va** el grueso del módulo: orden de escritura antes del `POST`, escritura atómica, `events.jsonl`, poda, borrado de huérfanos, rehidratación y servir autenticado siguen enteros. El nivel `series` era un struct y una vista, no la mitad del fichero. |
| 2 — Sin A/B | ~80 | **100** | Cuadra, y algo mejor: la vista A/B eran ~45 líneas en `ui.html`, ~25 de stub en `ui-render.mjs` y ~30 de caso en el arnés live. |
| 3 — Sin autodetección de rol, filtro único | ~400 | **215** | La enumeración de 18 roles, la heurística de nombre, la lectura del `manifest.json`, la comparación manifest-vs-nombre y sus tests son ~235. **Y se añaden ~20** porque los presets ahora **declaran** rutas en el TOML (1.7), y ~20 menos en el catálogo por quedarse con un único preset de Nivel 1. |
| 4 — Sin contrato HTTP | ~200 | **90** | Las ~110 restantes no eran código: eran prosa de spec y de tarea. Lo que se retiraba eran **90 líneas de aserciones** en `tests/smoke.mjs`, y se reemplazan por la sección de superficie de `tests/diffusion.mjs`, **dentro de su presupuesto**. |
| **Total** | **~930** | **505** | **−9,7% del alcance, no −23%.** |

**Y la conclusión que no se puede esquivar:** el aterrizaje previsto de **~3.200 líneas no sale de
estos cuatro cortes**. Sale de **~4.758**, y quedan **~4.630** por aterrizar. La diferencia son
**~1.430 líneas**. Llegar a 3.200 exigiría un **quinto corte que elimine una capacidad entera** — las
candidatas son el Nivel 2 de vídeo (7.1–7.3, ~510 líneas) o el bucle de iteración completo de la UI
(5.4 más su arnés, ~545) — y **ninguna de las dos está aprobada**, así que no se ha inventado
ninguna. Si el dueño quiere ese número, la conversación es sobre **qué capacidad se cae**, no sobre
cómo estimar mejor.

### Desglose por archivo (cada tarea atribuida a su fichero principal; reconcilia con el total)

| Archivo | Tareas | Líneas ~ | Comentario |
|---|---|---|---|
| `src-rust/src/diffusion.rs` | 2.3, 2.4, 2.5, 4.5, 4.6, 4.7, 7.1 (parcial) | 960 | `build_sd_cmd`, `parse_capabilities`, `DiffusionManager`, `EngineCoordinator`, `proxy_sd`. **Intacto por los cortes**: es la lease de VRAM, que es lo que no se toca. |
| `ui.html` | 5.1, 5.2, 5.3, 5.4 | 715 | Pestaña + panel de iteración **sin A/B**. **4843 líneas de fichero; añadir una pestaña no es pequeño.** |
| `src-rust/src/presets.rs` | 1.1, 1.2, 1.3, 1.4, 3.3, 7.1 (parcial) | 520 | Declaraciones + `validate_preset` + filtro único + catálogo. **Corte 3: −235 sobre 755.** |
| `src-rust/src/ledger.rs` | 4.1, 4.2, 4.3, 4.4 | 500 | Esquema plano + orden de escritura + poda + rehidratación. **Corte 1: −100.** |
| `src-rust/src/server.rs` | 3.1, 4.8, 7.1 (parcial) | 410 | Router mínimo + handlers + **denylist** + 409 de agentes. **Intacto**: es la postura de seguridad. |
| `tests/diffusion.mjs` | 6.1 | 320 | Arnés live, **dos secciones** (cortes 2 y 4). |
| `docs/` | 7.3, 7.4, 7.5 | 280 | `diffusion.md` + nota de licencias + **informe de vídeo**. |
| `tests/diffusion-bench.mjs` | 7.2 | 180 | **Intacto**: arnés de medición del Nivel 2. |
| `src-rust/src/config.rs` | 1.7, 7.6 | 140 | `GenerationDiffusionConfig` + `default_sd_port()` + **tabla de presets** + interruptor. |
| `tests/diffusion-vram-probe.mjs` | 3.2 | 130 | **Intacto**: instrumento de la medición de VRAM. |
| `src-rust/src/process.rs` | 1.8 | 110 | `pick_mmproj` + `sd_bin_dir` + `check_slots_busy`. **Intacto.** |
| `src-rust/src/models.rs` | 1.5, 1.6 | 105 | Enumeración **sin roles** + test de no contaminación. **Corte 3: −85.** |
| `src-rust/src/launcher.rs` | 4.9 | 90 | Guarda de la lease en los 5 agentes. **Intacto.** |
| `bin/sd-cpp/` + test estructural `#[ignore]` | 2.1, 2.2 | 75 | El artefacto binario no cuenta; el test estructural sí. |
| `src-rust/src/main.rs` | 2.6 | 50 | Cableado. **Intacto.** |
| `tests/ui-render.mjs` | 5.5 | 35 | Checks stub, sin A/B. |
| `tools/verify-bin-manifest.ps1` | 0.1 | 128 | **Ya aterrizado** (porción 1). No es una estimación. |
| registro de la porción 0 | 0.2 | 10 | Baseline de las 4 gates. |
| `tests/smoke.mjs` | — | **0** | **Corte 4**: no se toca. |
| `tests/ui-tabs.mjs` | (dentro de 5.1) | 2 | Son **2 líneas** y sin ellas la pestaña no se cubre (H1). |
| **Total** | | **~4.758** | Reconcilia con la suma por tarea. Incluye los 128 ya aterrizados. |

**~12 veces** el presupuesto de revisión. No es un PR, y fingir que lo es sería mentir sobre el
coste de revisión — más todavía cuando el número va **por encima** de lo que se esperaba.

### Suggested Work Units (boundaries de porción)

Cada porción es **autonomous**: se puede revisar, aprobar y mergear sin las siguientes.

| # | PR | Objetivo (deliverable) | Contenido | Test enfocado | Runtime harness | Frontera de rollback |
|---|---|---|---|---|---|---|
| 1 | PR-1 | Reversibilidad y declaraciones puras de preset | 0.1, 0.2, 1.1, 1.2, 1.3 | `cargo test --manifest-path src-rust/Cargo.toml` | `tools/verify-bin-manifest.ps1` (offline, exit 0) | Revertir el commit. `bin/` intacto por construcción; `models/` sin tocar. |
| 2 | PR-2 | Catálogo de difusión y configuración | 1.4, 1.5, 1.6, 1.7, 1.8 | `cargo test` | `tools/verify-bin-manifest.ps1` | Revertir `models.rs` + `config.rs` + `presets.rs`. Sin ruta nueva expuesta todavía. |
| 3 | PR-3 | Motor colocado y `argv` puro | 2.1, 2.2, 2.3, 2.4 | `cargo test` | `tools/verify-bin-manifest.ps1` → **0 diferencias en los 71 ficheros** | **Borrar `bin/sd-cpp/`.** `bin/` raíz intacto. |
| 4 | PR-4 | Ciclo de vida mínimo del motor | 2.5, 2.6 | `cargo test` | arranque real de `sd-server` vía `DiffusionManager` | Revertir `diffusion.rs` + `main.rs`. El LLM no se toca. |
| 5 | PR-5 | Router mínimo + **MEDICIÓN de VRAM** | 3.1, 3.2, 3.3 | `cargo test` + `node --check` | **`node tests/diffusion-vram-probe.mjs`** (live, app + GPU + pesos) | Revertir la rama del router y borrar `tests/diffusion-vram-probe.mjs`. **No tocar el valor `Exclusive` si no se midió.** |
| 6 | PR-6 | Libro de semillas: esquema plano y orden de escritura | 4.1, 4.2 | `cargo test` | ninguno necesario (todo puro + disco temporal vía `LOCALMIND_GENERATION_DIR`) | Borrar `%APPDATA%\LocalMind\generation\`. Ningún efecto en el resto. |
| 7 | PR-7 | Libro de semillas: poda, rehidratación, servir imagen | 4.3, 4.4 | `cargo test` | `GET /api/diffusion/image/{id}` autenticada (live, si el motor está) | Revertir `ledger.rs`. |
| 8 | PR-8 | Coordinador de VRAM | 4.5, 4.6 | `cargo test` (coordinador con closures inyectadas, **sin matar procesos**) | `node tests/diffusion.mjs --mode surface` → rama 409 (live) | Revertir el coordinador: `EngineHolder` vuelve a no existir. **No** tocar `launcher.rs` aquí. |
| 9 | PR-9 | Proxy HTTP y denylist | 4.7, 4.8, 4.9 | `cargo test` | `node tests/diffusion.mjs` (live) | Revertir `server.rs` + `launcher.rs`. `GET /api/models` y `POST /api/start` intactos. |
| 10 | PR-10 | Pestaña `generate` + aviso de lease (**+ `TABS` en el mismo commit**) | 5.1, 5.2, 6.4 | `node tests/ui-render.mjs` + `node --check` | **`node tests/ui-tabs.mjs`** (live, app + navegador) | Revertir `ui.html` + `tests/ui-tabs.mjs` **juntos**. Nunca uno sin el otro. |
| 11 | PR-11 | Bucle de iteración completo en la UI (sin A/B) | 5.3, 5.4, 5.5 | `node tests/ui-render.mjs` + `node --check` | `node tests/ui-tabs.mjs` + `node tests/diffusion.mjs` (live) | Revertir `ui.html`. La API no cambia. |
| 12 | PR-12 | Arnés live único (superficie + iteración) | 6.1 | `node --check` | **`node tests/diffusion.mjs`** (live, motor real) y `--mode surface` (sin motor) | Borrar `tests/diffusion.mjs`. Solo tests. |
| 13 | PR-13 | **Gate sin diff** — superficie LLM publicada intacta | 6.3 (6.2 **retirada** por corte 4) | — | `node tests/e2e.mjs` (live) | No hay diff. Si 6.1 no está, esta porción no aporta señal: es la puerta de la superficie publicada. |
| 14 | PR-14 | Spike de vídeo | 7.1, 7.2, 7.3, 7.6 | `cargo test` + `node --check` | `node tests/diffusion-bench.mjs` (live, motor H3) | `[generation_diffusion] enabled = false` apaga todo. No hay pestaña de vídeo que revertir. |
| 15 | PR-15 | Documentación | 7.4, 7.5 | — | `node tests/bench.mjs --self-test` (gate offline) | Revertir `docs/`. |

**Secuencia obligatoria:** PR-1 → PR-2 → PR-3 → PR-4 → **PR-5 (medición)** → PR-6 → PR-7 →
PR-8 → PR-9 → PR-10 → PR-11 → PR-12 → PR-13 → PR-14 → PR-15.

**Dependencias que no se pueden reordenar:**

1. **PR-5 (medición) antes que PR-8 (coordinador).** El coordinador no puede ramificar sobre
   `vram_class == Shared` con un número sin medir. 4.5 lo prohíbe explícitamente.
2. **PR-6 (ledger) antes que PR-11 (UI de iteración) y PR-12 (arnés live).** El ledger es la
   condición de la reproducibilidad; sin él, el bucle de iteración no se puede construir.
3. **PR-10 (pestaña) antes que PR-11 y PR-12.** Son la misma edición estructural de `ui.html`, y
   `TABS` va con la primera (H1).
4. **PR-12 antes que PR-14.** El arnés live establece el patrón (motor real, nada simulado) que el
   arnés de vídeo replica.
5. **PR-13 depende de PR-9, no de PR-12.** 6.3 comprueba que la superficie publicada no cambió, y eso
   lo verifica el router de PR-9. Es una unidad de **gate**, sin diff: si PR-9 y PR-12 están
   aterrizados, PR-13 es un `node tests/e2e.mjs` en verde y nada más. **No se puede usar PR-13 como
   Review Workload Forecast de sí mismo.**

**Sobre el estilo de cadena** — elegido por el orquestador: **`stacked-to-main`**, cada porción a
`main` en orden. Encaja porque las 15 unidades ya son autonomous y cada una tiene su frontera de
rollback. Se deja constancia de las alternativas para que la decisión sea revisable:

- **Stacked to main** ← **la elegida.** Cada porción ya es autonomous, así que funciona bien.
- **Feature branch chain** — PR-1 apunta a `feature/diffusion-generation` y cada PR posterior al
  inmediato anterior; solo la rama tracker llega a `main`. Tendría valor si PR-8 (el coordinador,
  que toca la ruta de arranque de los agentes) quisiera una frontera de rollback más fina.
- **`size:exception`** — un solo PR de ~4.758 líneas. **No recomendable**, y ahora todavía menos:
  los cortes lo bajaron de 5.195, pero la granularidad sigue saliendo gratis y una sola revisión
  de 4.758 líneas no la sostiene nadie.

### Lo que ninguna porción cubre, y es deliberado

- El **contenido de `bin/sd-cpp/`** no cuenta como líneas de autor (son binarios prebuilt, ~78 MB).
  La **garantía** de que no rompen nada es el manifiesto SHA256 y el test estructural, no un diff.
- El **informe de vídeo** (7.3) son ~130 líneas de prosa con números medidos. Si el spike no puede
  ejecutarse (sin pesos, sin espacio), la tarea se cierra con **el hallazgo**, no con un número
  inventado. Un informe negativo es un entregable válido.

### Divergencias de esta revisión — dónde no se pudo aplicar un corte al pie de la letra

Ninguna de las cuatro divergencias es una ampliación de alcance: son consecuencias que **elegí
registrar** en vez de resolver en silencio.

1. **Corte 3 y la guardia anti-VAE-cruzado (T2).** El corte elimina la lectura del `manifest.json`,
   y el `family` venía de ahí. Si se eliminara también la guardia, un VAE de Wan 2.2 en un preset de
   Qwen-Image-2.1 arrancaría y produciría basura. **Se conserva**, y su fuente pasa a ser la
   **declaración** en el TOML (§1.7). Lo que se pierde, y se dice en §1, es poder comprobar que el
   `family` declarado es el real: eso ya no es verificable por código.
2. **Corte 3 y la invariante de `q8_0`.** El corte pide un solo comportamiento de filtrado, y
   rechazar `=f32` era un segundo comportamiento. Pero **eliminar** ese token sin más es peor que
   el original: el motor caería a `auto` → FP32 → ~8 GiB → OOM. La solución que respeta "una lista,
   un comportamiento" es **el orden**: `filter_denied_flags` primero, `memory_args` inmutables
   después. Un solo comportamiento, y la invariante intacta (H5).
3. **Corte 4 y la puerta de las rutas de difusión.** Borrar el test de contrato dejaba sin ejecutor a
   los escenarios de 401, denylist, CORS y "apagado = 404". No se ha resucitado un fichero de
   contrato: se ha añadido la **sección de superficie** de `tests/diffusion.mjs` (6.1), que no
   necesita motor. Es la cobertura que faltaba, dentro del arnés que ya estaba pagado.
4. **Corte 1 y el recuento de requisitos.** `generation-seed-ledger` baja de **6 a 5**
   requisitos: el de *«Esquema de tres niveles»* existía **por** la serie, y sin serie su contenido
   (el `Draft` versionado, los IDs por tipo) vive dentro de *«Ubicación, forma, esquema y
   retención»*. `image-generation-iteration` baja de **11 a 10** por el requisito de A/B. Los otros
   tres conservan su recuento. Los escenarios de `image-generation-iteration` pasan de **21 a 20**.

---

## Trazabilidad requisito → tarea

| Capacidad | Requisitos (11/6/6/11/6 → **11/6/5/10/6**) | Tareas |
|---|---|---|
| `diffusion-engine-lifecycle` | 11 (sin cambio) | 0.1, 0.2, 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 3.1, 4.5, 4.6, 4.7, 4.8, 4.9, 6.1, 6.3, 7.1 |
| `diffusion-model-catalog` | 6 (sin cambio) | 1.1, 1.2, 1.4, 1.5, 1.6, 1.7, 1.8, 2.4, 3.1, 5.3, 6.3 |
| `generation-seed-ledger` | **5** (era 6) | 4.1, 4.2, 4.3, 4.4, 5.4, 6.1 |
| `image-generation-iteration` | **10** (era 11) | 5.1, 5.2, 5.3, 5.4, 5.5, 6.1, 6.4, 7.4, 7.5 |
| `video-generation-feasibility` | 6 (sin cambio) | 6.4, 7.1, 7.2, 7.3, 7.6 |

**Escenarios sin puerta hoy** (§1): los 20 de `image-generation-iteration` hasta 6.1; el escenario de
promoción a `Shared` hasta 3.3; las propiedades de SameSite solo como precondición afirmada; el
`family` declarado vs real, no verificable por diseño.

---

*Fase tasks, **revisión 2**. No se modificó código del producto. Artefacto en
`openspec/changes/diffusion-generation/tasks.md`; misma clave en Engram
(`sdd/diffusion-generation/tasks`, **upsert**). Siguiente: `sdd-apply`.*

---

## Registro de `apply` — Fase 0 (PR-1, porción 1 de 15)

> **Esta porción NO se ve afectada por la revisión 2.** 0.1 y 0.2 no aparecen en ninguno de los
> cuatro cortes, y el manifiesto SHA256 sigue siendo la prueba de reversión. Se conserva el
> registro tal cual, con sus números medidos.

**Rama**: `diffusion-generation/pr-1-bin-manifest` (encadena a `main`). **Commit**: `710f9cb`.

**Baseline de las 4 gates offline — registrado aquí porque `git status` NO es evidencia sobre
`bin/`** (`.gitignore:13` contiene `bin/`; `git ls-files bin` devuelve **0 ficheros**; comprobado
hoy). `git status` limpio antes y después no demuestra nada del motor: es verde por ausencia.

| Gate | Resultado observado | Exit |
|---|---|---|
| `cargo test --manifest-path src-rust/Cargo.toml` | `running 142 tests` → `142 passed; 0 failed; 0 ignored` (0.04s) | 0 |
| `node tests/ui-render.mjs` | **97** líneas `PASS` | 0 |
| `node tests/bench.mjs --self-test` | **23** líneas `PASS` → `self-test: all assertions PASS` | 0 |
| `Get-ChildItem tests/*.mjs \| ForEach-Object { node --check $_.FullName }` | **7** ficheros, todos exit 0 | 0 |

**Manifiesto**: `%APPDATA%\LocalMind\bin-manifest.json`, 17 309 B, `schema_version: 1`,
`{files: 71, subdirs: 0}`, 71 entradas `{name, bytes, sha256}`, capturado el 2026-09-30.
`bin/ggml-vulkan.dll` registrado en **57 547 264 B** — el valor que la spec exige conservar.

**Verificación de la propia puerta (para que el verde no sea vacuo):** re-hashear contra el
manifiesto reporta `diferencias: 0`, exit 0. Un control negativo en un directorio temporal
descartable (fichero modificado + fichero nuevo + fichero borrado) reporta `diferencias: 3` con
`faltan` / `nuevos` / `modificados` nombrados y **exit 1**. La puerta puede ponerse roja.

**Desviación de la estimación**: el forecast decía ~60 líneas para 0.1; el script tiene **128
líneas de autor** (`tools/verify-bin-manifest.ps1`, único fichero del commit). La diferencia es el
bloque de documentación, el control de `schema_version` y el diff del recuento `files`/`subdirs`,
que son lo que hace la puerta reutilizable en las 14 porciones siguientes. Sigue muy por debajo
del presupuesto de 400; no se recortó para encajar.

**No tocado en esta porción**: `bin/` (read-only), `models/`, `ui.html`, código Rust, `src-rust/`.
No se creó `presets.rs`, `diffusion.rs` ni `ledger.rs`.

---

## Registro de `tasks` — revisión 2 (2026-09-30, reducción de alcance)

**Rama**: `diffusion-generation/pr-1-bin-manifest` (ya creada; no se ha cambiado de rama porque
este cambio es solo de docs). **Cambio**: docs únicamente, en
`openspec/changes/diffusion-generation/`.

**Ficheros tocados**: `tasks.md` (este), los **5** `specs/*/spec.md`, `design.md`, `proposal.md`.
**Ningún fichero de código de producto.** Ni `src-rust/`, ni `ui.html`, ni `tests/`, ni `bin/`, ni
`models/`.

**Los cuatro cortes**, aplicados tal cual y sin nada más:

| Corte | Dónde |
|---|---|
| 1 — Ledger plano | 4.1, 4.2, 4.3 · `generation-seed-ledger` (6 → 5 requisitos) · `design.md` §7.3 |
| 2 — Sin A/B | 5.4, 5.5, 6.1 · `image-generation-iteration` (11 → 10 requisitos, 21 → 20 escenarios) · `design.md` §8.2 |
| 3 — Sin autodetección ni dos severidades | 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7, 7.1 · `diffusion-model-catalog` §*Escáner* → §*Componentes declarados* · `diffusion-engine-lifecycle` §*Filtro de flags* → un solo comportamiento · `design.md` §5.4, §5.6, §6.2 |
| 4 — Sin contrato HTTP | 6.2 **retirada** (número conservado) · todas las puertas de spec que nombraban `tests/smoke.mjs` → `tests/diffusion.mjs` |

**Intactos por decisión del dueño**: `tests/diffusion-vram-probe.mjs` y Fase 3 · Fase 7 completa
(instrumento + informe de H3) · `EngineCoordinator` y la lease de VRAM (4.5, 4.6) · la denylist y
el CORS del proxy (4.7, 4.8, 4.9) · `bin/sd-cpp/` con rutas absolutas (2.1–2.3).

**Cambio de catálogo**: `z-image-turbo` eliminado de todo el conjunto de artefactos. Único modelo de
Nivel 1: `qwen-image-2.1`. Objetivo de medición de VRAM: su pico (~13 GB declarados).

**Forecast**: **~4.758** líneas de autor (128 ya aterrizadas, **~4.630** pendientes), −505 respecto a
la base de la revisión 1. **No** son los ~3.200 previstos: el desglose está en §Forecast y la
diferencia son ~1.430 líneas que exigirían un quinto corte no aprobado.

### Las 4 gates offline, ejecutadas tras la revisión

| Gate | Resultado | Exit |
|---|---|---|
| `cargo test --manifest-path src-rust/Cargo.toml` | ver tabla de resultados al pie de esta revisión | 0 |
| `node tests/ui-render.mjs` | idem | 0 |
| `node tests/bench.mjs --self-test` | idem | 0 |
| `Get-ChildItem tests/*.mjs \| ForEach-Object { node --check $_.FullName }` | idem | 0 |

Las 4 estaban en verde **antes** de esta revisión y se ejecutan después: es un cambio de docs, así
que el resultado esperado es "sin cambios", y eso es exactamente lo que hay que comprobar. Los
**valores observados** se anotan en el mensaje de retorno de la fase, no aquí, para no escribir un
número que no se ha medido en este commit.

<!-- sdd:end -->