# Especificación: `diffusion-model-catalog`

Capacidad **nueva**. No existen requisitos previos: `openspec/specs/` estaba vacío antes de este
cambio.

> Fase: **spec**. Fecha: 2026-09-30. Cambio: `diffusion-generation`.
> La prosa va en español. Los identificadores, rutas, flags y nombres de fichero van en inglés.

## Purpose

Definir cómo se representa y se valida un **preset de difusión** —un bundle tipado de componentes,
no un `filename`—, cómo se escanea lo que hay en disco con sus tipos, y cómo se garantiza que ese
catálogo **no altera ni contamina** el catálogo LLM existente.

El requisito central de esta capacidad es la **invariante de separación**: `models/diffusion/` es
del catálogo de difusión, y nada de lo que hay dentro puede alcanzar jamás el catálogo LLM ni
servir de proyector multimodal al motor de texto. Esa invariante no es una preferencia de
organización: es lo que evita que una torre de visión de 8B se use como proyector de un modelo de
texto de 27B, que es una configuración semánticamente falsa que el motor acepta en silencio.

---

## Requirements

### Requirement: Separación por directorio, no por filtro de nombre

El catálogo de difusión MUST vivir bajo `models/diffusion/`, separado por **directorio** del
catálogo LLM, que sigue en `models/` raíz con su comportamiento actual sin cambios.

La separación NO MUST implementarse como un filtro de nombre. El filtro `!name.contains("mmproj")`
de `models.rs:750` y `models.rs:777` (y `process.rs:1189`) **existe y es correcto para el LLM**:
impide que un proyector multimodal aparezca como modelo arrancable, y `validate_req_model`
(`process.rs:1781`) valida contra esa lista. No se toca.

**Motivo de que la separación por directorio sea obligatoria, no preferible:**
`default_mmproj_files()` (`config.rs:469`) devuelve `["mmproj-BF16.gguf", "mmproj-F16.gguf"]` y
`find_mmproj` (`process.rs:1222`) prueba esos candidatos contra `models/`. El **segundo candidato
por defecto es `mmproj-F16.gguf`**, que es exactamente el nombre genérico que publican los releases
GGUF de Qwen-VL. Si la torre de visión de Qwen-Image-2.1 se descarga a `models/` con el nombre
upstream —que es lo natural, así como viene— y `[mmproj] auto = true`, `llama-server` arranca contra
un modelo de texto de 27B **con una torre de visión de 8B como proyector**. Eso no es basura visual:
es una configuración semánticamente falsa que el motor acepta en silencio.

`models/diffusion/` MUST organizarse así: `presets/<preset-id>/` con los componentes que el preset
**declara** en `[generation_diffusion.preset.<id>]` del TOML —**no** un `manifest.json`, que LocalMind
ya no lee: la declaración es la autoridad, y un fichero de metadatos al lado sería una segunda fuente
de verdad que puede discrepar—, más los directorios escaneados `loras/` (→ `--lora-model-dir`),
`upscalers/` (→ `--hires-upscalers-dir`, **ESRGAN RGB, no upscalers latentes**) y `embd/` (→
`--embd-dir`).

#### Scenario: La torre de visión de difusión no puede ser el proyector del LLM

- GIVEN `models/diffusion/presets/qwen-image-2.1/mmproj-F16.gguf` presente en disco
- AND `models/mmproj-BF16.gguf` presente (medido: 931 146 432 B)
- WHEN se resuelve el proyector multimodal del LLM con `pick_mmproj`
- THEN el resultado es `models/mmproj-BF16.gguf`
- AND el `mmproj-F16.gguf` de difusión **no** se considera, aunque esté registrado en `[mmproj] files`
- AND se emite un aviso en el log, nivel `warn`, una vez en el arranque

**Puerta:** `cargo test` (prueba 7 de `design.md` §8.1, sobre `pick_mmproj`).

#### Scenario: La ruta explícita del LLM tiene preferencia

- GIVEN `models/llm-mmproj/mmproj-F16.gguf` y `models/mmproj-F16.gguf`
- WHEN se resuelve el proyector del LLM
- THEN `models/llm-mmproj/mmproj-F16.gguf` gana, por ser la ruta explícita y sin ambigüedad
- AND el sondeo plano en `models/` se conserva, porque el `mmproj-BF16.gguf` que ya existe está
  ahí y moverlo sería tocar dato de usuario

**Puerta:** `cargo test` (prueba 7, orden de candidatos respetado).

#### Scenario: Un `mmproj` suelto en la raíz es reclamado por el catálogo de difusión

- GIVEN `models/mmproj-F16.gguf` suelto en la raíz de `models/`
- WHEN el escáner de difusión lo encuentra por nombre (`VisionEncoder`)
- AND el guardián `diffusion_owned` impide que el LLM lo use como proyector

**Puerta:** `cargo test` (pruebas 4 y 7).

---

### Requirement: Invariante de no contaminación del catálogo LLM

`list_gguf_files` (`models.rs:737`) recorre la raíz de `models/` y **un** nivel de subdirectorio, y
de un subdirectorio solo recoge **ficheros**. `models/diffusion/` contiene únicamente directorios
(`presets/`, `loras/`, `upscalers/`, `embd/`), y `models/diffusion/presets/` está a **dos** niveles.
Por lo tanto **cero** componentes de difusión alcanzan la lista LLM. Lo mismo para `list_models`
(`process.rs:1173`), que solo mira ficheros de la raíz.

Esta invariante MUST verificarse como **test de regresión**, no arguida: sobre un árbol de fixture,
`list_gguf_files` MUST devolver un resultado **idéntico** con y sin el subárbol
`models/diffusion/presets/x/y.gguf` presente.

`list_gguf_files` y `dest_for` (`models.rs:402`) MUST permanecer **intactos**. El catálogo LLM NO
MUST pasar a aceptar `.safetensors`: hacerlo rompería el contrato de `POST /api/start` (que valida
contra la lista), `resolve_model_filename` (`process.rs:1914`) y el chequeo explícito de
`import_model_from_path` (`process.rs:1160`). El hueco de `.safetensors` se cierra **solo del lado
de difusión**.

#### Scenario: El subárbol de difusión es invisible al catálogo LLM

- GIVEN un árbol de fixture con un modelo LLM en `models/` y
  `models/diffusion/presets/x/y.gguf` + `models/diffusion/presets/x/vae.safetensors`
- WHEN se ejecuta `list_gguf_files` sobre el árbol con y sin el subárbol de difusión
- THEN ambos resultados son idénticos
- AND ningún componente de difusión aparece en `GET /api/models`

**Puerta:** `cargo test` (prueba 6 de `design.md` §8.1) + gate live `node tests/e2e.mjs`
(forma de `GET /api/models` intacta, cliente externo real).

#### Scenario: El catálogo LLM no se ensancha con `.safetensors`

- GIVEN `models/diffusion/presets/qwen-image-2.1/vae.safetensors` en disco
- WHEN se llama `GET /api/models`
- THEN el `.safetensors` **no** aparece
- AND un `.gguf` plano en `models/` raíz sigue apareciendo igual que antes del cambio

**Puerta:** `cargo test` (prueba 6) + gate live `node tests/e2e.mjs`.

---

### Requirement: Componentes declarados por el preset, enumerados sin adivinar

**LocalMind MUST NOT deducir el papel de un componente.** No por su nombre de fichero, ni por los
metadatos del GGUF, ni por un `manifest.json`. Cada preset **declara** explícitamente la ruta de cada
componente que necesita, y la validación comprueba **esa** declaración contra el disco:

```toml
[generation_diffusion.preset.qwen-image-2.1]
family = "qwen-image-2.1"
diffusion_model = "presets/qwen-image-2.1/qwen_image_2.1-Q4_K.gguf"
text_encoder     = "presets/qwen-image-2.1/qwen3vl_8b-Q4_K.gguf"
vae              = { path = "presets/qwen-image-2.1/qwen_image_2_1_vae.safetensors", family = "qwen-image-2.1" }
vision_encoder   = "presets/qwen-image-2.1/mmproj-F16.gguf"
```

Un valor puede ser una ruta simple o una tabla `{ path, family }`. **Un componente que el preset no
declara no existe para ese preset**: no se busca, no se deduce, no se inyecta en el `argv`. Y un
`diffusion_model`, un `text_encoder` o un `vae` que no estén declarados MUST producir un
`MissingComponent` que nombre la clave, la ruta esperada y dónde buscó — **nunca** un arranque con un
componente adivinado.

**Lo que sí existe es una enumeración**, `models::list_diffusion_components`, cuyo único trabajo es
**listar lo que hay en disco** para la vista de modelos y para el diagnóstico: `rel` (relativo a
`models/diffusion/`, separadores `/`), `name`, `ext`, `bytes`. **Extensiones aceptadas: `.gguf`,
`.safetensors`, `.ckpt`, `.pt`**, y **sin** exclusión por `mmproj` — deliberado, y es lo contrario de
lo que hace el catálogo LLM, porque para Qwen-Image-2.1 el componente `mmproj-*.gguf` es
**obligatorio**. `DiffusionComponent` **no lleva `role` ni `family`**, porque no hay nada que deducir.

**El hueco de `.safetensors` se cierra solo del lado de difusión.** El catálogo LLM **no** pasa a
aceptar `.safetensors`: hacerlo rompería el contrato de `POST /api/start` (que valida contra la
lista), `resolve_model_filename` (`process.rs:1914`) y el chequeo explícito de
`import_model_from_path` (`process.rs:1160`). Es un cambio de alcance, no un arreglo de un filtro.

**Lo que se pierde, dicho con claridad:** al no leer metadatos, LocalMind **no puede** comprobar que
el `family` que el usuario declara sea el real. `validate_preset` sí rechaza que un componente
declarado lleve un `family` distinto del de su preset —eso atrapa copiar una línea de `wan2.2` en un
preset de `qwen-image-2.1`—, pero un `family` **mentiroso** es un error de configuración que el
código no puede detectar. Declarar la familia correcta es **contrato del usuario**, y se documenta
como tal. Es el precio de no adivinar, y es un precio aceptable: adivinar es lo que produce basura
silenciosa.

#### Scenario: Componentes que el catálogo LLM descartaría, aquí se encuentran

- GIVEN `models/diffusion/presets/qwen-image-2.1/` con `vae.safetensors` y `mmproj-F16.gguf`
- WHEN se ejecuta `list_diffusion_components`
- THEN el `.safetensors` aparece
- AND el `mmproj-*.gguf` aparece, sin exclusión por nombre
- AND ambos reportan su tamaño en bytes
- AND ninguno lleva campo de rol: el papel lo declara el preset, no el escáner

**Puerta:** `cargo test` (`.safetensors` listado; `mmproj-*.gguf` listado sin exclusión; `bytes`
reportado) + `design.md` §8.1.

#### Scenario: Un componente no declarado no llega al argv

- GIVEN un `vae.safetensors` presente en disco que el preset **no** declara
- WHEN se construye el `argv` de ese preset
- THEN el componente no aparece en el `argv`
- AND el resultado es un `MissingComponent` que nombra la clave `vae`, no un arranque degradado

**Puerta:** `cargo test` (prueba 1, `build_sd_cmd`, y prueba 4, `validate_preset`).

---

### Requirement: Un preset es un bundle declarado y validado, no una lista de rutas

Un preset MUST definir `id`, `name` (UI copy, inglés), `family`, `license`, `vram_peak_gb`
(declarado, y medido por el Nivel 1), `vram_class`, `max_resolution_mp` (1.0, el techo de la
atención MATH SDPA, O(n²)), `memory_args` **inmutables**, y la **tabla de componentes declarados**
de la sección anterior.

`validate_preset(preset, disk)` MUST ser **pura** —sin I/O, sin `unwrap`— y MUST devolver el argv
resuelto o la lista de `MissingComponent`. Cada `MissingComponent` MUST llevar **clave de
componente, ruta esperada, dónde buscó y qué encontró**, porque el modo de fallo que se evita es el
silencioso. Sus comprobaciones son, exactamente y solo, estas tres: (a) la ruta declarada existe;
(b) la extensión es de la aceptada; (c) la guardia de familia.

**Guardia anti-VAE-cruzado (riesgo T2, obligatorio):** los VAE **no son intercambiables** entre
familias (Qwen-Image-2.1 vs Qwen-Image vs Wan 2.2). Un componente declarado con `family` distinta de
la del preset MUST ser **error de validación, no un arranque**, y el error MUST detectarse **antes**
del `spawn()`. Con componentes declarados, el `family` es la **declaración** del usuario: la
guardia sigue atrapando el error de configuración más común —copiar la línea de un preset a otro—,
y lo que no puede hacer es descubrir un `family` mintiendo (ver la sección anterior).

`GET /api/diffusion/presets` MUST devolver por preset `{id, name, license, ready, missing[],
vram_class, memory_args, argv_preview}`. **`ready == false` implica que LocalMind se niega a
arrancar y dice qué falta.** Ese es el contrato.

#### Scenario: Un VAE de otra familia es error, no basura

- GIVEN un preset de familia `qwen-image-2.1`
- AND su `vae` se declara con `family: wan2.2`
- WHEN se ejecuta `validate_preset`
- THEN el resultado es error de validación nombrando la clave, la ruta y dónde se buscó
- AND el motor NO se arranca
- AND el mensaje advierte de que los VAE de Qwen-Image-2.1 no son intercambiables con los de
  Qwen-Image ni Wan 2.2

**Puerta:** `cargo test` (prueba 4 de `design.md` §8.1, caso de familia cruzada).

#### Scenario: Un componente ausente se nombra, no se ignora

- GIVEN el preset `qwen-image-2.1` con su `vae` declarado y el fichero **no** presente en disco
- WHEN se ejecuta `validate_preset`
- THEN el resultado lista el componente ausente con clave, ruta y directorio buscado
- AND `GET /api/diffusion/presets` devuelve `ready: false` con ese componente en `missing`
- AND `POST /api/diffusion/start` devuelve `400` en vez de arrancar un motor degradado

**Puerta:** `cargo test` (prueba 4) + gate live `node tests/diffusion.mjs`.

#### Scenario: El preset de Nivel 1 está declarado con su licencia

- GIVEN el catálogo embebido de presets
- WHEN se llama `GET /api/diffusion/presets`
- THEN aparece `qwen-image-2.1` con su licencia **no comercial** marcada
- AND expone `vram_class: Exclusive` y `max_resolution_mp: 1.0`
- AND es el **único** preset de Nivel 1, y por tanto el preset por defecto
- AND `z-image-turbo` **no aparece**: se retiró del catálogo por decisión del dueño, y no queda como
  default, ni como alternativa, ni como opción documentada

**Puerta:** `cargo test` (catálogo embebido; exactamente un preset de Nivel 1) + gate live
`node tests/diffusion.mjs`.

---

### Requirement: Descubrimiento desde `capabilities`, no números a ciegas

La UI MUST poblar sus desplegables de `samplers`, `schedulers` y `loras` descubiertos en
`GET /sdcpp/v1/capabilities`, MUST elegir el formato de salida a partir de
`output_formats_by_mode`, y MUST topar el barrido de semillas con `limits.max_batch_count` en vez de
codificar un máximo inventado. Los `upscalers` deben distinguir su `kind`: `Latent` frente a
`EsrganRgb`.

Discovery no significa codificar números a ciegas: un campo ausente o raro **degrada la UI, nunca
impide arrancar**. Un campo desconocido en la respuesta MUST ignorarse sin error.

#### Scenario: La UI no inventa máximos

- GIVEN `/sdcpp/v1/capabilities` devuelve `limits.max_batch_count: 4`
- WHEN la UI ofrece el barrido de semillas
- THEN el máximo ofrecable es 4
- AND con `limits` ausente el máximo cae a un default conservador documentado, sin error

**Puerta:** `cargo test` (prueba 3, `limits` ausente y presente) + gate live `tests/diffusion.mjs`.

#### Scenario: Los dos tipos de upscaler se distinguen

- GIVEN `/sdcpp/v1/capabilities` devuelve un upscaler `kind: Latent` y otro `kind: EsrganRgb`
- WHEN la UI presenta la lista de upscalers
- THEN ambos aparecen con su tipo distinguible

**Puerta:** `cargo test` (prueba 3) + gate live `tests/diffusion.mjs`.

---

### Requirement: Coexistencia sin alteración del contrato LLM

El catálogo de difusión MUST convivir con el catálogo LLM **sin alterarlo**. `GET /api/models` y
`POST /api/start` MUST mantener su forma actual para LLM. La superficie nueva MUST añadirse como
rutas o campos nuevos, nunca como alteración de las existentes.

`GET /api/diffusion/presets` es **superficie nueva**, y por tanto **no** es una ruptura de contrato:
es aditivo. Cualquier cambio de forma en `/api/models` o `/api/start` sería una ruptura de un
contrato publicado que consumen CLIs externos, y requeriría llamada explícita —que aquí no se da.

`enrich_model_entry` (`models.rs:832`) MUST reutilizarse para componentes **de modelo**, y MUST
NOT reutilizarse para imágenes: su forma (`filename`/`sha256`/`verified`/`source`) es de fichero de
modelo, y forzarle imágenes sería una mentira de esquema.

#### Scenario: Un CLI externo no nota el cambio

- GIVEN un cliente que consume `/api/models` y `POST /api/start` con la forma actual
- WHEN el catálogo de difusión está activo y hay presets instalados
- THEN las respuestas tienen la misma forma y los mismos campos de siempre
- AND ninguna respuesta contiene campos de difusión

**Puerta:** gate live `node tests/e2e.mjs` (cliente externo real contra la superficie publicada) +
gate live `node tests/diffusion.mjs` (el lado de difusión, contra el motor).

---

*Fin de la especificación `diffusion-model-catalog`. Fase `spec`, **revisión 2** (2026-09-30):
componentes **declarados por el preset** en vez de deducidos por nombre/`manifest.json`, y filtro de
flags de un solo comportamiento —corte 3 aprobado por el dueño—; `z-image-turbo` fuera del catálogo.
El recuento de requisitos **no cambia (6)**: *«Escáner de difusión tipado»* pasa a llamarse
*«Componentes declarados por el preset, enumerados sin adivinar»* y pierde 2 de sus 3 escenarios
(«el manifest manda sobre la heurística» y «un componente desconocido no llega al argv» → «un
componente **no declarado** no llega al argv»). No se modificó código del producto. Siguiente:
`tasks`.*
