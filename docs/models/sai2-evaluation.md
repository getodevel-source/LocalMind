# Evaluación "Sai 2" — identificación, factibilidad y veredicto 512K

Fecha: 2026-09-28. Máquina: RX 6800 XT 16 GB (512 GB/s), 15,5 GB RAM.
Modelo actual: `Qwen3.8-27B-IQ4_XS_4BPW.gguf` (13 905 649 312 B ≈ 12,95 GiB).
Motor: `bin/llama-server.exe` build 10683 (commit d8f26eec7, 2026-09-09); app r14
con recovery MTP dirigido (reintento sin `--spec-*` ante firma exacta).
Estado de la máquina al cierre de este informe: app `stopped`, sin `llama-server.exe`,
GUI viva. `localmind.toml` intacto (sin cambios manuales en este turno).

## PASO 1 — Identificación: no existe ningún modelo "Sai 2"

Búsquedas realizadas (web + API de Hugging Face):

- `"Sai 2" LLM model parameters` → sin ningún LLM con ese nombre. El único
  "SAI 2" conocido es **Systemax PaintTool SAI Ver. 2** (software de dibujo, no un modelo).
- `GET https://huggingface.co/api/models?search=sai2` → solo ruido
  (`sai2002/symtoms`, un LoRA de gatos ASCII). Nada con ese nombre.
- `"Sai2" OR "Sai-2" language model GGUF` → el único hit real es **Saiga 2**
  ("Sai-ga 2", serie rusa sobre Llama-2 de Ilya Gusev: 7B/13B/70B, contexto 4K,
  arquitectura de 2023). Coincidencia de nombre parcial, nada más.

Lectura de "27 millones de parámetros": literalmente 27M no corresponde a ningún
LLM conversacional usable; interpretado como **2,7B**, sí hay un match exacto:
**Microsoft Phi-2 (2,7B)**. Los 2–3 candidatos reales más cercanos son:

| Candidato | Por qué se parece a lo oído | Parámetros | Contexto nativo | Licencia | GGUF | Veredicto previo |
|---|---|---|---|---|---|---|
| **Liquid AI LFM2.5-2.6B** | ~2,6B ("dos coma seis", próximo a lo descrito) | ≈2,6B (2,69B reportados) | **131 072 (128K)** | LFM Open License v1.0 (gratis comercial si facturación < 10 M USD; si no, licencia Enterprise) | **Oficial**: `LiquidAI/LFM2.5-2.6B-GGUF` (Q4_0, Q4_K_M, Q5_K_M, Q6_K, Q8_0, BF16, F16) | **Recomendado para probar** |
| Microsoft Phi-2 | **2,7B exactos** | 2,7B | **2048 (2K)** | MIT | Comunitario (`TheBloke/phi-2-GGUF`, Q4_K_M ≈ 1,6 GB) | Descartado: 2K nativo no sirve a esta app (32K–262K) |
| Saiga 2 (IlyaGusev) | Nombre literal "Sai(-ga) 2" | 7B / 13B / 70B | 4096 (4K) | Llama 2 Community | Sí (`IlyaGusev/saiga2_7b_gguf`, etc.) | Descartado: viejo (Llama-2, 2023), 4K, el 7B ya es ~4 GB sin ventaja |
| Sailor2 (`sail/…`) | Nombre "Sai(-lor) 2" | 1B / 8B / 20B | 32K (heredado Qwen2.5) | Apache-2.0 (vía Qwen2.5) | Solo comunitario (TensorBlock e.o.) | Suplente: el 1B podría, pero 1B rinde poco como agente |

Conclusión del Paso 1: lo único que encaja a la vez en **tamaño (~2,7B)**,
**contexto largo (≥32K)** y **"rendiría mejor en esta GPU"** es el
**LFM2.5-2.6B**. Si el dueño oyó "Sai 2" en un vídeo o podcast, las confusiones
fonéticas más plausibles son "Saiga 2", "Sailor 2" o "Phi-2"; de las cuatro, la
única que vale la pena cargar es LFM2.5-2.6B.

### Ficha del candidato (fuentes)

- Publica: **Liquid AI** (líquida, EEUU). Modelo: `LiquidAI/LFM2.5-2.6B`
  (hermano mayor de la familia 350M / 1.2B / 2.6B / 8B-A1B).
- Arquitectura híbrida: 30 capas (22 convoluciones cortas + **8 de atención
  completa**), hidden 2048, 32 heads Q / 8 KV (GQA), vocab 128k, RoPE θ=10M.
  Fuente: `config.json` del repo (`max_position_embeddings: 131072`).
- Contexto nativo: **131 072 tokens** (docs.liquid.ai: LFM2.5-2.6B; la serie
  anterior LFM2-2.6B era 32K y está marcada deprecated).
- Licencia: **LFM Open License v1.0** (marco Apache-2.0 con tope comercial:
  gratis < 10 M USD/año de facturación; ver https://www.liquid.ai/lfm-license).
  Uso personal/local en esta máquina: cubierto.
- GGUF oficial: https://huggingface.co/LiquidAI/LFM2.5-2.6B-GGUF —
  `LFM2.5-2.6B-Q4_K_M.gguf` (**1 674 455 040 B ≈ 1,56 GiB** por cabecera
  `content-length` del CDN), revisión `main` = `e7caca5d835a3901a8e0d63e94009429bafafdfc`
  (2026-09-22). También Q4_0 / Q5_K_M / Q6_K / Q8_0 / BF16 / F16.
- Soporte en el motor: el soporte LFM2 se fusionó en llama.cpp con el PR #14620
  (2025-07-11); el `llama-server.exe` local es build 10683 del **2026-09-09**,
  ~14 meses posterior. Evidencia binaria local (2026-09-28, scan de `bin/` con
  `node`): `bin/llama.dll` contiene la tabla de arqs (`...gpt-oss.lfm2....lfm2moe...`),
  las rutas de fuente `src/models/lfm2.cpp` y `lfm2moe.cpp`, y los símbolos
  `llama_model_lfm2@@` / `llama_model_lfm2moe@@` (+ grafos); `bin/llama-common.dll`
  trae `Using specialized template: LFM2` y `LFM2.5`. (`llama-server.exe` es un
  stub de 10 240 B; el código vive en las DLL.) Soporte confirmado, no inferido.

## PASO 2 — Factibilidad y análisis 512K (LFM2.5-2.6B Q4_K_M)

### KV-cache: aritmética (con `-ctk q4_0 -ctv q4_0`, 0,5 B/elemento)

Solo las 8 capas de atención acumulan KV (el estado conv es de tamaño fijo):

> 2 (K+V) × 8 capas × 8 heads KV × 64 dim (2048/32) = 8192 elementos
> × 0,5 B = **4096 B/token ≈ 4 KB/token**.

| Contexto | KV (q4_0) | Pesos (Q4_K_M) | Total aprox. en VRAM | ¿Cabe en 16 GB? | ¿Lo soporta el modelo? |
|---|---|---|---|---|---|
| 32K (perfil `velocidad`) | 32 768 × 4 KB = **128 MB** | 1,56 GiB | ≈ 2 GB (+buffers) | Sí, sobrado | Sí |
| 64K (perfil `multi_doc`) | **256 MB** | 1,56 GiB | ≈ 2,2 GB | Sí | Sí |
| 128K (perfil `libros`) | **512 MB** | 1,56 GiB | ≈ 2,5 GB | Sí, entero en VRAM | Sí (es su máximo nativo) |
| 262K (`max_contexto`) | 1 GB (hipotético) | 1,56 GiB | ≈ 3 GB | Cabría, **pero** | **NO**: supera `max_position_embeddings` (131 072) → no se prueba |
| 512K (hipotético) | 524 288 × 4 KB = **2 147 483 648 B = 2 GB** | 1,56 GiB | ≈ 4 GB | Cabría, **pero** | **NO**: 4× su máximo entrenado → no se prueba |

`--cache-ram` es irrelevante aquí: hasta el peor caso cabe en VRAM con 11+ GB
libres (el flag existe para el Qwen de 13 GB, no para un modelo de 1,6 GB).

### Velocidad esperada (matemática de ancho de banda, 512 GB/s)

Techo teórico = 512 GB/s ÷ tamaño del modelo en GB:

- Qwen actual: 512 ÷ 13,9 ≈ **36,8 t/s** → medido 30,2 t/s @32K (82 % del techo: coherente).
- LFM2.5 Q4_K_M: 512 ÷ 1,674 ≈ **306 t/s** → techo ~8,3× el del Qwen.
  Realista en Vulkan con overhead de kernels para modelo pequeño: **hipótesis
  100–250 t/s en decode** (a verificar en vivo; el prefill de prompts de ~10K
  debería resolverse en segundos, no en los ~300 s que el Qwen paga en frío).

### Veredicto 512K: NO, y con dos argumentos independientes

1. **El modelo no lo soporta**: máximo nativo 131 072. Pedir 524 288 es 4× su
   ventana entrenada; la calidad más allá de 131K sería extrapolación RoPE
   degradada (basura honesta, no contexto útil). La app valida hasta 1048576
   (`profile_context_ok`: 1024..=1048576 en pasos de 1024), así que **nada
   impide pedirlo por API**: la guarda tiene que ser humana, no de código.
2. **No hace falta ni cabría ganancia**: el KV de 512K (2 GB) sí cabría, pero
   ¿para qué? Ningún perfil operativo lo pide y el modelo no puede llenarlo con
   calidad. Añadir un perfil 512K sería una trampa en la UI: carga OK, números
   verdes, respuestas malas.

Tampoco 262K para este modelo (supera 131 072). Plan de prueba: **32K / 64K /
128K** con los perfiles existentes (`velocidad`, `multi_doc`, `libros`).
## PASO 3 — Descarga + importación manual (EJECUTADO 2026-09-28)

Aprobado por el dueño con el rationale: "Sai 2" no existe como LLM (evidencia
web + Hub del Paso 1) y LFM2.5-2.6B es el único candidato que encaja en la
descripción (chico, rápido en 16 GB, 128K nativo) — corroborado porque el
proyecto de referencia (local-ai-registry de 0xSero) lo recomienda para
tarjetas de 8–16 GB.

Comando ejecutado (descarga manual, sin el gestor de la app):

    curl -L --fail --retry 3 -o models/LFM2.5-2.6B-Q4_K_M.gguf "https://huggingface.co/LiquidAI/LFM2.5-2.6B-GGUF/resolve/e7caca5d835a3901a8e0d63e94009429bafafdfc/LFM2.5-2.6B-Q4_K_M.gguf"

- Tamaño: **1 674 455 040 B** (1,56 GiB; coincide con el `content-length` del CDN).
- SHA256 en disco: `02a8b7e17487d326e46d68ce0ba24211e1b80a14c4cd0597fa73c1cd697f52ed`.
- El repo GGUF no publica hash de referencia (solo los safetensors originales
  lo tendrían), así que no hay contra qué comparar: el tamaño exacto más la
  carga exitosa del motor son la verificación.
- Descarga: ~31 s.

Verificación de importación manual (`GET /api/models`, el mismo listado que usa
el selector de la UI):

    {"filename": "LFM2.5-2.6B-Q4_K_M.gguf", "rel": "LFM2.5-2.6B-Q4_K_M.gguf", "size_bytes": 1674455040, "size_gb": 1.56, "source": "local", "verified": false}

El fichero copiado a mano aparece junto al Qwen sin importar nada: la ruta
manual queda probada.

## PASO 4 — Vía app r14/r15 con recovery MTP (EJECUTADO 2026-09-28)

### 4.0 Historial: MTP bloqueaba, r14 lo recupera (32K del ship worker; evidencia ajena citada)

- `POST /api/start {"model":"LFM2.5-2.6B-Q4_K_M.gguf","context":32768}` →
  `200 {"status":"starting","pid":17456}`.
- A los ~45 s: `{"status":"error","pid":null,"acceptance_ok":null}` (sin
  `last_error` en el JSON; el motivo está en `%APPDATA%/LocalMind/logs/localmind.log`).
- Causa (log del motor, verbatim): `creating MTP draft context against the
  target model ...` → `context type MTP requested but model doesn't contain
  MTP layers` → `failed to create MTP context` → `exiting due to model loading error`.
- La app siempre pasa `--spec-type draft-mtp ...` porque el TOML vivo tiene
  `[engine.speculation] enabled = true` (pensado para el Qwen, que sí trae
  capas MTP). El LFM2.5 no las trae → el arranque vía app es imposible sin
  tocar config. **No se tocó**: `src-rust/**` y la config viva están fuera del
  permiso; el TOML quedó idéntico al backup (verificado con `diff`).
- Segundo intento (tras >3 min, mismo request, para descartar transitorio) →
  idéntico `error` (pid 16888). Van **2 de 3 cargas** del presupuesto. Se para
  la vía app: un tercer intento daría el mismo error determinista.
- Moraleja: no es que el modelo "no rinda" — es que **la app hoy solo arranca
  modelos con capas MTP**. Cualquier modelo sin MTP (la mayoría del Hub)
  fallará igual hasta que el arranque sea condicional.
- FIX VERIFICADO (run del ship worker, mismo día, build r14 con recovery
  dirigido — no es run mío; lo cito como evidencia ajena): el log muestra la
  secuencia `failed to create MTP context` →
  `[LocalMind] El modelo no soporta decodificación especulativa (MTP):
  reintentando sin --spec-*` → `model loaded` + `listening on 127.0.0.1:8080`
  (~2 s entre reintento y loaded). Puerta: `Verificación de arranque OK:
  187 t/s (mediana de 3)` (muestras del log: 187,89 / 187,86 / 187,38 t/s).
  Probes vía gateway en ese run: corto 15 tok en 42,9 ms (349,8 t/s) +
  decode 10 tok en 49,7 ms (180,9 t/s); largo 29 tok en 44,5 ms (651,2 t/s) +
  611 tok en 3299,8 ms (184,9 t/s). Líneas `usage.jsonl` (la pieza que
  faltaba): `{"completion_tokens":10,...,"model":"LFM2.5-2.6B-Q4_K_M","ms":94,
  "prompt_tokens":15,"stream":true}` y `{"completion_tokens":611,...,
  "ms":3349,"prompt_tokens":29,"stream":true}`. Stop limpio posterior
  (`Servidor detenido`, sin `llama-server.exe`). El recovery dirigido
  funciona; 64K/128K vía app quedan como re-test mío pendiente del go-ahead.

### 4.0b Mis ventanas vía app r14 (2026-09-28; runs propios, gateway + usage reales)

El build r14 reintenta una vez sin `--spec-*` ante la firma exacta
(`model doesn't contain MTP layers`), en `starting`, sin consumir
cooldown/tope (verificado en `src-rust/src/process.rs`: `mtp_retry_decision` +
`spawn_child_nospec` + `build_engine_cmd(..., skip_spec)`). Secuencia en cada
arranque LFM2.5: `failed to create MTP context` →
`[LocalMind] El modelo no soporta decodificación especulativa (MTP):
reintentando sin --spec-*` → `model loaded` + `listening` (~2 s después).

**Hallazgo de precedencia (afecta a 64K/128K)**: mis tres arranques
(`{"model":..., "context":65536}` 1790634905; `{"model":..., "profile":"multi_doc",
"context":65536}` 1790635249; `{"model":..., "profile":"libros", "context":131072}`
1790635555) dejaron el motor con **`n_ctx_slot = 32768`** pese al contexto pedido
(log: `Contexto: 65536/131072 tokens` pero `initializing, n_slots = 1,
n_ctx_slot = 32768`). Causa: `start()` calcula `context` vía `resolve_context`
pero `spawn_child_nospec()` (el reintento que deja el motor en pie) reconstruye
el argv leyendo `st.context` del status — que aún conserva la sesión anterior
(32768) porque `start()` solo persiste `last` al final del arranque original.
**Todo lo medido vía app en este turno es 32K efectivo**: las tres ventanas son
tres réplicas 32K independientes (repetibilidad, no curva por contexto). La
curva real 32K/64K/128K es la de cargas directas §4.3–§4.5, donde `-c` sí se
respetó. No toco código (fuera de permiso); defecto derivado al engine worker:
el reintento debería releer el `context` ya resuelto (o persistir `last` antes
de relanzar).

Réplica A ("64K" pedido, start 1790634905 pid 1568 → efectivo 18480;
`running` + `acceptance_ok:true`, gate `186,39`, samples
`[187,09, 186,39, 185,36]`, `engine_slow: false`):

- Corto (`max_tokens: 30`, wall gateway 225 ms): `finish: length` (razona
  primero), prompt 20 tok en ~44 ms (455,9 t/s), decode 30 tok en 153,5 ms
  (**189,0 t/s**).
- Largo (1–200, `finish: stop`, wall 5607 ms): prompt 25 tok en 43,2 ms
  (578,5 t/s), 1029 tok en 5531,6 ms (**185,8 t/s decode**).
- `usage.jsonl`: `{"completion_tokens":30,...,"model":"test","ms":200,
  "prompt_tokens":20,"stream":false,"ts":1790635031}` y
  `{"completion_tokens":1029,...,"ms":5582,"prompt_tokens":25,"stream":false,
  "ts":1790635041}`.
- `GET /api/metrics`: `tokens_predicted: 1675`, `spec_draft_tokens: 0` (sin
  especulación, como debe ser). `POST /api/stop` → `stopped` (1790635045), limpio.

Réplica B ("64K" + `profile: multi_doc`, start 1790635249 pid 18020 → efectivo
15928; gate `185,87`, samples `[187,09, 185,87, 183,15]`, `engine_slow: false`):

- Corto (wall 225 ms): prompt 455,9 t/s, decode **189,0 t/s**.
- Largo (wall 5587 ms, `finish: stop`): prompt 578,0 t/s, decode **186,5 t/s**.
- `usage.jsonl`: `{"completion_tokens":30,...,"ms":198,...,"ts":1790635339}` y
  `{"completion_tokens":1029,...,"ms":5561,...,"ts":1790635347}`.
- Stop → `stopped` (1790635350), limpio. Espera antes del siguiente:
  1790635350 → 1790635555 (205 s + carga).

Réplica C ("128K" + `profile: libros`, start 1790635555 pid 18912 → efectivo
3256; gate `185,53`, samples `[186,39, 185,53, 185,19]`, `engine_slow: false`):

- Corto (wall 226 ms): prompt 458,7 t/s, decode **187,0 t/s**.
- Largo (wall 5662 ms, `finish: stop`): prompt 575,3 t/s, decode **184,0 t/s**.
- Prefill 12K vía gateway (wall 7690 ms): 12016 tok en 7496 ms = **1603,0 t/s**,
  decode posterior 168,1 t/s.
- `usage.jsonl`: `{"completion_tokens":30,...,"ms":200,...,"ts":1790635643}`,
  `{"completion_tokens":1029,...,"ms":5637,...,"ts":1790635652}` y
  `{"completion_tokens":30,...,"ms":7684,"prompt_tokens":12016,...,"ts":1790635663}`.
- `GET /api/metrics`: `prompt_tokens_total: 12094`, `tokens_predicted: 1705`
  (`gen_tps: 0.0` en idle: el gateway expone acumulados; el decode real está en
  los `timings` de cada respuesta).
- Stop → `200 {"status":"stopped"}` (1790635679), sin `llama-server.exe`, GUI viva.

Puertas app del modelo (32K ship 187 / 186,57 + mis réplicas 186,39 / 185,87 /
185,53): dispersión ±1 t/s — repetibilidad excelente. TTFT gateway en caliente:
~225 ms wall para el corto (vs ~1 s del Qwen con thinking). Cero inestabilidad
(938 s entre mi primer start y el stop final, 3 arranques + ship).

### 4.1 Plan B (sin consumir cargas): mismo binario, mismos argv menos MTP, puerto aparte

Para no dejar la evaluación a medias se midió con el propio
`bin/llama-server.exe` en `127.0.0.1:18099` (puerto que la app no usa),
mismos flags que la app (`-ngl 99 -c N -ctk q4_0 -ctv q4_0 -np 1 --metrics`)
menos los `--spec-*`. Cada carga directa se mató solo por su PID
(`taskkill /PID <pid>`, nunca `/IM`), con 3+ min entre cargas. La app quedó
en `stopped` todo el tiempo; al final `POST /api/stop` →
`200 {"status":"stopped"}` y se confirmó sin `llama-server.exe`.

### 4.2 Medidas 32K (carga directa `-c 32768`, `model loaded` en ~1,5 s)

- Corto ("Responde exactamente: OK", `max_tokens: 10`, frío): prompt 45 ms
  (**353,7 t/s**), decode 10 tok en 49 ms (**185,4 t/s**).
- Corto en caliente: decode **180,2 t/s**.
- Largo ("números del 1 al 300", `finish: stop`, coherente `1..N` uno por línea):
  prompt 25 tok en 43 ms (**575,9 t/s**), **1180 tok en 6379 ms = 184,8 t/s decode**.
- Extra ("Cuenta del 1 al 10", `max_tokens: 60`): decode **186,0 t/s**.
- Coherencia: correcta en listas y cuentas; el probe "exactamente OK" con
  `max_tokens: 10` devuelve `content` vacío con 10 tokens de
  `reasoning_content` (el Instruct razona antes de responder y el tope lo
  corta: `finish: length`). Con `max_tokens: 30` responde OK (ver 64K/128K).

### 4.3 Medidas 64K (carga directa `-c 65536`, `model loaded` en ~1,5 s)

- Corto ("Responde con la palabra OK y nada más", `max_tokens: 30`):
  prompt **450,0 t/s**, decode **187,9 t/s**.
- Largo ("números del 1 al 200", `finish: stop`, 1029 tok coherentes):
  prompt **593,8 t/s**, decode **184,3 t/s** en 5,6 s de wall.

### 4.4 Medidas 128K (carga directa `-c 131072` = máximo nativo, `model loaded` en ~1,5 s)

- Corto: prompt **452,0 t/s**, decode **186,4 t/s**.
- Largo (1–200, `finish: stop`, 1029 tok): prompt **583,7 t/s**, decode
  **184,1 t/s** en 5,6 s de wall. Sin degradación respecto a 32K/64K.
- Prefill largo (~9,7K prompt real / 12016 tok con plantilla, `max_tokens: 30`):
  **8173 ms → 1470,2 t/s prefill**; decode posterior 166,2 t/s. El mismo orden
  de prompt que al Qwen le cuesta ~300 s en frío aquí son **8 s**.

### 4.6 Contextos NO probados (y por qué)

- **262K / 512K**: no probados a propósito. Máximo nativo 131 072; 262K = 2× y
  512K = 4× extrapolación RoPE (respuestas degradadas), con KV de 1–2 GB que sí
  cabría pero para nada útil. Veredicto en pie: **no añadir perfil 512K** (ni
  262K para este modelo). La aritmética del Paso 2 no cambia.
- **64K/128K efectivos vía app**: conseguidos con el build fijo r15 (§4.7).
  Las ventanas previas (§4.0b) quedan como réplicas 32K-efectivas (evidencia de
  repetibilidad, no de curva por contexto).

### 4.7 Ventanas vía app con build FIJO r15 (n_ctx_slot correcto; 64K propio + 128K ship citado)

Fix del engine worker (r15 `DB51E196…`): el reintento MTP usa los parámetros
resueltos del request (no el status obsoleto) y la app verifica el contexto
real del motor vía `/props` (falla a gritos si no coincide).
**64K propio** (start 1790636774 `{"model":"LFM2.5-2.6B-Q4_K_M.gguf","context":65536}`
→ `200 {"status":"starting","pid":12864}` → `running` pid 9064,
`profile: libros`, `context: 65536`; engine log:
`initializing, n_slots = 1, n_ctx_slot = 65536` ✅ + MTP-retry loggeado +
`Verificación de arranque OK: 187 t/s`; status: gate `decode_tps: 186,92`,
samples `[187,09, 186,92, 184,50]`, `engine_slow: false`, `acceptance_ok: true`):

- Corto (wall gateway 227 ms): prompt 20 tok en 42,2 ms (473,9 t/s), decode 30
  tok en 155,6 ms (**186,3 t/s**), `finish: length` (razona primero).
- Largo 1–200 (wall 5642 ms, `finish: stop`): prompt 579,3 t/s, 1029 tok en
  5568,5 ms (**184,6 t/s decode**).
- `usage.jsonl`: `{"completion_tokens":30,...,"model":"test","ms":199,
  "prompt_tokens":20,"stream":false,"ts":1790636863}` y
  `{"completion_tokens":1029,...,"ms":5617,"prompt_tokens":25,"stream":false,
  "ts":1790636871}`.
- `GET /api/metrics`: `tokens_predicted: 1675`, `spec_draft_tokens: 0`.
- Stop → `200 {"status":"stopped"}` (1790636874), sin `llama-server.exe`.
- Una sola ventana 64K (suficiente: el número replica los otros cuatro 32K y
  el 128K fijo; más cargas no aportarían curva, solo desgaste PSU).

**128K fijo** (ship worker r15, evidencia ajena citada; start 1790636568
`context: 131072`): engine log `n_ctx_slot = 131072` ✅ (cero 32768
obsoletos), MTP-retry con `-c 131072` sin `--spec-*`, carga 9 s, gate
**186,74** (samples 186,74 / 185,70 / 187,97, `engine_slow: false`),
`usage.jsonl`:
`{"completion_tokens":10,...,"model":"LFM2.5-2.6B-Q4_K_M","ms":92,
"prompt_tokens":15,"stream":true,"ts":1790636588}` y
`{"completion_tokens":611,...,"ms":3318,"prompt_tokens":29,"stream":true,
"ts":1790636591}`.
Puertas app del modelo (todas las réplicas): 187 / 186,57 / 186,39 / 185,87 /
185,53 (32K-efectivas r14) + 186,92 (64K real r15) + 186,74 (128K real r15):
**seis puertas entre 185,5 y 187,0** — el modelo rinde lo mismo en 32K, 64K y
128K (lógico: 2 GB de pesos + KV de MB, todo en VRAM en los tres casos).


## Comparación LFM2.5-2.6B vs Qwen3.8-27B (Qwen: `tests/bench-results/*.json` + docs)

| Contexto | Modelo (tamaño) | Carga | Prefill | Decode t/s | Puerta app |
|---|---|---|---|---|---|
| 32K | Qwen3.8-27B IQ4_XS (12,95 GiB) | 24,5 s | frío ~10K ≈ 30–70 t/s (300 s, frágil) | **30,17** (`bench-20260924-231526`) | 34,1 OK |
| 32K | LFM2.5-2.6B Q4_K_M (1,56 GiB) | **~6 s + puerta** (MTP-retry incl.) | 349–578 t/s (ms) | **≈185–189** | **185–187 OK (4 réplicas), `engine_slow: false`** |
| 64K | Qwen3.8-27B IQ4_XS | 22,3 s | ídem | **30,07** | 34,0 OK |
| 64K | LFM2.5-2.6B Q4_K_M | **~8 s + puerta** (MTP-retry incl.) | 474–579 t/s | **≈185–186** (directa + app r15) | **186,92 OK** (`n_ctx_slot = 65536` ✅) |
| 128K | Qwen3.8-27B IQ4_XS | 26,2 s | frío 8K cuelga (~300 s); caliente 189,4 t/s | **29,45** campaña / 15–18 fair | 14–32 según carga |
| 128K | LFM2.5-2.6B Q4_K_M (su tope nativo) | **9 s + puerta** (MTP-retry incl.) | **1470–1603 t/s** a 12K prompt; sin degradación | **≈184–187** | **186,74 OK** (`n_ctx_slot = 131072` ✅, ship r15) |
| 262K | Qwen3.8-27B IQ4_XS | 34–40 s | — | **14,16** campaña / OMP eff 33,03 ronda 9 | 15–30 según carga |
| 262K | LFM2.5-2.6B | — | — | **no soportado** (2× su máximo) | — |

Lectura honesta:

- **Velocidad**: el LFM2.5 decodifica **~6× más rápido** que el Qwen (185–189 vs
  30 t/s) y carga en segundos frente a 24–40 s; el prefill largo pasa de minutos
  a segundos (12K: 7,5–8 s vía gateway, 1470–1603 t/s). El techo teórico
  (306 t/s) no se alcanza — el kernel Vulkan para un modelo tan chico rinde al
  ~60 % —, pero ~186 t/s estables es el número real: 6 puertas app (4×32K +
  64K + 128K, todas 185,5–187,0) y 3 contextos directos sin degradación.
- **Para qué sirve**: chat rapidísimo, borradores, clasificación, tool-calling
  ligero y colas de muchos requests cortos. Para **coding-agent** (system
  prompts de 5–12K, razonamiento largo, código correcto a la primera) el 27B
  sigue en otra liga: ningún t/s compensa una respuesta peor. Son
  complementarios, no sustitutos.
- **Recomendación práctica**: LFM2.5 para chat/borradores (TTFT ~225 ms,
  186 t/s, carga en segundos, 128K nativos verificados); Qwen3.8-27B para
  agentes de código y contexto largo real (262K). No añadir perfil 512K; no
  probar 262K en el LFM2.5.

## Conclusión honesta (vía app r15 + directas)

- **Para velocidad/chat en esta máquina**: confirmado por vía app con contexto
  real: puertas 185–187 (6 réplicas: 4×32K + 64K + 128K), TTFT ~225 ms, decode
  184–189 t/s, prefill 12K en ~7,7 s vía gateway.
- **Para trabajo de coding-agent**: el 27B sigue siendo el modelo serio; el
  2,6B es el modelo rápido. La decisión es por tarea, no por t/s.
- **512K**: error añadirlo. Tope honesto: 128K para LFM2.5, 262K para el Qwen
  (§4.6: 4×/2× extrapolación RoPE; KV 512K = 2 GB que cabría pero para nada).
- **Defectos que este test surfacó (ambos ya fixeados en r14/r15)**:
  (1) MTP-retry inexistente → recovery dirigido; (2) reintento con contexto
  obsoleto → usa parámetros resueltos + verificación `/props`.
## Estado final

El `LFM2.5-2.6B-Q4_K_M.gguf` queda en `models/` listado por la app
(`source: local`, `verified: false`, como el Qwen). Si lo que escuchaste era
literalmente "Saiga 2" o "Sailor 2", decilo y ajusto el plan.
