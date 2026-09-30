# Especificación: `video-generation-feasibility`

Capacidad **nueva**. No existen requisitos previos: `openspec/specs/` estaba vacío antes de este
cambio.

> Fase: **spec**. Fecha: 2026-09-30. Cambio: `diffusion-generation`.
> La prosa va en español. Los identificadores, rutas, flags y nombres de fichero van en inglés.

## Purpose

Especificar el **spike medido** de generación de vídeo. Este es el Nivel 2 del cambio y su
entregable **no es una experiencia de usuario**: es un **informe con números medidos**, que
sustituye una extrapolación sin base.

El único dato que existe hoy ("8–15 min por clip") se calculó sobre el **GGUF equivocado**: el único
fichero de 6.26 GiB es `ref2va_pruned-Q2_K`, que exige imagen de referencia y **no** hace
texto-a-video. El único T2AV razonable es `fl2va_pruned-Q4_K_M` de 10.64 GiB, lo que lleva la suma
realista a ~28.25 GiB en disco. **Esa cifra es una extrapolación, no un hecho.** El entregable de
este nivel es *sustituir la extrapolación por un hecho medido*, para que el dueño decida después qué
construir.

**Un informe negativo es un entregable válido**, y esta especificación está escrita para que
"H3 no cabe" pase los mismos criterios que "H3 cabe".

---

## Requirements

### Requirement: El spike no entrega UX de vídeo

El Nivel 2 MUST NOT incluir pestaña de vídeo, panel de vídeo, previsualización, escalonado, ni
audio. LocalMind **renderiza**, no **monta**: quedan explícitamente fuera `end_image`,
`control_frames`, `_TIMELINE` y cualquier editor de vídeo.

La única superficie añadida MUST ser `POST /api/diffusion/vid_gen`, expuesta bajo
`/api/diffusion/*` como el resto, y usada por el arnés de medición, no por la UI.

`[generation_diffusion] enabled` MUST permitir apagar la generación de difusión y dejar LocalMind
exactamente como está, de modo que la reversión del spike sea un cambio de configuración y no un
`git revert`.

#### Scenario: No existe pestaña de vídeo

- GIVEN la app viva con el spike de vídeo ejecutado
- WHEN se recorre la lista de pestañas
- THEN hay 8 pestañas y ninguna es de vídeo
- AND `ui-tabs.mjs` sigue teniendo 8 entradas, no 9

**Puerta:** gate live `node tests/ui-tabs.mjs`.

#### Scenario: Apagar la generación revierte el cambio

- GIVEN `[generation_diffusion] enabled = false` en el TOML
- WHEN LocalMind arranca
- THEN no hay motor de difusión, ni rutas `/api/diffusion/*`, ni pestaña `generate`
- AND el LLM funciona exactamente igual que antes del cambio

**Puerta:** `cargo test` (carga de configuración) + gate live
`node tests/diffusion.mjs --mode surface` (comprueba el 404 de `/api/diffusion/*` con el
interruptor apagado, **sin** necesitar motor) + `node tests/e2e.mjs` (el LLM funciona igual).

---

### Requirement: Carga de MiniMax-H3 con presupuesto de memoria explícito

El spike MUST cargar `MiniMax-H3` (`minimax_h3_fl2va_pruned-Q4_K_M.gguf`, 10.64 GiB, más encoder y
`--audio-vae`) con un presupuesto de memoria explícito, incluyendo `--max-vram` negativo y
`--auto-fit`, y MUST registrar el **plan de memoria** que se pasa, no solo el que se configuró.

El preset `h3-fl2va` MUST pasar por el mismo `validate_preset` (sobre sus rutas **declaradas**, igual
que los presets de imagen) y por el **mismo filtro único de flags por backend**. En particular,
`--diffusion-fa` MUST desaparecer del `argv` antes del `spawn()`, y cualquier forma de
`int8_convrot` MUST desaparecer también: **cuelga la GPU en RDNA2**, y el hecho de que este preset no
se use en la UI no lo hace menos peligroso.

Si H3 **carga**, el spike MUST registrar que cargó. Si H3 **no** carga, el fallo MUST capturarse
**con su firma exacta** —código de salida y `stderr` literal—, no resumido como "no funcionó".

#### Scenario: H3 carga con su presupuesto de memoria

- GIVEN el preset `h3-fl2va` con todos sus componentes validados
- WHEN se arranca el motor con ese preset
- THEN el banner nombra el plan de memoria que **se pasa** (`--max-vram`, `--auto-fit`, `--audio-vae`)
- AND `GET /sdcpp/v1/capabilities` responde 200 con el JSON parseado
- AND `is_healthy` pasa a `true`

**Puerta:** `cargo test` (prueba 1, `build_sd_cmd` con el preset `h3-fl2va`) + gate live
`node tests/diffusion-bench.mjs`.

#### Scenario: Si H3 no carga, el fallo se captura con su firma

- GIVEN el motor arrancándose con `h3-fl2va`
- WHEN el proceso termina antes del handshake
- THEN la captura del arnés incluye el **código de salida** y el `stderr` literal del hijo
- AND el informe registra el fallo con esos datos exactos
- AND el estado del spike es **fallo de carga**, que es un resultado válido

**Puerta:** gate live `node tests/diffusion-bench.mjs` (salida a JSON, incluido el caso de fallo).

#### Scenario: El filtro de flags protege también al preset de vídeo

- GIVEN el comando upstream documentado de H3, que usa `--diffusion-fa`
- WHEN se construye el argv para Vulkan
- THEN `--diffusion-fa` se descarta
- AND ningún argv de este preset puede contener `int8_convrot`

**Puerta:** `cargo test` (prueba 2).

---

### Requirement: Trabajos reales de `vid_gen`, no simulaciones

Los trabajos MUST ser **reales**: `POST /sdcpp/v1/vid_gen` contra el motor vivo. Una simulación,
una estimación o un replay de la cifra anterior MUST NOT contar como medición.

El arnés MUST ejecutar **varias** combinaciones de resolución / frames / pasos, para que el informe
pueda decir cuál cabe y cuál no, en lugar de dar un único número sin contexto.

Cada trabajo MUST medirse con **reloj de pared**, no con una estimación. La matriz MUST incluir al
menos la combinación de referencia (la que la documentación recomienda) y al menos una combinación
de frames crecientes, para localizar el límite en lugar de solo confirmarlo.

#### Scenario: La matriz incluye la combinación que sí y la que no

- GIVEN H3 cargado y el arnés `node tests/diffusion-bench.mjs` ejecutándose
- WHEN completa la matriz
- THEN hay al menos un trabajo **completado** con su tiempo de pared
- AND hay al menos un intento que **no** cabe, con su firma de fallo
- AND ambos aparecen en el informe

**Puerta:** gate live `node tests/diffusion-bench.mjs`.

#### Scenario: Ningún número del informe viene de una estimación

- GIVEN el informe del spike
- WHEN se revisa cada cifra de tiempo de pared
- THEN cada una procede de un reloj de pared sobre un trabajo real
- AND no hay cifras heredadas de la estimación previa

**Puerta:** gate live `node tests/diffusion-bench.mjs` (salida JSON con reloj de pared por trabajo).

---

### Requirement: Medición de tiempo de pared y de RAM pico por trabajo

El arnés MUST medir, **por cada trabajo**, el tiempo real de pared transcurrido y el **pico de RAM**
del proceso, y MUST volcar el resultado a JSON para que el informe se construya sobre datos y no
sobre notas a mano.

La salida MUST incluir, por trabajo: la combinación de resolución / frames / pasos, el estado
terminal, el tiempo de pared, el pico de RAM, y —si falló— la firma del fallo.

El arnés MUST entrar en el gate de sintaxis: `node --check` sobre `tests/*.mjs` lo cubre junto al
resto.

#### Scenario: Cada trabajo deja su medición en JSON

- GIVEN un trabajo de `vid_gen` ejecutado por el arnés
- WHEN el trabajo alcanza su estado terminal
- THEN el JSON de salida registra su combinación de parámetros, su estado, su tiempo de pared y su
  pico de RAM
- AND el fichero pasa `node --check` junto al resto de `tests/*.mjs`

**Puerta:** gate live `node tests/diffusion-bench.mjs` + gate offline `node --check` sobre
`tests/*.mjs`.

#### Scenario: El pico de RAM se mide, no se deduce de los pesos

- GIVEN un trabajo de vídeo que falla por memoria
- WHEN se registra su medición
- THEN el informe da el **pico de RAM observado**
- AND no lo sustituye por el tamaño de los pesos en disco

**Puerta:** gate live `node tests/diffusion-bench.mjs`.

---

### Requirement: El informe sustituye explícitamente la estimación previa

El informe MUST **sustituir** la estimación vigente de "8–15 min por clip", diciendo **cuál era el
error** de esa estimación y **cuál es el dato real**. La estimación no puede quedar flotando al lado
del dato sin ser desmentida explícitamente.

El error conocido MUST quedar escrito: la estimación se calculó sobre `ref2va_pruned-Q2_K` (único
fichero de 6.26 GiB disponible), que exige imagen de referencia y **no** hace texto-a-video. El
único T2AV razonable es `fl2va_pruned-Q4_K_M` de 10.64 GiB, con una suma realista de ~28.25 GiB en
disco. La cifra de 8–15 min no se midió sobre ese modelo.

#### Scenario: La estimación queda desmentida, no olvida

- GIVEN el informe del spike
- WHEN se busca en él la mención a "8–15 min por clip"
- THEN aparece junto a la explicación de que se calculó sobre el GGUF equivocado
- AND aparece al lado el tiempo de pared **medido**
- AND la cifra medida sustituye formalmente a la estimación

**Puerta:** revisión del artefacto del informe en la fase `archive`; el dato lo produce el gate live
`node tests/diffusion-bench.mjs`.

---

### Requirement: Veredicto accionable, con informe negativo como entregable válido

El informe MUST dar un **veredicto accionable**: con estos números medidos, el objetivo de producción
razonable es **T2AV**, **Ref2AV**, o **ninguno todavía** —y qué haría falta para cada caso.

**Un informe negativo es un entregable válido.** Si H3 no cabe en 15.1 GB de RAM, el informe MUST
decirlo con la **misma claridad** con la que habría dicho que sí cabe: número medido, combinación que
lo causa, y qué haría falta. El spike MUST NOT reescribir un resultado negativo como ambiguo, ni
degradarlo a "no se pudo completar" sin la firma del fallo.

El informe MUST registrar también el hallazgo de espacio en disco si lo hubo: `models/` está en C:
con 54 GB libres y H3 pide ~28.25 GiB; si el Nivel 2 se queda sin espacio, **eso es el hallazgo** y
MUST reportarse como tal.

#### Scenario: H3 no cabe — el informe pasa igualmente

- GIVEN H3 no carga, o carga pero ningún trabajo de la matriz completa
- WHEN se revisa el informe
- THEN el veredicto dice que no cabe, con la firma exacta del fallo y la combinación que lo causa
- AND el informe dice qué haría falta para que cabiera
- AND el estado del spike es **éxito del spike**, no fracaso del cambio

**Puerta:** el informe es el entregable; el arnés live `node tests/diffusion-bench.mjs` produce los
datos, y la fase `archive` registra qué gates se ejecutaron.

#### Scenario: H3 sí cabe — el veredicto es igual de concreto

- GIVEN al menos un trabajo de `vid_gen` completado con su tiempo de pared y su RAM pico
- WHEN se revisa el informe
- THEN el veredicto distingue entre T2AV y Ref2AV según lo que la matriz haya podido ejecutar
- AND en ambos casos dice qué haría falta para el siguiente escalón

**Puerta:** gate live `node tests/diffusion-bench.mjs` + artefacto del informe.

#### Scenario: El espacio en disco insuficiente es un hallazgo

- GIVEN `models/` en una unidad sin espacio suficiente para los ~28.25 GiB de H3
- WHEN se intenta la instalación o la carga
- THEN el informe reporta el fallo de espacio con su cifra
- AND el spike lo reporta como hallazgo, no lo silencia

**Puerta:** gate live `node tests/diffusion-bench.mjs` (el arnés captura el fallo y su firma).

---

*Fin de la especificación `video-generation-feasibility`. Fase `spec`, **revisión 2** (2026-09-30):
fase **intacta** —es un spike medido y un informe negativo sigue siendo un entregable válido—, con
la redacción del filtro de flags alineada con el comportamiento único —corte 3— y la puerta de
"apagado = 404" apuntando a `node tests/diffusion.mjs --mode surface` —corte 4—. Requisitos: **6, sin
cambio**. No se modificó código del producto. Siguiente: `tasks`.*
