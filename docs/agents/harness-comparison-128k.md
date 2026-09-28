# Comparativa de harnesses a 128K (OpenCode, DeepSeek, Pi, OMP)

## Estado del documento

- Final: tabla de campaña a 128K (8 runs, 4 harnesses, máquina contenida —
  ver § "Tabla medida (contendido)"), tabla fair en quietud (§ "Tabla fair"),
  metodología, gate-vs-control, TTFT vs nivel de razonamiento, truncamiento en
  127, presión de RAM (§ "Presión"), integración por ruta de la app (§ "Integración"),
  disclaimers de contención, y curva t/s vs profundidad (§ "Curva": solo el punto
  ~512 es válido; 8k colgó el prefill ~300 s, 32k omitido por inestabilidad).
- Pendiente de una re-medición en quietud: deriva térmica/sostenida, y
  truncamiento con `finish_reason` registrado. La puerta de aceptación varía
  entre cargas (32,7 / 32,47 / 14,46 / 15,13) — ítem abierto.

Carga única compartida: perfil `libros`, contexto 131072, modelo Qwen3.8-27B-IQ4_XS.
Arranque: `POST /api/start {"profile":"libros","context":131072}` aceptado de inmediato
(`{"status":"starting","pid":4140}`); `running` + `acceptance_ok:true` a los ~60 s
(listo a las 12:21:09 tras pedirlo a las 12:20:09). El `decode_tps` visible entonces
(13,35) era el valor de la carga ANTERIOR (el status conserva el último gate; ver
§ "Gate vs control: resuelto").
Orden secuencial: pi-reacción, control, omp-reacción, omp-throughput, pi-throughput,
opencode-reacción, opencode-throughput, deepseek-reacción, deepseek-throughput.
Motor parado al final (`POST /api/stop` → `{"status":"stopped"}`).
Evidencia bruta: `tests/harness-bench/results/*.json` (8 ficheros).
Script reutilizable: `tests/harness-bench.mjs` (Node ESM, sin dependencias).

## Tabla medida (contendido: campaña original, thinking max/default)

Campaña original con la máquina contenida (workers + sesiones del dueño activos).
Definición de "¿supera 30 t/s?": `effective_tps = completion_tokens / gateway_ms`
agregado por run (INCLUYE prefill — el `ms` del gateway cubre la request completa).
`gen_tps` = decode puro del motor (`GET /api/metrics`) justo después del run.
NINGÚN harness supera 30 t/s en `effective_tps`; el control directo SÍ (42,6 t/s decode).

| Harness (versión) | Reacción 1er byte | Reacción wall / tok | Throughput wall / 1er byte | Prompt+compl (tput) | eff t/s (prefill incl.) | gen_tps (decode) | reqs | ¿>30 t/s? |
|---|---|---|---|---|---|---|---|---|
| pi 0.87.0 | 235,5 s | 235,5 s / 20747+467 (4 req) | 277,2 s / 277,2 s | 132153+2844 (19 req) | 10,3 | 10,7 | 19 | NO |
| omp 18.3.1 | 1,4 s | 335,4 s / 7732+20 (1 req) | 67,4 s / 0,6 s | 7749+1128 (1 req) | 16,9 | 18,2 | 1 | NO |
| opencode 1.18.10 | 1,9 s | 287,3 s / 12870+251 (2 req) | 386,3 s / 2,2 s | 12906+1583 (2 req) | 3,7 | 11,8 | 2 | NO |
| deepseek 0.1.5-rc.3 | 199,4 s | 199,4 s / 9869+95 (2 req) | 301,4 s / 198,1 s | 9900+1253 (2 req) | 4,1 | 11,2 | 2 | NO |
| **control directo** (gateway, stream) | TTFT 74,4 s | 131,1 s / 73+2418 (1 req) | — | 73+2418 | — (decode 42,6) | — | 1 | SÍ (decode) |

Detalle por pata (gateway `ms` → t/s por request):

- pi reacción (4 req, eff agregado 2,0): 4907+159/204973 ms (0,8 t/s), 5148+119/12486 (9,5),
  5281+118/9867 (12,0), 5411+71/6790 (10,5). Primera pata con el prefill frío de ~5k.
- omp reacción (1 req, eff 0,06): 7732+20/333813 ms. Prefill frío de 7,7k domina.
- omp throughput (1 req, eff 16,9, gen 18,2): 7749+1128/66592 ms. Respuesta 1–127, corte a
  mitad (el modelo paró en 127 de 300; salida 1092 B).
- pi throughput (19 req, eff agregado 10,3, gen 10,7): NO escribió los números a stdout;
  creó `numeros.txt` con los 300 vía herramientas y resumió en texto (stdout 219 B).
  Patas de ~5k→8,3k prompt, 60–305 completion cada una.
- opencode reacción (2 req, eff 0,8): 592+221/38941 (5,7 t/s), 12278+30/281946 (0,1).
  Segunda pata = prefill frío de 12,3k.
- opencode throughput (2 req, eff 3,7, gen 11,8): 608+394/43790 (9,0), 12298+1189/381156 (3,1).
  Respuesta 1–127, corte igual que omp.
- deepseek reacción (2 req, eff 0,5): 166+64/9831 (6,5), 9703+31/196392 (0,2). Prefill frío 9,7k.
- deepseek throughput (2 req, eff 4,1, gen 11,2): 181+64/7221 (8,9), 9719+1189/299189 (4,0).
  Respuesta 1–127, mismo corte.
- control (1 req): prompt 73, completion 2418, TTFT 74,4 s, decode 42,6 t/s, `ms` gateway 131105.


## Tabla fair (quietud, `--fair --thinking low --warmup`, orden inverso)

Ventana de integración+medición en máquina quieta (CPU media 5,7 % durante el run;
agentes del dueño presentes pero idle; una sola carga 128K sin reinicio, wall total
2661 s). Evidencia: `tests/harness-bench/results/2026-09-25T17-54-26-616Z.json`
(2026-09-25T17:54:26Z, `libros`, ctx 131072). Puerta de ESA carga: 14,46 t/s
(`acceptance_ok:true`) — ver nota sobre varianza de la puerta más abajo.
Orden: deepseek → opencode → omp → pi. Pi/omp con `--thinking low` explícito;
opencode/deepseek sin flag equivalente (defaults propios). `--warmup` = un request
mínimo con thinking off antes de cada harness (lo medido es TTFT caliente).

| Harness (versión) | Reacción 1er byte | Reacción wall / tok | Throughput wall / 1er byte | Prompt+compl (tput) | eff t/s | gen_tps | reqs | ¿>30 t/s? |
|---|---|---|---|---|---|---|---|---|
| omp 18.3.1 | 0,6 s | 331,6 s / 7732+17 (1 req) | 66,1 s / 0,6 s | 7749+1128 (1 req) | 17,3 | 18,8 | 1 | NO |
| pi 0.87.0 | 205,3 s | 205,3 s / 20498+356 (4 req) | 125,7 s / 125,7 s | 78932+1482 (14 req) | 11,8 | 15,3 | 14 | NO |
| opencode 1.18.10 | 1,8 s | **timeout 600 s, ok=false** (1 req 592+28) | 208,7 s / 2,1 s | 25180+2432 (3 req) | 2,4 | 18,3 | 3 | NO |
| deepseek 0.1.5-rc.3 | 482,1 s | 482,2 s / 9869+81 (2 req) | 544,2 s / 471,8 s | 9902+1374 (2 req) | 5,6 | 17,6 | 2 | NO |
| **control directo** (300 números, `max_tokens` 1500) | primer byte 159,4 s | último 207,7 s / 1208 tok | — | — | 5,8 (con prefill) | **25,0 decode** (1208 tok en 48,35 s puros) | 1 | NO (ni decode) |

Lectura: **ni un harness supera 30 t/s a 128K bajo ninguna de las dos definiciones,
y el techo de decode del motor en esa carga fue ~25 t/s** (puerta 14,46; `gen_tps`
de los harnesses 15–19; control decode 25,0). La salida de "reacción" de pi no fue
el OK literal sino chit-chat (run exit 0; comportamiento del agente, no del bench).
**Hallazgo abierto: la pata de reacción de opencode falló por timeout (600 s,
ok=false, 1 req 592+28 registrado)** — no es "no medido", es un fallo a investigar
(la pata de throughput del mismo harness funcionó: 3 req, 208,7 s).

Detalle por pata (fair, gateway `ms` → t/s por request):

- omp throughput (1 req, eff 17,3, gen 18,8): 7749+1128/65351 ms.
- omp reacción (1 req, eff 0,1): 7732+17/330827 ms. Mismo prefill frío que en
  contenido, reacción dominada por prefill.
- pi throughput (14 req, eff 11,8, gen 15,3): patas 4896+110/25898 (4,2) y luego
  12,9–15,7 t/s estables (p. ej. 5628+102/7025 = 14,5; 6432+118/8114 = 14,5).
  Primera pata fría, resto calientes y parejas.
- pi reacción (4 req, eff 1,7): 4896+128/185706 (0,7), 5106+80/7262 (11,0),
  5200+84/6233 (13,5), 5296+64/5509 (11,6). Misma forma: una fría + resto ~11-13.
- opencode throughput (3 req, eff 2,4, gen 18,3): 12278+24/669179 (0,0),
  608+1163/132428 (8,8), 12294+1245/204606 (6,1). Primera pata = prefill frío 12,3k.
- deepseek throughput (2 req, eff 5,6, gen 17,6): 181+64/5656 (11,3),
  9721+1310/241394 (5,4). Segunda pata = prefill frío 9,7k.
- deepseek reacción (2 req, eff 0,5): 166+64/6842 (9,4), 9703+17/168786 (0,1).
- Nota: los `gen_tps` 15–19 de esta ventana superan los 10–12 de la campaña
  contenida — la contención sí pesaba (~5 puntos de decode).

La puerta de aceptación varía entre cargas 128K: 32,7 / 32,47 / 14,46 / 15,13 en
cuatro cargas distintas — probablemente el estado del KV/VRAM al momento de la
puerta; queda como ítem abierto (no invalida las comparaciones intra-carga).

## Curva t/s vs profundidad (carga fresca 128K, vía directa)

Carga fresca `libros`/131072 (pid 15924, lista 15:41:04, ~26 s de carga; puerta
`decode_tps = 15,13`, `acceptance_ok:true`). Los puntos usan el mismo prompt base
(frase fija repetida) + sufijo fijo, `temperature: 0`, `max_tokens: 400`,
streaming por el gateway, sin harness. Sampler de 240 s en paralelo: 40 muestras
(`tests/harness-bench/results/pressure-2026-09-25T18-41-08-757Z.jsonl`).

| Punto | prompt real | cached | completion | `ms` gateway | eff (con prefill) | decode puro | `gen_tps` post |
|---|---|---|---|---|---|---|---|
| ~512 | 745 | 42 | 400 (`length`) | 39932 | 10,0 | 400 tok en 23,1 s (40,0−16,9): **17,3** | 16,0 |
| ~8k | — (sin línea: el request no terminó) | 271 | — | — | — | — | — |
| ~32k | no medido (omitido por inestabilidad, ver abajo) | — | — | — | — | — | — |

Lectura honesta: solo el punto ~512 es válido. El primer delta llegó a los 16,9 s
y todo fueron `reasoning_content` (1730 chars, sin ni un token de contenido;
`finish: length` a los 400). El punto ~8k nunca devolvió headers
(`HeadersTimeoutError` tras ~300 s de prefill; el motor quedó con
`requests_processing: 1` y el sampler lo vio al 98–121 % de compute). El 32k no se
intentó siquiera: con el 8k colgando al motor, empujar 32k habría sido forzar la
máquina. El decode caliente de ~17 t/s a 745 tokens de contexto cuadra con los
`gen_tps` 15–19 de la ventana fair (no con los 42–94 de probes de 2–300 tokens:
la completion aquí son 400 tokens de razonamiento puro, no texto corto).
Caveat frío-vs-cálido confirmado una vez más: 314 s en frío vs 325 ms en caché
para el mismo prompt de 10786 tokens (medido en la campaña anterior).

Sampler con el perfil nuevo activo: inicio **libre 6580 MB, commit 87,6 %, page
9,0 %, llama RSS 3,29 GB, CPU 1,4 %**; mitad libre 4765 MB, commit 89,1 %, compute
98,7 %, RSS 4,88 GB; final libre 4721 MB, commit 89,1 %, CPU 9,3 %, RSS 4,93 GB.
Hay varios GB libres al inicio (mejor que los ~1,4 GB sin el perfil nuevo), pero
el commit sigue ~88–89 % y el 8k cuelga igual: el perfil nuevo alivia la RAM
visible, no el costo del prefill largo a 128K.

## Metodología
- `tests/harness-bench.mjs --harnesses pi,omp,opencode,deepseek --profile libros --context 131072`:
  reutiliza la carga `running` al contexto pedido (no la reinicia), espera una carga ajena
  `starting` en vez de duplicarla, y solo arranca si está `stopped`. Clave como
  `tests/smoke.mjs` (env `LM_KEY`, si no `%APPDATA%/LocalMind/gateway.key`); nunca la imprime.
- Env/argv espejan `src-rust/src/launcher.rs` + `agents.rs` + `server.rs`:
  pi `pi --provider localmind --model localmind/localmind --thinking max -p` (+`OPENAI_*`,
  `PI_CODING_AGENT_DIR`); omp `omp --model localmind/qwen3.8-27b --thinking max -p` (+ mismo env);
  opencode `opencode --model localmind/qwen3.8-27b run` (+ XDG aislados y
  `<home>/config/opencode/opencode.json` con clave inline, como `launch_opencode` — NO por env);
  deepseek `dsh --profile headless` (+ `DSH_HOME`, telemetría off, patch Cordis generado con
  puerto/contexto vivos). Cwds scratch bajo `%TEMP%`; configs en
  `%APPDATA%/LocalMind/agents/<agent>/`.
- Primer byte con `process.hrtime.bigint()` sobre chunks de stdout/stderr (no por líneas).
- `--dry-run` imprime comandos/env exactos (clave redactada) sin tocar nada (ni red);
  `--repeats 2` repite solo runs <180 s (ignorado bajo `--fair`: siempre 1 run por fase).
  `--fair` invierte el orden, fija pi/omp a `--thinking low|off`, añade `--warmup`
  (request mínimo con thinking off antes de cada harness) y registra patas por request
- Salida no-cero si algún harness falla; en la campaña original los 8 runs salieron
  con exit 0; en la ventana fair, 7/8 (reacción de opencode: timeout 600 s, ok=false).

## Caveats honestos

- Overhead de system prompt: pi ~4,9–8,3k, omp ~7,7k, opencode ~12,3k, deepseek ~9,7k prompt
  tokens por request frente a 73 del control. Eso hunde el `effective_tps` (prefill incluido).
- Prefill frío vs cálido: la primera pata grande de cada harness paga el prefill completo
  (0,06–0,8 t/s en reacciones); las patas siguientes reutilizan prefijo (métricas finales:
  `prompt_tokens_cached` hasta 155k). Los runs tardíos van sobre KV-cache más caliente:
  efecto a favor de deepseek (último) imposible de separar con una sola carga.
- omp/opencode/deepseek cortaron la lista en "127" (3 de 3): ver § "Truncamiento en
  127": no es un cap de los harnesses (caps 4–16k frente a ~1,19k observados) ni del
  proxy (control directo con `max_tokens: 2000` llegó a 2000 tokens / 196 líneas);
  probablemente EOS del modelo en esos contextos — marcado NO PROBADO, pendiente de
  re-run en quietud con `finish_reason` registrado. Comparar "t/s" entre una lista
  truncada y 19 turnos de herramienta es aproximado: la métrica común es
  completion/gateway_ms en ambos casos.
- `effective_tps` incluye prefill; `gen_tps` es decode puro post-run. La pregunta "¿30 t/s?"
- Tiempo total de motor ≈ 35 min (235+131+335+67+277+287+386+199+301 s de runs).

## Gate vs control: resuelto

El 13,35 t/s que mostraba `/api/status` al arrancar la campaña era el gate de la
carga ANTERIOR: el status conserva el último valor hasta que la puerta de la nueva
carga termina (observación de producto; no es un bug en sí, pero conviene saberlo).
La puerta de la carga fresca 128K (`libros`, pid 19360, lista 13:11:18) midió
`decode_tps = 32,72` con `acceptance_ok:true`, consistente con los 32,47 de la bench
anterior a 128K. Se retira la hipótesis de "cold-start": la puerta (no-stream,
`Count from 1 to 60`, `max_tokens: 200`) y el control (stream, mismo prompt,
`max_tokens: 300`) midieron en caliente 32,7 y 94,6/94,7 t/s decode respectivamente
(dos probes idénticos seguidos). La diferencia gate-vs-control es el modo de medida
y el tamaño de la completion, no varianza real del motor.

## Reacción / TTFT vs nivel de razonamiento

Probes directos al gateway, prompt trivial (`Responde exactamente: OK`), motor
caliente. Se distingue TTFT al primer delta (`reasoning_content` o `content`) del
TTFT al primer token de CONTENIDO: el modelo razona (~n chars) antes de responder.

| Ajuste | TTFT 1er delta | TTFT 1er contenido | prompt+compl | reasoning | decode t/s |
|---|---|---|---|---|---|
| default (sin campo; lo que mandan los harnesses) | 1,8 / 1,0 s (2 rep) | 1,8 / 1,0 s | 57+28 | ~99 chars | n/a (2 tok) |
| `reasoning_effort: "low"` | 1,2 / 0,9 s (2 rep) | 1,2 / 0,9 s | 45+23 | ~78 chars | n/a (2 tok) |
| thinking off (`chat_template_kwargs: {"enable_thinking": false}`) | 0,5 s | 0,5 s | 17+2 | 0 chars | n/a (2 tok) |
| `reasoning_effort: "max"` | — | — | — | — | HTTP 500: la plantilla Qwen rechaza el mapeo `max` |

Lectura: `enable_thinking:false` SÍ lo honra el motor (prompt 17 vs 57, completion 2
vs 28, `reasoning_content` vacío). El `"max"` que mandan pi/omp por defecto
(`--thinking max`) el proxy hoy lo deja pasar y el motor lo rechaza con 500 —
defecto ya derivado para corrección (el proxy normalizará `max → xhigh`); pi/omp lo
sufrirían en cada request si el gateway lo devolviera en vez de reintentar (en la
campaña los runs salieron con exit 0: el stack lo absorbió aguas arriba).

## Por qué las "reacciones" de los harnesses tardaron 199–335 s

Dos sumandos, ambos medidos:

1. Prefill frío del system prompt (5–12k tokens) con la máquina contendedora: el
   punto depth-8k lo muestra en crudo — mismo prompt de 10786 tokens, `ms` 314222
   en frío frente a 325 ms en caliente con el prefijo en caché (cached 11269).
   Mil veces de diferencia por el caché de prefijo, no por el harness.
2. Fase de razonamiento antes del primer token de contenido (~500 chars en el
   control frío con TTFT-contenido 74,4 s; ~99 chars incluso en probes triviales
   calientes).

Conclusión práctica: en caliente y en quietud estos prompts responden en ~1 s (0,5 s
con thinking off). Lo que hace lenta la primera respuesta es el system prompt propio
de cada harness pagando prefill, y el thinking quita la mayor parte del resto.

## Truncamiento en 127

No es un cap de salida de los harnesses: omp `maxTokens` 16384 (`agents.rs`), opencode
`limit.output` 8192 (`launcher.rs:434`), deepseek `maxTokens` 4096 (patch) — todos muy
por encima de los ~1,19k completion observados. Tampoco es un cap del proxy: el mismo
prompt por vía directa con `max_tokens: 2000` llegó a 2000 tokens / 196 líneas
(`finish: length`, cortado a mitad del "195"), y con `max_tokens: 900` a 900 tokens.
Causa más probable: EOS del modelo en esos contextos de harness (system prompts de
5–12k que desplazan la trayectoria), pero marcado NO PROBADO: falta re-run en quietud
con ajustes idénticos y `finish_reason` registrado.

## Presión durante la ventana fair (medido, no narrativa)

Sampler de 900 s, 142 muestras (`tests/harness-bench/results/pressure-2026-09-25T17-09-30-820Z.jsonl`):
al inicio **RAM libre 1376 MB, commit 87,6 %, page-file 9,1 %, llama RSS 8,19 GB,
CPU 2,2 %**; a mitad commit 88,9 %, GPU compute (pid del motor) 99,6 %;
CPU min/media/max 1,5/5,7/16,8 %. Conclusión medida: **el jank a 128K es presión
de RAM/commit, no CPU** (el CPU casi no se usa). Detalle y palancas en
`docs/agents/engine-pressure.md`.

## Integración por ruta de la app (128K, `POST /api/launch`, uno por uno)

Ventana de integración en máquina quieta, por la ruta real de la app (no el bench
directo): los cuatro harnesses devolvieron 200 con la carga 128K viva.
`agents/pi/models.json` con `apiKey` + gateway `:17860` + ctx 131072 (pi 200, spawn
`wt pid 4776`); `agents/omp/models.yml` con apiKey/gateway/ctx (omp 200);
`agents/opencode/config/opencode/opencode.json` con
`baseURL http://127.0.0.1:17860/v1` y ctx 131072 (opencode 200);
`agents/deepseek/profiles/headless/cordis.patch.yml` con `baseURL :17860/v1` y
`contextWindow 131072` ya generado con valores vivos, no hardcodeado (deepseek 200).
Para los interactivos (pi/omp) la prueba es config+spawn+200 (la tarea se ignora por
diseño); para los one-shot (opencode/deepseek) el tráfico por el gateway quedó
probado por las patas del bench fair. Binario shippeado `db1a1b8f…` (3551232 B) con
el fix de `reasoning_effort` vivo: triple `max`/`minimal`/`medium` sobre el binario
shippeado devolvió 200 con `reasoning_content` y prompts 56/44/14 (el fix funciona).

## Contención y qué significan los números (campaña original)

Todo el follow-up se midió con sesiones propias del dueño activas (`omp`, `opencode`,
Orca) más la campaña en curso; la lectura GPU final en idle quedó <6% de uso. Las
cifras de throughput son cotas inferiores. Techo real a 128K de asignación con prompt
corto: probe caliente de 300 tokens a **94,6 t/s decode** (dos repeticiones idénticas).

## Qué significa esto para el dueño (actualizado con la ventana fair)

Para respuestas rápidas: contexto real corto, nivel de razonamiento bajo o thinking
off, y perfiles 32–64K cuando el documento lo permita. En la ventana fair (quietud,
orden inverso, thinking low) **ningún harness supera 30 t/s ni en efectivo ni en
decode**: el mejor efectivo es omp (17,3) y los decode por pata caliente rondan
11–19 t/s; el techo de decode del motor en esa carga fue ~25 t/s (control directo).
Si quiere velocidad: prompts cortos, thinking bajo/off, y aceptar que a 128K el
jank medido es RAM/commit (ver § "Presión"), no CPU.

## Protocolo de ventana libre (~10 min, copiar-pegar)

Preparado mientras la máquina está ocupada; NADA pesado ejecutado para escribirlo
(`node --check` + `--dry-run` + un sampler de 12 s contra el gateway parado).
Orden inverso (deepseek, opencode, omp, pi), `thinking` explícito `low` en pi/omp
(desvía del default `max` de la app: compara harnesses, no la config instalada),
una reacción + un throughput por harness, patas por request (cada `usage.jsonl`
con su prompt/completion/ms y su t/s de pata), más `--warmup` (un request mínimo
con thinking off antes de cada harness — lo declarado pasa a ser TTFT caliente).

```bat
node tests/harness-bench.mjs --fair --thinking low --warmup --profile libros --context 131072 --no-control
```

Ventana completa (un solo `POST /api/start`, secuencial, parar al final):

1. `POST /api/start {"profile":"libros","context":131072}` → esperar `running` +
   `acceptance_ok:true`; anotar `decode_tps` de la puerta y segundos de carga (~1 min).
2. Arrancar el sampler en paralelo: `node tests/pressure-sampler.mjs --duration 600`
   (cada 5 s: RAM libre/comprometida, page-file, CPU total, `llama-server.exe`,
   top-3 GPU engines, `gen_tps` si la app responde; JSONL a
   `tests/harness-bench/results/pressure-<ts>.jsonl`). No arranca nada por sí mismo.
3. `node tests/harness-bench.mjs --fair --thinking low --warmup --profile libros --context 131072 --no-control`
   (~8 min: 8 runs con sus patas por request).
4. Opcional, puntos de profundidad con thinking off (mismo prompt + sufijo fijo):
   ~512 / ~8k / ~32k tokens (omitir 96k si tarda >5 min o algo se ve inestable).
5. `POST /api/stop` → confirmar `stopped` y `llama-server.exe` ausente.

