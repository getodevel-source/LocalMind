# Especificación: `image-generation-iteration`

Capacidad **nueva**. No existen requisitos previos: `openspec/specs/` estaba vacío antes de este
cambio.

> Fase: **spec**. Fecha: 2026-09-30. Cambio: `diffusion-generation`.
> La prosa va en español. Los identificadores, rutas, flags y nombres de fichero van en inglés.
> La **copia de UI va en inglés**, por `openspec/config.yaml` §context (`Style: identifiers and UI
> copy in English`).

## Purpose

Especificar la UX de iteración sobre imágenes generadas, que es el **requisito titular del dueño**:
no "generar una imagen" —eso lo hace cualquier web— sino **iterar sobre ella**: volver a un
resultado, cambiar un parámetro, reusar la semilla, comparar dos prompts con la misma semilla.

Esta capacidad es Nivel 1 del cambio y es el **único punto donde una gate offline no puede validar
el comportamiento**: el stub DOM de `tests/ui-render.mjs` no detecta HTML roto, y el P0 ya ocurrido
en `tests/ui-tabs.README.md` —un `<div>` sin cerrar que entró a producción con las comprobaciones
stub en verde— tiene aquí su análogo declarado.

---

## Requirements

### Requirement: Pestaña `generate` y cobertura real en el arnés de pestañas

La UI MUST exponer una pestaña `generate` junto a las 7 existentes, registrada en el `switchTab`
existente (`ui.html:2734`, botones de navegación en `ui.html:1436-1456`) y con su panel
`#tab-generate`.

**Requisito explícito y no obvio:** añadir la pestaña MUST venir acompañado de añadir `"generate"`
al array `TABS` de `tests/ui-tabs.mjs:25`, hoy hardcodeado a 7 entradas:

```js
const TABS = ['dashboard','chat','models','usage','terminal','info','settings'];
```

Sin esa actualización, el arnés **sigue en verde sin cubrir la pestaña nueva** — y lo hace por un
motivo distinto al P0 documentado, pero de la misma clase de fallo: cobertura aparente sobre
comportamiento no ejercido. Esta es una **tarea explícita**, no una consecuencia esperable de
"añadir una pestaña".

Añadir una pestaña es un **cambio estructural** de `ui.html`, no solo de lógica: es exactamente el
caso que el stub DOM no puede validar. Por tanto `node tests/ui-tabs.mjs` contra la app viva es
**puerta dura y obligatoria** (`config.yaml` §testing `conditional_gates: structural HTML edit`, y
C4 de la propuesta).

`tests/ui-render.mjs` MUST cubrir los controles nuevos como stub offline, y MUST declararse
**explícitamente insuficiente** para la estructura: verde en `ui-render.mjs` no dice nada sobre si
el panel pinta.

#### Scenario: La pestaña nueva se cubre en el arnés live

- GIVEN la app viva en `127.0.0.1:17860` con la pestaña `generate` añadida
- WHEN se ejecuta `node tests/ui-tabs.mjs`
- THEN el arnés recorre 8 pestañas, incluida `generate`
- AND para `generate` comprueba que el panel existe, no está anidado dentro de otro panel, y tiene
  `offsetHeight > 0`

**Puerta:** gate live `node tests/ui-tabs.mjs` (**obligatoria**; la app debe estar viva).

#### Scenario: `ui-render.mjs` verde no es prueba de estructura

- GIVEN un cambio estructural en `ui.html` que anide `#tab-generate` dentro de otro panel
- WHEN se ejecuta `node tests/ui-render.mjs` offline
- THEN el resultado puede ser verde
- AND el defecto solo aparece con `node tests/ui-tabs.mjs` contra la app viva

**Puerta:** ambas, y la segunda **no es opcional** en un cambio estructural.

---

### Requirement: Generación sin rutas ni flags escritos a mano

Desde la UI, LocalMind MUST permitir elegir **preset, prompt, resolución y semilla** y generar, sin
que el usuario tenga que pegar rutas de fichero ni escribir flags.

Los desplegables de sampler, scheduler, LoRAs y formato de salida MUST poblarse desde
`GET /sdcpp/v1/capabilities` (discovery), y los límites MUST tomarse de
`limits.max_batch_count`, nunca de un número inventado en el código.

Un preset con `ready: false` MUST aparecer en la UI marcado como no disponible, con la lista de
componentes que faltan, y su botón de generar MUST estar deshabilitado. `POST /api/diffusion/start`
con un preset no listo MUST devolver `400`.

#### Scenario: Elegir preset y generar

- GIVEN el motor de difusión arrancado con un preset `ready: true`
- WHEN el usuario elige preset, escribe el prompt, fija resolución y semilla y pulsa generar
- THEN el trabajo se encola sin que el usuario escriba ninguna ruta ni flag
- AND la UI muestra la cola con `queue_position` y el estado del trabajo

**Puerta:** gate live `node tests/diffusion.mjs` + gate live `node tests/ui-tabs.mjs`.

#### Scenario: Un preset incompleto no se puede arrancar

- GIVEN un preset con `ready: false` por faltar su VAE
- WHEN la UI lista los presets
- THEN ese preset aparece con `missing` nombrando el componente
- AND su acción de arrancar está deshabilitada
- AND al invocarla igualmente por API, la respuesta es `400` con el componente nombrado

**Puerta:** `cargo test` (prueba 4) + gate live `node tests/diffusion.mjs`.

---

### Requirement: Barrido de semillas con progreso por imagen

El barrido de semillas MUST generar N imágenes y el usuario MUST poder saber **qué semilla produjo
cada una**. El máximo ofrecible MUST venir de `limits.max_batch_count` reportado por
`/sdcpp/v1/capabilities`, con un default conservador documentado cuando el campo falte.

El progreso MUST ser **por imagen**, no un spinner de lote: la UI MUST reflejar
`batch_count`, cuántas imágenes han completado y cuál es la semilla de cada una, porque la
corresponcia índice↔semilla solo la conoce LocalMind.

#### Scenario: El barrido muestra la semilla de cada imagen

- GIVEN `batch_count: 4` y `seed_base: 5000`, con `limits.max_batch_count >= 4`
- WHEN el barrido completa
- THEN la UI muestra 4 imágenes, cada una etiquetada con su semilla 5000, 5001, 5002 y 5003
- AND el ledger registra las cuatro con su `index` y su `seed`

**Puerta:** gate live `node tests/diffusion.mjs` (barrido con `max_batch_count` respetado).

#### Scenario: El barrido se topa por el límite del motor

- GIVEN `/sdcpp/v1/capabilities` con `limits.max_batch_count: 4`
- WHEN el usuario intenta un barrido de 16
- THEN la UI limita la petición a 4
- AND el máximo no está codificado en el cliente

**Puerta:** `cargo test` (prueba 3, `limits`) + gate live `node tests/diffusion.mjs`.

#### Scenario: La semilla ganadora se reusa en el render de mayor calidad

- GIVEN un barrido en el que la imagen de la semilla 5002 es la elegida
- WHEN el usuario lanza el render de mayor resolución con esa imagen
- THEN el render usa la semilla 5002, no una nueva
- AND esa reusa es el mismo principio del Nivel 2 aplicado al Nivel 1

**Puerta:** gate live `node tests/diffusion.mjs`.

---

### Requirement: Variación y metamorfosis

La UI MUST ofrecer variación sobre una imagen existente, por dos vías distintas y explícitas:

1. **Variación de parámetros**: mismo prompt, **misma semilla**, un parámetro cambiado. Produce la
   variación esperada y la registra como una generación **nueva** en el historial.
2. **Metamorfosis / img2img**: `init_image` (la imagen existente por `init_image_id`) más
   `strength`. `mask_image` también está disponible como parte del bucle de variación.

Ambos caminos MUST usar la semilla registrada, no `-1`, y MUST quedar registrados en el ledger con su
`draft` completo, de modo que la variación sea a su vez reproducible.

Inpainting avanzado, face-swap, workflows de nodos tipo ComfyUI y la **comparación A/B de dos
prompts** MUST NOT formar parte de este cambio: quedan fuera de alcance y no deben construirse. Dos
prompts se comparan generando dos generaciones por separado con la misma semilla y mirándolas en el
historial.

#### Scenario: Variación de un parámetro con misma semilla

- GIVEN una imagen generada con semilla 5002
- WHEN el usuario pulsa "variar" cambiando solo `cfg`
- THEN el trabajo se lanza con la semilla 5002 y el resto de parámetros idénticos
- AND la generación queda registrada como una entrada más del historial, con su `draft`
- AND la imagen original se conserva

**Puerta:** gate live `node tests/diffusion.mjs`.

#### Scenario: Metamorfosis con `init_image` y `strength`

- GIVEN una imagen existente en el ledger con `id` `i_…`
- WHEN el usuario elige metamorfosis con `strength: 0.4`
- THEN el trabajo se lanza con `init_image_id` apuntando a esa imagen, la misma semilla y el
  `strength` indicado
- AND el borrador registrado contiene `init_image_id` y `strength`

**Puerta:** gate live `node tests/diffusion.mjs`.

---

### Requirement: Los dos caminos de upscale, distinguibles

LocalMind MUST ofrecer los dos caminos de upscale, y la UI MUST **distinguirlos explícitamente**,
porque no son lo mismo:

| Camino | Ruta | Qué hace |
|---|---|---|
| **Hires fix (latente, dentro de la generación)** | parte del `draft` (`hires{}`) y se lanza por `img_gen` | **Regenera**: vuelve a pasar por el DiT con el prompt, a mayor resolución efectiva. |
| **Upscale de la imagen** | `POST /sdcpp/v1/upscale` | **No regenera** y **no toca el DiT**: escala el resultado ya generado. |

La UI MUST etiquetar cuál es cuál en el momento de la acción, para que el usuario sepa si va a
"re-generar más detalle" o a "escalar lo que ya salió". Confundirlos es el error de UX que este
requisito previene.

**El techo es ~1 megapixel**, por la atención MATH SDPA O(n²). Cualquier ampliación de ese límite es
otro cambio; la UI MUST validar el techo antes de lanzar y explicar el rechazo.

Los upscalers descubiertos en `capabilities` MUST distinguir su `kind`: `Latent` frente a
`EsrganRgb`, porque no son intercambiables.

#### Scenario: Hires fix regenera; upscale no toca el DiT

- GIVEN una imagen generada con `width: 1024, height: 1024`
- WHEN el usuario elige "hires fix" con factor 2
- THEN se lanza un trabajo `img_gen` con el `hires{}` en el borrador
- WHEN en cambio elige "upscale" sobre esa misma imagen
- THEN se lanza `POST /sdcpp/v1/upscale` y **no** se relanza el DiT
- AND la UI distinguió las dos acciones antes de confirmarlas

**Puerta:** gate live `node tests/diffusion.mjs`.

#### Scenario: El techo de ~1 megapixel se valida antes de lanzar

- GIVEN una resolución solicitada de 2 megapíxeles
- WHEN el usuario intenta generar
- THEN la petición se rechaza con la explicación de que la atención MATH SDPA es O(n²)
- AND el techo de 1.0 `mp` del preset es el que se aplica

**Puerta:** `cargo test` (validación de `max_resolution_mp`) + gate live `node tests/diffusion.mjs`.

#### Scenario: El tipo de upscaler se muestra

- GIVEN `capabilities` con un upscaler `kind: Latent` y otro `kind: EsrganRgb`
- WHEN la UI ofrece la lista de upscalers
- THEN ambos aparecen con su tipo distinguible

**Puerta:** `cargo test` (prueba 3) + gate live `node tests/diffusion.mjs`.

---

### Requirement: Encolado y cancelación sin matar el proceso

Un trabajo MUST poder encolarse y cancelarse. Cancelar un trabajo en curso MUST NOT matar el proceso
del motor, MUST NOT perder la cola, y MUST NOT afectar a los demás trabajos encolados.

La cancelación usa lo que el motor ya da: `POST /sdcpp/v1/jobs/{id}/cancel`. No hace falta inventar
un modelo de trabajos ni una cola en LocalMind. `GET /sdcpp/v1/jobs/{id}` aporta
`status`/`queue_position`/`result`, y la UI MUST reflejar `queue_position` mientras espera.

La generación encolada en un ledger MUST registrar su transición a `Cancelled` con `status`, sin
perder el `draft` ni las semillas: un trabajo cancelado sigue siendo reproducible si se relanza.

#### Scenario: Cancelar no mata el motor

- GIVEN un trabajo en curso y un segundo trabajo encolado detrás
- WHEN el usuario cancela el primero
- THEN la respuesta es correcta y el proceso de `sd-server` sigue vivo
- AND el segundo trabajo pasa a estar en curso, con su `queue_position` actualizado
- AND el ledger marca el primero como `Cancelled` conservando `draft` y `seeds`

**Puerta:** gate live `node tests/diffusion.mjs` (cancelar sin perder la cola).

#### Scenario: La cola es visible

- GIVEN dos trabajos encolados
- WHEN se consulta el estado
- THEN la UI muestra la posición de cada uno en la cola
- AND el progreso real del trabajo en curso se ve en el log/SSE, no como un spinner mudo

**Puerta:** gate live `node tests/diffusion.mjs`.

---

### Requirement: Aviso de reinicio al cambiar de preset

Cambiar de preset **debe reiniciar el proceso**: `new_sd_ctx` se construye una vez, antes de
`svr.listen`, y **no existe endpoint de recarga**. Eso MUST ser **contrato explícito y visible en la
UI antes de la acción**, no un efecto colateral: es la operación más cara y la más fácil de lanzar
por error, y recarga pesos que cuestan segundos.

Después de confirmar, el motor MUST detenerse y arrancar con el nuevo preset, y
`GET /api/diffusion/status` MUST reflejar el `preset_id` y la `vram_class` nuevos.

#### Scenario: El aviso aparece antes de la acción

- GIVEN el motor arrancado con un preset `A` y otro preset `B` declarado en la configuración
- WHEN el usuario selecciona `B`
- THEN la UI advierte **antes** de actuar de que la operación reinicia el proceso
- AND el proceso se detiene y arranca con el nuevo preset tras la confirmación

**Puerta:** `cargo test` + gate live `node tests/diffusion.mjs` + gate live `node tests/ui-tabs.mjs`.

> El catálogo embebido trae **un solo** preset de imagen de Nivel 1 (`qwen-image-2.1`), así que este
> escenario se ejercita con un segundo preset **declarado por el usuario** en
> `[generation_diffusion.preset.*]`. El requisito es del **motor**, no del catálogo: `new_sd_ctx` se
> construye una vez y no hay recarga, así que el aviso es obligatorio siempre que haya dos presets
> que elegir.

---

### Requirement: Aviso de exclusividad de VRAM en ambas pestañas

Mientras `active == "diffusion"` en el `EngineCoordinator`, la UI MUST avisar de forma persistente
en la **pestaña Chat** de que el chat está en pausa por la GPU, con un enlace para volver al
generador. Y en sentido inverso, si el LLM tomó la GPU, la **pestaña `generate`** MUST avisar de que
la generación está en pausa.

Los dos avisos son la traducción directa de la consecuencia medida: los presets del cambio son
`Exclusive`, así que generar una imagen **apaga el chat** y lanzar un agente **apaga el generador**.

La UI MUST ofrecer, cuando un lanzamiento de agente recibe `409` con `blocked_by`, el botón exacto
que resuelve el conflicto, en lugar de un error genérico.

#### Scenario: El aviso de lease es visible y accionable

- GIVEN el lease de VRAM lo tiene la difusión
- WHEN el usuario está en la pestaña Chat
- THEN la UI muestra un aviso persistente de que la GPU la está usando el generador de imágenes, con
  la acción de volver
- WHEN el usuario intenta lanzar un agente
- THEN la UI muestra el `409` con `blocked_by` y ofrece la acción que lo resuelve

**Puerta:** gate live `node tests/ui-tabs.mjs` (Chat pinta con el aviso) + gate live
`node tests/diffusion.mjs --mode surface` (rama `409` de `handle_launch_generic`).

---

### Requirement: La restricción de licencia no comercial es visible (C1)

El preset `qwen-image-2.1` MUST declarar y la UI MUST mostrar de forma **visible** su licencia **no
comercial**. Es la restricción C1 de la propuesta: no permite uso comercial, y es riesgo documentado
y aceptado por el dueño para uso personal, no pregunta abierta.

**Consecuencia de que `z-image-turbo` se haya retirado del catálogo** (decisión del dueño,
2026-09-30): `qwen-image-2.1` es el **único** preset de Nivel 1 y por tanto el **default**. La
mitigación que antes existía —"el default es un modelo permisivo, así que la ruta por defecto no
toca la licencia restringida"— **ya no es posible**. El riesgo de licencia queda **concentrado en un
único modelo**, y lo que MUST sustituirla no es un default distinto sino la **visibilidad
inequívoca**: la licencia se ve en la ficha del preset **antes** de generar, en la propia pestaña
`generate`, no en una pantalla de ajustes.

La restricción MUST quedar registrada en el `docs/` del modelo y en `docs/licencias.md` si existe.

#### Scenario: La licencia no comercial se ve antes de generar

- GIVEN el preset `qwen-image-2.1` seleccionado
- WHEN el usuario ve la ficha del preset
- THEN aparece su licencia no comercial
- AND el detalle se ve sin tener que generar nada
- AND está visible **en la propia pestaña `generate`**, no escondida en ajustes

**Puerta:** `cargo test` (datos de licencia del catálogo embebido) + gate live
`node tests/ui-tabs.mjs`.

#### Scenario: El único preset de Nivel 1 declara su licencia

- GIVEN una instalación sin selección explícita de preset
- WHEN la UI ofrece el preset por defecto
- THEN es `qwen-image-2.1`, con su licencia **no comercial** marcada
- AND la UI **no** ofrece un Tier-1 permisivo que la contravenga, porque no existe
- AND la consecuencia está documentada en `docs/models/diffusion.md`

**Puerta:** `cargo test` (preset por defecto del catálogo; **exactamente un** preset de Nivel 1) +
gate live `node tests/ui-tabs.mjs`.

---

### Requirement: La iteración solo se ejercita en vivo, y eso se declara

`strict_tdd: false` para este cambio. Las **gates offline** (`cargo test`, `ui-render.mjs`,
`bench.mjs --self-test`, `node --check`) **no cubren nada de este dominio**: no hay motor, no hay
render, no hay cola real.

El bucle de iteración completo —el requisito central del cambio— MUST ejercitarse con
`node tests/diffusion.mjs` contra la **app viva** con el motor arrancado. Un test que simule el motor
MUST NOT declararse como cobertura de la iteración: daría verde sobre comportamiento no ejercido,
que es exactamente lo que `config.yaml` §`strict_tdd_rationale` dice evitar.

#### Scenario: La iteración se verifica con la app viva

- GIVEN la app viva en `127.0.0.1:17860` con el motor de difusión arrancado
- WHEN se ejecuta `node tests/diffusion.mjs`
- THEN se ejercitan: ledger ↔ semilla, cancelación sin perder la cola, barrido con
  `max_batch_count` respetado, rehidratación y los dos caminos de upscale

**Puerta:** gate live `node tests/diffusion.mjs`. **No puede ser offline.**

#### Scenario: Un test que simula el motor no se acepta como cobertura

- GIVEN un arnés que simula `POST /sdcpp/v1/img_gen` sin motor detrás
- WHEN pasa en verde
- THEN no se considera evidencia de que la iteración funciona
- AND la puerta sigue siendo el arnés live

**Puerta:** revisión de `tasks`; el arnés live es la única prueba admisible de la iteración.

---

*Fin de la especificación `image-generation-iteration`. Fase `spec`, **revisión 2** (2026-09-30):
retirado el requisito de comparación A/B —corte 2 aprobado por el dueño—, **11 → 10 requisitos** y
**21 → 20 escenarios**; `z-image-turbo` fuera del catálogo, `qwen-image-2.1` como único Nivel 1 y
default. No se modificó código del producto. Siguiente: `tasks`.*
