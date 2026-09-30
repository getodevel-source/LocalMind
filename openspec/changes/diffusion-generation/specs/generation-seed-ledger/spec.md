# Especificación: `generation-seed-ledger`

Capacidad **nueva**. No existen requisitos previos: `openspec/specs/` estaba vacío antes de este
cambio.

> Fase: **spec**. Fecha: 2026-09-30. Cambio: `diffusion-generation`.
> La prosa va en español. Los identificadores, rutas, flags y nombres de fichero van en inglés.

## Purpose

Especificar el **libro de semillas** de LocalMind de extremo a extremo: asignar la semilla,
persistirla junto al borrador completo que la produjo **antes** de lanzar el render, y rehidratar
ese borrador para reproducir la imagen a partir de los parámetros registrados.

Este es el requisito literal del dueño y la condición que hace que exista la UX de iteración.
`POST /sdcpp/v1/img_gen` **no devuelve la semilla usada**: `seed` es un entero único (`-1` =
aleatorio), no hay rango ni incremento, y `batch_count` solo informa `index`, no qué semilla produjo
cada imagen de un lote. Por tanto, sin un libro propio, `seed: -1` produce resultados
**irreproducibles** y "esa me gusta, cambiarle el texto y volver a esa" es imposible.

No es una mejora de la UX. Es la condición que hace que la UX exista.

---

## Requirements

### Requirement: LocalMind es el dueño de la semilla

LocalMind MUST asignar y enviar la semilla de cada generación. `-1` MUST NOT usarse en ningún
trabajo que deba ser reproducible, y `seed: -1` MUST rechazarse —o convertirse en una asignación
explícita persistida— antes de llegar al motor.

La asignación MUST ser determinista y pura: `next_seeds(seed_base, batch_count)` devuelve
`batch_count` semillas consecutivas a partir de `seed_base`, sin I/O, sin estado global y sin
`unwrap`. Esa pureza es la que permite testearla y la que garantiza que la correspondencia del lote
se conoce **antes** de enviar nada.

El motor **no** devuelve la semilla usada, así que la correspondencia **imagen ↔ semilla** solo puede
vivir en LocalMind. `Generation.seeds[i]` MUST corresponderse con `Image.seed` de la imagen de
índice `i`.

#### Scenario: El motor nunca recibe `-1`

- GIVEN una generación de Nivel 1 con `batch_count: 2` y `seed_base: 12345`
- WHEN se envía `POST /sdcpp/v1/img_gen`
- THEN el `seed` enviado es `12345`
- AND la generación registra `seeds: [12345, 12346]`
- AND `-1` no aparece en ninguna petición registrada por el ledger

**Puerta:** `cargo test` (prueba 5 de `design.md` §8.1, `next_seeds(base, n)` = consecutivos) +
gate live `node tests/diffusion.mjs`.

#### Scenario: La correspondencia del lote se conoce sin preguntar al motor

- GIVEN un lote de 3 imágenes completado, devuelto por el motor solo con `index` 0, 1 y 2
- WHEN se consulta `GET /api/diffusion/ledger`
- THEN cada `image` expone su `index` y su `seed`, y el par es consistente con `generation.seeds`
- AND la semilla no se infiere del resultado: LocalMind la sabía de antemano

**Puerta:** gate live `node tests/diffusion.mjs` (ledger ↔ semilla).

---

### Requirement: Ubicación, forma, esquema y retención del ledger

El ledger MUST persistirse en `%APPDATA%\LocalMind\generation\`, replicando el par de convención que
ya existe en `usage.rs` (bitácora append-only + índice cacheado, la única convención de
persistencia no-TOML del proyecto):

```
%APPDATA%\LocalMind\generation\
  ledger.json     # indice autoritativo; tope de imagenes, poda por edad
  events.jsonl    # bitacora append-only; rotacion 1 MiB -> 5000 lineas (mismo criterio que usage.rs:37-38)
  images\<image_id>.png
```

**El esquema es plano: un registro por generación.** No hay nivel `series`, ni `series_id`, ni
`parent_id`, ni agrupación en hilos de iteración. Cada registro MUST llevar prompt, semilla(s),
`preset_id`, **todos** los parámetros de generación, ruta de salida y timestamp. El historial MUST
ser una lista ordenada por fecha, y "volver a esa y cambiarle el texto" MUST resolverse
rehidratando **ese** registro, sin un hilo que lo agrupe.

- **`Generation`** (`id` `g_<unix>-<rand6>`, `created_ts`, `job_id` que devolvió sd-server,
  `preset_id`, `kind` `ImgGen | Upscale`, `seeds: Vec<i64>`, `draft` —**copia exacta de lo
  enviado**—, `status` `Queued|Generating|Completed|Cancelled|Failed`, `image_ids`, `error`).
- **`Image`** (`id` `i_<unix>-<rand6>`, `generation_id`, `index`, `seed`, `width`, `height`,
  `output_format`, `mime_type`, `bytes`, `rel_path`).

Los dos identificadores MUST ser únicos y prefijados por tipo, de modo que `image_id` baste como
clave de unión entre el índice y el almacén de blobs.

`Draft` MUST ser **versionado** (`draft_version: u32`) y MUST contener cada parámetro que la UI
expone: `prompt`, `negative_prompt`, `sampler`, `scheduler`, `steps`, `cfg`, `clip_skip`, `width`,
`height`, `batch_count`, `seed_base`, `loras[]`, `hires{}`, `init_image_id`, `mask_image_id`,
`strength`, `denoise`, `taesd`, `vae_tiling` — **más** los `model_args` de solo lectura del preset,
para que el borrador siga siendo reproducible aunque el preset cambie después.

La resolución del directorio MUST admitir override por variable de entorno
`LOCALMIND_GENERATION_DIR` para tests, replicando el patrón de `usage_path()` (`usage.rs:16`) con
`LOCALMIND_USAGE_PATH`.

La configuración editable MUST vivir en el TOML, en una sección `[generation_diffusion]`, con
`#[serde(default = …)]` para que un `localmind.toml` existente cargue **sin migración**.

El índice MUST tener un tope de imágenes (default **2000**) con poda por edad. El tope MUST ser
configurable, porque es una decisión del dueño, no una constante enterrada.

Servir la imagen MUST hacerse por `GET /api/diffusion/image/{id}`, autenticado, en el mismo origen
que la WebView (la cookie `lm_key` ya la emite `auth.rs:78`).

#### Scenario: Cada generación es un registro plano e independiente

- GIVEN una generación registrada con prompt, semilla, preset, parámetros y ruta de salida
- WHEN se listan las generaciones
- THEN cada una aparece como un registro **independiente**, ordenado por `created_ts` descendente
- AND no existe ningún campo de serie, ni agrupación, ni padre
- AND la generación anterior sigue estando en el historial aunque la nueva la reemita

**Puerta:** `cargo test` (round-trip del esquema plano; el historial sale ordenado sin ninguna noción
de serie) + gate live `node tests/diffusion.mjs`.

#### Scenario: El ledger se resuelve fuera del árbol de modelos

- GIVEN la app viva en la máquina del dueño
- WHEN se genera una imagen
- THEN los ficheros aparecen bajo `%APPDATA%\LocalMind\generation\`
- AND nada se escribe dentro de `models/` ni de `bin/`

**Puerta:** `cargo test` (resolución del directorio con override de env) + gate live
`node tests/diffusion.mjs`.

#### Scenario: La poda respeta el tope configurado

- GIVEN un ledger con más imágenes que el tope configurado
- WHEN se ejecuta la poda
- THEN el índice conserva como máximo el tope de imágenes
- AND los blobs huérfanos de las podadas se eliminan
- AND las generaciones **activas** (con `status` no terminal) nunca se podan

**Puerta:** `cargo test` (ledger, caso de poda).

#### Scenario: La imagen se sirve autenticada

- GIVEN una imagen persistida con `id` `i_…`
- WHEN se solicita `GET /api/diffusion/image/{i_…}` con la cookie `lm_key` válida
- THEN la respuesta es el binario con su `mime_type`
- WHEN se solicita la misma ruta sin credencial
- THEN la respuesta es `401`

**Puerta:** gate live `node tests/diffusion.mjs` (sección de superficie, ruta autenticada y 401).

---

### Requirement: Escritura del ledger ANTES del `POST` (propiedad de supervivencia a crash)

El orden de escritura MUST ser exactamente este, y el paso 2 va **deliberadamente** antes del 3:

```
1. next_seeds(seed_base, batch_count)              -> local, determinista, pura
2. ledger.begin_generation(...)                     -> ESCRIBE A DISCO   <-- ANTES de la llamada
3. POST /sdcpp/v1/img_gen  { seed: seeds[0], ... }  (nunca -1)
4. ledger.attach_job(gen_id, job_id)
5. GET  /sdcpp/v1/jobs/{id}  hasta completed | failed | cancelled
6. blob -> images/<image_id>.<ext>
7. ledger.complete_generation(gen_id, image_ids)
```

Si el proceso de LocalMind muere durante un render de 15 minutos, el ledger **ya** tiene la semilla
y el borrador completo: al reabrir, la generación se puede relanzar con `seed` idéntico. Si se
escribiera después, un crash perdería justo la información cuyo motivo de existir es sobrevivir a un
crash. Esta es la propiedad que convierte el ledger en un taller y no en un panel de demostraciones.

La escritura MUST ser **atómica** (fichero temporal + `rename`), siguiendo el patrón de
`config.rs:788-791`. Un ledger truncado a mitad de escritura es peor que uno ausente.

#### Scenario: Un crash a mitad de render conserva semilla y borrador

- GIVEN una generación de larga duración ya despachada con el ledger escrito
- WHEN el proceso de LocalMind muere antes de que el motor termine
- THEN al reabrir LocalMind, el ledger contiene la `generation` con su `seed`, su `draft` completo
  y un `status` no terminal
- AND la generación se puede relanzar con `seed` idéntico

**Puerta:** `cargo test` (el ledger escribe antes del `POST`; append/rehydrate) + gate live
`node tests/diffusion.mjs` (cancelación y reapertura).

#### Scenario: El estado terminal se escribe al terminar

- GIVEN una generación que completa
- WHEN `ledger.complete_generation` se ejecuta
- THEN `status` pasa a `Completed` y `image_ids` referencia las imágenes persistidas
- AND los blobs existen en `images/<image_id>.<ext>` con el `bytes` declarado

**Puerta:** `cargo test` (ledger) + gate live `node tests/diffusion.mjs`.

#### Scenario: Un fallo se registra, no se pierde

- GIVEN una generación que falla en el motor
- WHEN el polling detecta el estado terminal
- THEN la `generation` queda con `status: Failed` y `error` con la firma del fallo
- AND el `draft` y las semillas siguen disponibles para reintento

**Puerta:** `cargo test` (ledger) + gate live `node tests/diffusion.mjs`.

---

### Requirement: Reproducción exacta desde la semilla registrada

El contrato observable del ledger es que **reabrir un resultado devuelve exactamente lo que lo
produjo**, y que relanzar con la semilla registrada reproduce la misma imagen.

Cuando una generación se reabre, el `draft` MUST rehidratarse **editable** en la UI: cambiar un
parámetro y regenerar debe producir **la variación esperada**, no una imagen distinta porque otro
parámetro se perdió por el camino.

Todos los parámetros que afectan el resultado MUST estar en el `draft`. Si un parámetro que el motor
aplica no está en el `draft`, la reproducibilidad está rota por construcción, y eso es un defecto,
no una limitación documentada.

#### Scenario: Reabrir restaura el borrador exacto

- GIVEN una generación completada con prompt, negative prompt, sampler, scheduler, steps, cfg,
  clip_skip, resolución, LoRAs, hires, seed base y `model_args` registrados
- WHEN se reabre en la UI
- THEN todos esos campos aparecen con los valores con los que se generó
- AND el usuario puede editar uno y relanzar conservando los demás y la misma semilla

**Puerta:** gate live `node tests/diffusion.mjs` (ledger ↔ parámetros) + `cargo test` (rehidratación).

#### Scenario: Mismo draft, misma semilla, misma imagen

- GIVEN una generación completada y registrada
- WHEN se relanza con el `draft` rehidratado **sin cambios**, incluida la misma `seed`
- THEN el resultado es idéntico a la imagen original
- AND la UI muestra que se reprodujo, no una variación

**Puerta:** gate live `node tests/diffusion.mjs`. **Este escenario es live-only y no puede
simularse offline**: la reproducibilidad es una propiedad del par (semilla enviada, semilla
guardada) más el resultado del motor, y sin motor no hay resultado. Un test que simulase el motor
daría verde sobre comportamiento no ejercido — precisamente lo que `config.yaml`
§`strict_tdd_rationale` dice evitar. **Y su parte de bit-identidad NO es verificable**: sd.cpp sobre
Vulkan no garantiza el mismo resultado byte a byte con la misma semilla, porque la planificación del
scheduler no es determinista. Lo verificable es que la semilla enviada y el borrador rehidratado
son los mismos y el motor devuelve una imagen; la igualdad exacta se registra como **no verificada**
en el informe de `verify` y, si el dueño la exige, es una decisión de producto (tolerancia perceptual
o hash), no una tarea de test.

#### Scenario: Un cambio de parámetro produce exactamente la variación

- GIVEN una generación registrada con su semilla
- WHEN se cambia un único parámetro (por ejemplo `steps`) y se relanza con la misma semilla
- THEN el resultado difiere de la imagen anterior
- AND la nueva generación queda registrada como un **registro plano más** en el historial,
  conservando la original y su semilla

**Puerta:** gate live `node tests/diffusion.mjs`.

#### Scenario: La generación reabierta tras un crash conserva la semilla

- GIVEN una generación interrumpida con `status` no terminal
- WHEN se reabre el ledger
- THEN la `generation` muestra su `seed` y su `draft` completos
- AND relanzarla usa esa `seed`, no una nueva

**Puerta:** `cargo test` (append/rehydrate) + gate live `node tests/diffusion.mjs`.

---

### Requirement: Sin migración de datos ni reconciliación

El ledger MUST ser el índice del almacén de imágenes: `image_id` es la clave de unión y no hay nada
que reconciliar. LocalMind hoy **no persiste ninguna imagen generada**, así que no hay migración,
ni import, ni backfill.

La UI MUST NOT ofrecer acciones de migración, importación o reconciliación del ledger, porque no
tienen ningún destino al que migrar.

#### Scenario: No hay nada que migrar

- GIVEN una instalación nueva de LocalMind con el ledger activo
- WHEN se abre la pestaña de generación por primera vez
- THEN el ledger arranca vacío, sin pedir nada al usuario
- AND no existe ninguna acción de "migrar" ni "reconciliar" en la UI

**Puerta:** `cargo test` (ledger vacío es un estado válido) + gate live `node tests/ui-tabs.mjs`.

---

*Fin de la especificación `generation-seed-ledger`. Fase `spec`, **revisión 2** (2026-09-30):
esquema aplanado a un registro por generación —corte 1 aprobado por el dueño—, **6 → 5
requisitos** (se retiran *«Esquema de tres niveles»* y su escenario de serie; el `Draft` versionado
queda dentro de *«Ubicación, forma, esquema y retención»*). No se modificó código del producto.
Siguiente: `tasks`.*
