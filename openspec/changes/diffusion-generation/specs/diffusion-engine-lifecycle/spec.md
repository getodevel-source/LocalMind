# Especificación: `diffusion-engine-lifecycle`

Capacidad **nueva**. No existen requisitos previos: `openspec/specs/` estaba vacío antes de este
cambio.

> Fase: **spec**. Fecha: 2026-09-30. Cambio: `diffusion-generation`.
> Store: híbrido (OpenSpec + Engram, clave `sdd/diffusion-generation/spec`).
> Entradas: `proposal.md`, `design.md`.
> La prosa va en español. Los identificadores, rutas, flags y nombres de fichero van en inglés
> porque son del runtime.

## Purpose

Definir el ciclo de vida de `sd-server` como **segundo motor de larga duración** de LocalMind:
dónde vive su binario, cómo se construye su `argv`, qué flags se le niegan, cómo se sabe que está
sano, y —la parte que no es obvia— **cómo se reparte la GPU con el LLM residente**, porque en
16 GB los presets del cambio son `Exclusive` y la exclusividad es una política, no una coincidencia.

Requisitos cubriendo: colocación del build, construcción de `argv`, handshake de salud, filtrado de
flags, exclusividad conmutada, superficie HTTP autenticada, y reversión verificable.

---

## Requirements

### Requirement: Colocación del motor en `bin/sd-cpp/`

El motor de difusión MUST vivir en el subdirectorio `bin/sd-cpp/`, separado de los binarios de
llama.cpp que ya están en `bin/`. El sistema MUST NOT aplanar el contenido del zip prebuilt de
sd.cpp en `bin/`, MUST NOT renombrar ni mover ningún binario preexistente de `bin/`, y MUST NOT
recompilar ni regenerar `bin/` desde el código fuente de este repositorio.

Todas las rutas de componentes de un preset MUST ser **absolutas**, y el `cwd` del `spawn()` MUST
ser `bin/sd-cpp/`, nunca la raíz del proyecto. `creation_flags` MUST ser `CREATE_NO_WINDOW`
(`process.rs:16`), igual que `llama-server`.

**Motivo, que es contrato y no preferencia:** Windows resuelve las DLL de un proceso hijo con
SafeDllSearchMode, y el **paso 1** —el directorio desde el que se cargó la aplicación— gana sobre
todos los demás. Medido en este repositorio: `bin/ggml-vulkan.dll` son 57 547 264 B (build de
llama.cpp) y `bin.prev-10683/ggml-vulkan.dll` son 54 365 184 B (build anterior del mismo nombre de
fichero). El zip de sd.cpp trae su propia `ggml-vulkan.dll`. Aplanar en `bin/` haría que
`sd-server.exe` cargara la build de llama.cpp contra un `stable-diffusion.dll` de 35.3 MB que
espera la suya: no es un error limpio, es un backend Vulkan ajeno. Sobrescribir
`bin/ggml-vulkan.dll` rompe `llama-server`, que es la función que hoy funciona. **No hay orden de
aplanado que funcione.**

#### Scenario: Colocación y resolución de la DLL de Vulkan

- GIVEN el árbol actual del repositorio, con `bin/ggml-vulkan.dll` de 57 547 264 B y sin
  `bin/sd-cpp/`
- WHEN se añade el contenido del zip prebuilt Vulkan de sd.cpp
- THEN los ficheros nuevos aparecen **solo** bajo `bin/sd-cpp/`
- AND `bin/ggml-vulkan.dll` conserva exactamente 57 547 264 B
- AND `llama-server.exe` sigue en `bin/` y su `argv` no cambia

**Puerta:** `cargo test` (test estructural que compara el conjunto de ficheros de `bin/` raíz antes
y después) + verificación manual del hash.

#### Scenario: Rutas relativas no pueden colarse en el argv

- GIVEN un preset resuelto con componentes bajo `models/diffusion/presets/qwen-image-2.1/`
- WHEN se construye el `argv` con `build_sd_cmd`
- THEN todas las rutas de componentes aparecen como absolutas, con el prefijo de la raíz del
  proyecto
- AND el `cwd` del proceso es `bin/sd-cpp/`

**Puerta:** `cargo test` (prueba 1 de `design.md` §8.1, sobre `build_sd_cmd`).

---

### Requirement: Construcción del `argv` de difusión

El sistema MUST construir el `argv` de `sd-server` en una única fuente pura y testeable
(`build_sd_cmd`), siguiendo el precedente de `build_engine_cmd` (`process.rs:2960`), y MUST NOT
tocar `build_engine_cmd` —la ruta de arranque del LLM no se modifica, solo se le añade una función
hermana.

El puerto por defecto MUST ser `17861` (`default_sd_port()` en `config.rs`, al lado de
`default_http_port()` = 17860 y `default_llama_port()` = 8080), resuelto con `find_free_port`
(`process.rs:897`, preferred + 0..50 intentos). El puerto `1234`, default público de sd.cpp, MUST
NOT usarse nunca: es el primer puerto que un escáner de `127.0.0.1` prueba.

El `argv` MUST incluir `-l 127.0.0.1` explícito, aunque sea el default upstream, y MUST NOT
contener jamás `0.0.0.0`. El arranque MUST pasar `--max-vram` negativo (default `-2`, es decir VRAM
libre − 2 GiB) y `--auto-fit`.

#### Scenario: `argv` completo con loopback explícito

- GIVEN un preset listo y el puerto preferido 17861 libre
- WHEN `build_sd_cmd` resuelve el comando
- THEN el programa es `<base>/bin/sd-cpp/sd-server.exe`
- AND el `argv` contiene `--diffusion-model`, `--vae` y `--llm` con rutas absolutas
- AND el `argv` contiene `-l 127.0.0.1`
- AND el `argv` contiene `--max-vram -2` y `--auto-fit`
- AND el puerto resuelto no es 1234

**Puerta:** `cargo test` (prueba 1 de `design.md` §8.1).

#### Scenario: El puerto upstream por defecto está excluido por construcción

- GIVEN `find_free_port` devuelve el puerto preferido cuando está libre
- WHEN se resuelve el puerto de difusión
- THEN el valor devuelto nunca es 1234
- AND cuando 17861 está ocupado se prueban 17862, 17863, … hasta +50 intentos

**Puerta:** `cargo test` (prueba 8 de `design.md` §8.1).

---

### Requirement: Handshake de salud por `capabilities`

El estado de salud del motor de difusión MUST determinarse con el parseo de
`GET /sdcpp/v1/capabilities` en el puerto local, sondeado cada **250 ms** tras el `spawn()` hasta
un techo derivado del tamaño del bundle (estilo `eta_secs`, `process.rs:1855`).

`is_healthy` MUST ser `true` **solo** cuando el JSON parseó correctamente. "Puerto abierto = sano"
MUST NOT aceptarse como prueba de salud, a diferencia del `/health` del LLM. El parseo MUST ser
tolerante: nunca debe panicar con `{}`, con claves desconocidas o con `limits` ausente, y un campo
ausente MUST degradar la UI, nunca impedir el arranque.

El `stderr` del hijo MUST fluir por el mecanismo ya existente (`attach_reader_threads`,
`process.rs:804`) hacia `/api/events` (SSE) y el log, de modo que el usuario vea progreso real
durante la carga de pesos.

#### Scenario: Puerto abierto pero sin parseo no es sano

- GIVEN el `spawn()` de `sd-server` responde en `/sdcpp/v1/capabilities` con un cuerpo que no es el
  JSON esperado
- WHEN termina el sondeo
- THEN `is_healthy` sigue en `false`
- AND el `stderr` del hijo está disponible en `/api/logs` y en el stream SSE

**Puerta:** `cargo test` (prueba 3, `parse_capabilities` con `{}`, cuerpo completo, claves
desconocidas y `limits` ausente) + gate live `tests/diffusion.mjs`.

#### Scenario: Handshake correcto activa la salud

- GIVEN `sd-server` levantándose con un preset válido
- WHEN `GET /sdcpp/v1/capabilities` responde 200 con `samplers`, `schedulers` y `limits`
- THEN `is_healthy` pasa a `true` con el JSON parseado
- AND `GET /api/diffusion/capabilities` devuelve esos mismos valores

**Puerta:** gate live `node tests/diffusion.mjs` (app viva en 127.0.0.1:17860).

---

### Requirement: Filtro de flags por backend: una lista, un comportamiento

El sistema MUST filtrar el `argv` de difusión por capacidad del backend activo **antes** del
`spawn()`, siguiendo el precedente de `psu_unsafe_flag` (`config.rs:306`, invocado en `start()` en
`process.rs:1448`). La detección MUST ser por **subcadena** sobre todo el `argv`, no por lista
exacta de flags, para capturar `-fa on`, `--diffusion-fa`, `--model-args key=…int8_convrot…`, las
**dos grafías** del término que documenta la exploración, y nombres de fichero como
`qwen_image_2.1_int8_convrot.safetensors`.

**Una sola lista, un solo comportamiento:** todo token del `argv` que contenga cualquier subcadena
de la lista MUST **eliminarse** y quedar **registrado una vez**. No hay dos severidades, ni rechazo
condicional, ni doble registro. La lista es:

| Subcadena | Por qué está en la lista |
|---|---|
| `int8_convrot`, `int8-convrot` (las **dos** grafías) | **Cuelga la GPU en RDNA2**: necesita WMMA/MFMA. Es la razón por la que la lista existe. |
| `--diffusion-fa`, `--flash-attn`, `-fa` | Vulkan no soporta Flash Attention (solo cpu, cuda/rocm, metal). Quitarlas **es** el comportamiento deseado, y un preset cuyo comando de documentación upstream las incluye debe arrancar igual. |
| `qwen_image_2_1_prefix_cache_type=f32` / `=f16` / `=auto` | Sin FA, `auto` cae a **FP32**: ~4 GiB por condición × 2 ≈ **8 GiB** de RAM en un equipo de **15.1 GB**. Sin quitarlo, "q8_0 por defecto" sería una sugerencia que un `extra_flags` mal puesto deshace. |

**Cómo sobrevive la invariante de `q8_0` con un único comportamiento.** Si el token `=f32` se
eliminara y nada más, el motor caería a `auto` → FP32 → ~8 GiB → OOM, que es **peor** que el
original. La solución es el **orden**, y es contrato:

```
construir argv  ->  filter_denied_flags(argv, backend)  ->  aplicar memory_args INMUTABLES  ->  spawn()
```

Los `memory_args` del preset se aplican **después** del filtro y no son sobrescribibles, de modo que
el `argv` final lleva `qwen_image_2_1_prefix_cache_type=q8_0` **exactamente una vez**, se haya
inyectado lo que se haya inyectado.

**Defensa a nivel de catálogo:** ningún preset MUST declarar una ruta ni una flag que contenga una
subcadena de la lista. Esto se verifica sobre el catálogo embebido **completo**, como test: es una
invariante de los datos, no una comprobación en tiempo de ejecución.

#### Scenario: Flash Attention se descarta antes de arrancar

- GIVEN un preset cuyo comando upstream documentado incluye `--diffusion-fa`
- AND el backend activo es Vulkan
- WHEN se aplica `filter_denied_flags`
- THEN `--diffusion-fa` no aparece en el `argv` resultante
- AND el resto del `argv` queda intacto
- AND el descarte queda registrado **una** vez

**Puerta:** `cargo test` (prueba 2 de `design.md` §8.1).

#### Scenario: `int8_convrot` nunca llega al `spawn()`

- GIVEN un `argv` que contiene `int8_convrot`, en cualquiera de sus dos grafías, incluso dentro de
  `--model-args` o como nombre de fichero de componente
- WHEN se aplica `filter_denied_flags` con backend Vulkan
- THEN el `argv` resultante no contiene ninguna de las dos grafías
- AND **no** se ejecuta ningún `spawn()` con ese `argv`

**Puerta:** `cargo test` (prueba 2 de `design.md` §8.1).

#### Scenario: Ningún preset declara un componente `int8*`

- GIVEN el catálogo embebido de presets
- WHEN se recorre **entero**, presets y componentes
- THEN ninguna ruta ni flag contiene `int8_convrot` ni `int8-convrot`
- AND ninguna contiene `--diffusion-fa`

**Puerta:** `cargo test` (invariante del catálogo).

---

### Requirement: `qwen_image_2_1_prefix_cache_type=q8_0` como invariante

El preset `qwen-image-2.1` MUST llevar `memory_args = ["qwen_image_2_1_prefix_cache_type=q8_0"]`
como valor **inmutable** del preset, aplicado al `argv`. El valor por defecto de LocalMind MUST
ser `false` solo en su argumento `enabled` de rollout, nunca para este `memory_args`.

Sin Flash Attention, `auto` cae a **FP32**: ~4 GiB por condición × 2 condiciones ≈ **8 GiB de RAM**
en un equipo de **15.1 GB**. `q8_0` son ~1.06 GiB. Por eso `q8_0` no es un ajuste opcional: los
`memory_args` del preset son **inmutables** y se aplican **después** del filtro de flags (§Filtro de
flags), de modo que un `argv` —de preset, de `extra_flags` o de override de perfil— que fije `f32`,
`f16` o `auto` ve ese token eliminado y el `q8_0` del preset vuelve a estar en su sitio. **Con el
orden fijado, es una invariante.**

La UI MUST exponer el porqué, no solo el número: `memory: { prefix_cache_type: "q8_0",
approx_gib: 1.06, reason: "Without flash attention, the `auto` value falls back to f32 (~4 GiB per
condition × 2)" }`. El banner de arranque MUST nombrar el argumento que **se pasa**, no el que se
configuró (lección de `process.rs:849-854`).

#### Scenario: Un `extra_flags` mal puesto no deshace la invariante

- GIVEN el preset `qwen-image-2.1` con `memory_args` en `q8_0`
- WHEN un perfil inyecta `extra_flags` con `qwen_image_2_1_prefix_cache_type=f32`
- AND se aplica el filtro y después los `memory_args` inmutables
- THEN el `argv` final no contiene `=f32`
- AND contiene `qwen_image_2_1_prefix_cache_type=q8_0` **exactamente una vez**

**Puerta:** `cargo test` (prueba 2, caso de `f32`/`f16`/`auto` + orden filtro→`memory_args`).

#### Scenario: Qwen-Image-2.1 arranca sin OOM en 15.1 GB

- GIVEN el preset `qwen-image-2.1` con todos sus componentes validados
- WHEN se arranca el motor y se genera una imagen a la resolución máxima del preset
- THEN el proceso no termina por falta de memoria
- AND el banner de arranque nombra `qwen_image_2_1_prefix_cache_type=q8_0`

**Puerta:** gate live `node tests/diffusion.mjs`.

---

### Requirement: Exclusividad conmutada de la VRAM mediante `EngineCoordinator`

LocalMind MUST coordinar sus dos motores con un `EngineCoordinator` que posee un *lease* de VRAM
con exactamente un titular, en uno de tres estados: `llm_resident`, `diffusion_resident`,
`reloading`. El estado del lease MUST ser explícito y consultable; no MUST NOT ser inferido.

**La regla es una sola y simétrica:** la VRAM se concede a quien la pide, y **solo se le quita a
quien está inactivo**.

- **Adquisición**: `POST /api/diffusion/start` pide el lease para difusión; `POST /api/launch` (los
  cinco agentes de `AGENT_ORDER = ["pi", "omp", "opencode", "web", "deepseek"]`) lo pide para el
  LLM. Ningún agente queda fuera de la guarda.
- **Cesión**: si el titular está **ocupado**, la petición se rechaza con `409` y un campo
  `blocked_by` que nombra el estado del otro motor, para que la UI ofrezca el botón exacto que lo
  resuelve. Si está **inactivo**, la cesión es automática y se registra.
- **Ocupado** se determina con el precedente que ya existe: `check_slots_busy(port)`
  (`process.rs:865`), que consulta `/slots` con la clave del gateway y mira `is_processing`,
  combinado con la ventana de actividad reciente (`last_activity`, alimentado por `touch_activity()`
  desde `server.rs:1828`).

El sistema MUST NOT matar un motor en mitad de un turno de agente. Una sesión de `pi`, `omp`,
`opencode` o `deepseek` con una conexión TCP abierta al gateway que recibe un `502 engine_down` es
daño irreversible: la cesión solo procede ante inactividad, nunca ante ocupación.

El coordinador MUST ser testeable sin matar procesos: las dependencias (`llm_busy`, `stop_llm`,
`start_llm`, `stop_sd`) MUST inyectarse por *closure*, siguiendo el criterio ya escrito en
`new_with_log_file` (`process.rs:176-182`).

#### Scenario: Cesión automática ante inactividad

- GIVEN el lease de VRAM lo tiene el LLM y `check_slots_busy` devuelve `false` y no hay actividad
  reciente
- WHEN se recibe `POST /api/diffusion/start`
- THEN el LLM se detiene
- AND el lease pasa a `diffusion_resident` con estado visible `reloading`
- AND la cesión queda registrada en el log

**Puerta:** `cargo test` (coordinador con dependencias inyectadas).

#### Scenario: Cesión rechazada ante ocupación

- GIVEN el lease de VRAM lo tiene el LLM y `check_slots_busy` devuelve `true` (un slot
  `is_processing`)
- WHEN se recibe `POST /api/diffusion/start`
- THEN la respuesta es `409`
- AND el cuerpo lleva `blocked_by` con el estado del LLM
- AND el motor LLM **no** se detiene

**Puerta:** `cargo test` (coordinador con dependencias inyectadas) + gate live
`node tests/diffusion.mjs --mode surface`.

#### Scenario: Ningún agente queda fuera de la guarda

- GIVEN el lease de VRAM lo tiene la difusión
- WHEN se lanza cualquiera de los cinco agentes de `AGENT_ORDER`, incluido `web`
- THEN la respuesta es `409` con `blocked_by: diffusion`
- AND no se lanza ningún proceso de agente

**Puerta:** `cargo test` + gate live `node tests/diffusion.mjs --mode surface`.

#### Scenario: Un turno de agente en curso no se interrumpe nunca

- GIVEN el lease de VRAM lo tiene el LLM con un turno de agente en curso
- WHEN se pide el lease para difusión con `VramPolicy::Auto`
- THEN la petición se rechaza con `409`
- AND el proceso del agente sigue vivo y su conexión al gateway intacta

**Puerta:** `cargo test` (coordinador con `llm_busy` que devuelve `true`).

---

### Requirement: Restitución del LLM por `mgr.start()`, con los guardarraíles de energía

Cuando el motor de difusión se detiene —a mano, o por su propio auto-stop por inactividad
(`sd_idle_timeout_secs`, default 900 s sin trabajos—), el coordinador MUST volver a arrancar el LLM
con `cfg.last.model` + `cfg.last.profile` + `cfg.last.context`, que es la precedencia que `start()`
ya implementa (`process.rs:1339-1363`).

La restitución MUST llamar a `mgr.start()` y por tanto MUST pasar por los guardarraíles de energía
(`start_cooldown_secs` = 120, `max_starts_per_hour` = 4, `check_start_guard`, `process.rs:1296`).
Si el guardarraíl bloquea, el LLM **se queda apagado** y la UI MUST decirlo con el número de
segundos que faltan. No se abre una puerta trasera para el LLM: si el hardware dice que no, es que
no.

El ciclo stop/start de difusión seguido de restart del LLM MUST contar contra el tope de 4
arranques/hora. Esto MUST avisarse al usuario, porque el diseño no gasta menos energía: la hace
**visible**.

#### Scenario: Restitución normal tras parar difusión

- GIVEN el lease de VRAM lo tiene la difusión y el LLM tiene `cfg.last` poblado
- WHEN se ejecuta `POST /api/diffusion/stop`
- THEN el LLM arranca vía `mgr.start()` con modelo, perfil y contexto de `cfg.last`
- AND la UI recibe el estado `reloading` durante la recarga

**Puerta:** `cargo test` (coordinador con `start_llm` inyectado) + gate live
`node tests/diffusion.mjs --mode surface`.

#### Scenario: El guardarraíl de energía no se esquiva

- GIVEN el guardarraíl de guardas de arranque bloquea (cooldown activo o 4 arranques/hora
  alcanzados)
- WHEN la difusión se detiene y el coordinador intenta restituir el LLM
- THEN el LLM **no** arranca
- AND el estado devuelto es `Guardrail` con el motivo y los segundos restantes
- AND la UI muestra el motivo

**Puerta:** `cargo test` (coordinador con `start_llm` inyectado que devuelve `Err`).

---

### Requirement: Correctitud de `VramClass` y su regla de promoción

`DiffusionPreset.vram_class` MUST reflejar la **medición**, no la suposición. Los presets del cambio
son `Exclusive` hoy, y esa consecuencia MUST declararse sin adornos: **generar una imagen desactiva
el chat, y lanzar un agente desactiva la generación.** No es un descuido; es lo que dicen los picos
contra los ~6 GB de LLM residente en 16 GB de VRAM.

**El objetivo de medición es `qwen-image-2.1`, ~13 GB de pico declarado.** Antes lo era
`z-image-turbo` (~9 GB), que era el preset **pequeño**; el dueño lo retiró del catálogo, así que el
número que falta es ahora el del modelo **grande**. **Eso hace la coexistencia más difícil, no
menos**: ~13 GB contra ~6 GB de LLM residente deja poco margen, y el resultado esperado de la
medición es `Exclusive`. La regla de promoción se conserva **por mecanismo**, no por expectativa, y
la **alternancia** —no la convivencia— es lo que este cambio entrega.

`GET /api/diffusion/presets` MUST exponer `vram_class` por preset para que la UI pueda anunciar el
coste **antes** de la operación.

**Regla de promoción, medible y no negociable:** si el pico **medido** de VRAM de `qwen-image-2.1`
en esta máquina es **≤ 6 GB**, su `VramClass` MUST bajar a `Shared` y MUST poder coexistir con el
LLM residente. Mientras no se mida, el valor declarado del preset MUST permanecer en `Exclusive` y
el sistema MUST comportarse en consecuencia.

#### Scenario: Con los presets `Exclusive`, generar apaga el chat

- GIVEN `qwen-image-2.1` con `vram_class: Exclusive` (valor vigente hasta que exista la medición)
- WHEN se arranca el motor de difusión
- THEN el LLM queda detenido
- AND la pestaña Chat muestra el aviso persistente de que la GPU la está usando el generador de
  imágenes
- AND `GET /api/diffusion/presets` declara `vram_class: "Exclusive"` para ese preset

**Puerta:** `cargo test` (clase del preset embebido) + gate live `node tests/ui-tabs.mjs` (la
pestaña Chat pinta con el aviso) + gate live `node tests/diffusion.mjs`.

#### Scenario: La promoción a `Shared` depende de una medición

- GIVEN el pico de VRAM de `qwen-image-2.1` **medido** en esta máquina
- WHEN ese pico es ≤ 6 GB
- THEN `vram_class` del preset MUST actualizarse a `Shared`
- AND el motor MUST poder arrancar con el LLM residente sin detenerlo
- WHEN ese pico es > 6 GB
- THEN `vram_class` MUST permanecer en `Exclusive` y la cesión sigue siendo automática

**Puerta:** gate live `node tests/diffusion-vram-probe.mjs` (lee el pico del proceso `sd-server`; el
arnés lo construye el Nivel 1) + `cargo test` sobre el valor de la clase. **Nota:** hasta que exista
esa medición, este escenario **no** puede declararse cumplido; su estado es *pendiente de medición*,
y el valor por defecto sigue siendo `Exclusive`. **Con `qwen-image-2.1` como único Nivel 1, el
resultado esperado es > 6 GB.**

---

### Requirement: Superficie HTTP autenticada bajo `/api/diffusion/*`, con denylist explícita

Toda la generación MUST exponerse **únicamente** bajo el prefijo `/api/diffusion/`, resuelto por
ramas explícitas en `handle_request` **después** del chequeo de autenticación de `server.rs:532` y
**antes** del `404` final. Así las rutas caen automáticamente bajo la clave local (D-7) sin abrir un
segundo camino de autenticación.

Rutas de proxy al motor: `GET /api/diffusion/capabilities`, `POST /api/diffusion/img_gen`,
`POST /api/diffusion/upscale`, `GET /api/diffusion/jobs/{id}`,
`POST /api/diffusion/jobs/{id}/cancel`, `POST /api/diffusion/vid_gen` (solo Nivel 2).
Rutas de LocalMind: `POST /api/diffusion/start`, `POST /api/diffusion/stop`,
`GET /api/diffusion/status`, `GET /api/diffusion/presets`, `GET /api/diffusion/image/{id}`,
`GET /api/diffusion/ledger`.

**La denylist es una decisión, no una omisión.** De las tres familias que expone `sd-server`
(`examples/server/routes.h`), LocalMind MUST NOT reenviar:

- `/v1/images/generations` y `/v1/images/edits` (familia OpenAI de sd-server). Chocarían
  conceptualmente con la superficie `/v1/*` publicada de LocalMind, que consumen CLIs externos con
  la clave del gateway. Reenviarlas **publicaría** generación a cualquier CLI con la clave.
- `/sdapi/v1/txt2img`, `/sdapi/v1/img2img`, `/sdapi/v1/upscalers`,
  `/sdapi/v1/latent-upscale-modes` (familia A1111). Compatibilidad WebUI sin consumidor.

La generación MUST NOT añadirse a la API pública `/v1/*` de LocalMind ni a `/sdapi/v1/*`: eso es
una decisión posterior del dueño, no de este cambio. La denylist convierte esa prohibición en una
**propiedad del código**, no en una promesa.

`GET /api/models` y `POST /api/start` MUST conservar su forma actual para LLM. Cualquier cambio de
forma en un contrato publicado que consumen CLIs externos es una ruptura y requiere llamada
explícita; aquí no hay ninguna.

#### Scenario: La generación no aparece en la superficie publicada

- GIVEN la app viva con el motor de difusión arrancado
- WHEN se solicita `POST /v1/images/generations` contra LocalMind
- THEN la respuesta **no** alcanza el motor de difusión
- AND se rechaza por la denylist o por no existir en el router
- WHEN se solicita `POST /api/diffusion/img_gen` con la cookie `lm_key` válida
- THEN la petición llega a `sd-server` y devuelve el `id` del trabajo

**Puerta:** gate live `node tests/diffusion.mjs` — la **sección de superficie** cubre la denylist (no
alcanza el motor) y la **sección de iteración** cubre que la ruta legítima sí llega a `sd-server` y
devuelve el `id` del trabajo.

#### Scenario: Sin credencial, `/api/diffusion/*` no hace nada

- GIVEN la app viva
- WHEN se solicita `GET /api/diffusion/status` sin `Authorization`, sin `x-api-key` y sin cookie
- THEN la respuesta es `401`
- AND no se consulta ni se reenvía nada al motor

**Puerta:** gate live `node tests/diffusion.mjs --mode surface` (no necesita motor: el 401 se decide
antes de tocarlo).

#### Scenario: La cookie de sesión no viaja cross-site

- GIVEN la cookie `lm_key` emitida por `set_cookie_value` (`auth.rs:78-80`) con
  `HttpOnly; SameSite=Strict`
- WHEN una página hostil en un origen no-loopback intenta usar `GET /api/diffusion/img_gen`
- THEN el navegador **no** adjunta `lm_key` a la petición cross-site
- AND `/api/diffusion/*` **nunca** emite `Access-Control-Allow-Credentials`, a diferencia de
  `sd-server`

**Puerta:** `cargo test` (el valor de la cookie contiene `SameSite=Strict`; la ausencia de
`Allow-Credentials` en las respuestas de difusión) + gate live `tests/diffusion.mjs`.

#### Scenario: Un `Origin` no-loopback no recibe ACAO

- GIVEN una petición a `/api/diffusion/*` con `Origin: https://evil.example`
- WHEN LocalMind responde
- THEN la respuesta no lleva `Access-Control-Allow-Origin`, porque `cors_origin_header`
  (`server.rs:51`) con `is_loopback_origin` (`meta.rs:177`) solo refleja orígenes loopback

**Puerta:** `cargo test` + gate live `node tests/diffusion.mjs --mode surface`.

#### Scenario: Auto-stop: la mayor parte del día no hay nada escuchando

- GIVEN el motor de difusión arrancado y sin trabajos
- WHEN transcurre `sd_idle_timeout_secs` (default 900 s)
- THEN el motor se detiene solo
- AND el LLM se restituye según la regla de guardarraíles

**Puerta:** `cargo test` (lógica de inactividad del coordinador) + gate live `tests/diffusion.mjs`.

---

### Requirement: Reversión verificable por manifiesto de hashes

Como `bin/` **no está versionado** (`.gitignore:13` contiene `bin/`, `.gitignore:10` contiene
`models/`, y `git ls-files bin` devuelve **0 ficheros**), `git status` **no muestra nada** de `bin/`
ni antes ni después del cambio. **Green por ausencia: no es evidencia de nada.** La invariante "los
binarios de llama.cpp no se tocan" MUST verificarse por otro medio.

**Antes** de añadir `bin/sd-cpp/`, el sistema MUST capturar un manifiesto **SHA256 de los ficheros
preexistentes** de `bin/` (medido: **71 ficheros**, 0 subdirectorios) en
`%APPDATA%\LocalMind\bin-manifest.json`. Una verificación posterior MUST re-hashear y diffear. Ese
manifiesto es la prueba real.

Además, como garantía estructural, un test MUST verificar que **exactamente una** entrada nueva
existe en `bin/` y que `bin/sd-cpp/` **no** contiene ningún nombre de fichero que ya exista en
`bin/` raíz con tamaño distinto. No es una comprobación de "no toqué nada": es que **el diseño hace
que no se pueda tocar nada**.

Ningún fichero bajo `models/` existente MUST escribirse, moverse ni borrarse: `models/` es dato de
usuario. Solo se crea `models/diffusion/`.

#### Scenario: `git status` no es evidencia; el manifiesto sí

- GIVEN el cambio aplicado con `bin/sd-cpp/` añadido
- WHEN `git status --porcelain` no muestra ninguna línea sobre `bin/`
- AND se re-hashea la superficie de `bin/` y se compara con
  `%APPDATA%\LocalMind\bin-manifest.json`
- THEN las 71 entradas de fichero del manifiesto coinciden en hash
- AND el único conjunto nuevo es `bin/sd-cpp/`

**Puerta:** script de verificación del manifiesto (parte de las puertas de `verify`) + test
estructural en `cargo test`.

#### Scenario: La colisión de nombres es imposible por construcción

- GIVEN `bin/` raíz con sus ficheros preexistentes
- AND `bin/sd-cpp/` con el contenido del zip de sd.cpp
- WHEN se comparan los nombres de fichero de ambos conjuntos
- THEN no existe un nombre presente en ambos con **tamaño distinto**
- AND el conjunto de ficheros de `bin/` raíz es idéntico al previo

**Puerta:** `cargo test` (test estructural de colocación).

#### Scenario: `models/` es dato de usuario

- GIVEN `models/` con el modelo LLM y `mmproj-BF16.gguf` (931 146 432 B medidos)
- WHEN se añade el layout de difusión
- THEN solo se crea `models/diffusion/` y sus subdirectorios
- AND ningún fichero existente cambia de ruta, de tamaño ni de contenido

**Puerta:** `cargo test` (prueba 6 de `design.md` §8.1, sobre árbol de fixture).

---

### Requirement: Ciclo de vida completo y aviso de reinicio por cambio de preset

El motor MUST arrancar, parar y reiniciar limpiamente, con estado observable desde la UI. El
cambio de preset **debe reiniciar el proceso**: `new_sd_ctx` se construye una vez, antes de
`svr.listen`, y **no existe endpoint de recarga**. Eso MUST ser **contrato explícito y visible en
la UI**, no un efecto colateral: es la operación más cara y la más fácil de lanzar por error.

Durante la recarga, la UI MUST reutilizar **sin cambios** los campos que ya sabe pintar
(`starting_for_secs`, `eta_secs`, `verifying`, `last_error` de `ServerStatus`,
`process.rs:26-62`). No se inventa UX de progreso: se reutiliza la que ya funciona.

#### Scenario: Cambiar de preset reinicia el proceso, avisando antes

- GIVEN el motor de difusión arrancado con un preset declarado `A`
- AND otro preset `B` declarado en `[generation_diffusion.preset.*]`
- WHEN el usuario selecciona `B` en la UI
- THEN la UI avisa **antes** de actuar de que la operación reinicia el proceso
- AND tras confirmar, el proceso se detiene y arranca con el nuevo preset
- AND `GET /api/diffusion/status` refleja el nuevo `preset_id` y la nueva `vram_class`

**Puerta:** `cargo test` + gate live `node tests/diffusion.mjs` + gate live `node tests/ui-tabs.mjs`.

#### Scenario: El progreso de carga se ve real

- GIVEN el motor de difusión arrancándose con un bundle grande
- WHEN el hijo escribe en `stderr`
- THEN esas líneas llegan a `/api/logs` y al stream SSE `/api/events`
- AND `GET /api/diffusion/status` reporta `starting_for_secs` y `verifying`

**Puerta:** gate live `node tests/diffusion.mjs`.

#### Scenario: La superficie LLM publicada no cambia

- GIVEN la app viva con el motor de difusión añadido
- WHEN se llama `GET /api/models` y `POST /api/start` con la forma actual de siempre
- THEN las respuestas conservan su forma anterior
- AND los CLIs externos que consumen el gateway siguen funcionando sin cambios

**Puerta:** gate live `node tests/e2e.mjs` (la superficie LLM publicada, cliente externo real) +
gate live `node tests/diffusion.mjs` (la superficie de difusión).

---

*Fin de la especificación `diffusion-engine-lifecycle`. Fase `spec`, **revisión 2** (2026-09-30):
filtro de flags de **una lista y un comportamiento** en lugar de dos severidades —corte 3 aprobado
por el dueño, con la invariante de `q8_0` garantizada por el **orden** filtro → `memory_args`
inmutables—, `z-image-turbo` fuera del catálogo con `qwen-image-2.1` como objetivo de medición, y las
puertas que nombraban el test de contrato HTTP apuntando a `node tests/diffusion.mjs` —corte 4—.
El recuento de requisitos **no cambia (11)**. No se modificó código del producto. Siguiente:
`tasks`.*
