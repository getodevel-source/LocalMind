# Auditoría de rendimiento OMNI (2026-10-03, Windows, RX 6800 XT + Ryzen 5 7600)

Metodología: números medidos, no estimados. Research upstream vía
`web_fetch` (llama.cpp master: README + `tools/server/README` + perf tips).

## Números verificados

| Métrica | Valor | Cómo |
|---|---|---|
| Arranque frío → ventana | ~0,6 s | `MainWindowHandle` tras lanzar `OMNI.exe` |
| API local en caliente | ~1 ms | `GET /api/version` (401 sin clave, pero responde) |
| Idle RAM total | ~370 MB | `OMNI.exe` ~23 MB + árbol WebView2 ~350 MB (atribuido por `ParentProcessId`) |
| Payload chat 200 KB | 18,94 → **5,71 ms** (3,3×) | test temporal 20 iter, luego borrado; queda test de equivalencia |
| Decode motor | ~25 t/s | medido en sesión anterior (Qwen3.8-27B IQ4_XS, Vulkan) |
| Exe release / debug | 3,68 / 12,3 MB | `lto + strip` ya activos |
| `ui.html` | 217 KB | parse único al cargar |
| Carga UI en reposo | ~1,8 req/s | status 800 ms + metrics 2 s + usage 30 s + SSE |
| Gateway `/api/status` | p50 **0,42 ms** | 500 req keep-alive, loopback (incl. ~0,3 ms cliente) |
| Gateway `/v1/models` | p50 **0,37 ms** | idem; costo servidor real < 0,2 ms |
| Markdown 16 KB en Chromium | **0,27 ms** por parse completo | CDP `renderMarkdown`, 30 iter |
| Paint de streaming | ≤25 Hz → **≤6,7 Hz** | throttle 150 ms + render final (el parse era barato; el layout no) |
| Carga de página UI | dom ~150 ms, load ~200 ms | CDP navigation timing |
| Cambio de tab | p50 **33 ms** | click → paint, 10 iter CDP |
| Log del motor en arranque | O(n²) → **O(1)** por línea | `textContent +=` re-serializaba 200 KB por línea; ahora nodos de texto con poda por conteo |
| Historial de chat | tope **300 mensajes** | sin tope mataba la cuota localStorage de 5 MB en silencio |

Nota honesta: dos intentos de medir "cold start a API 200" dieron 30-60 s,
pero estaban contaminados por mi propio setup (locks stale + puertos de
instancias de prueba). La medición limpia con handle de ventana da 0,6 s.
SmartScreen/Defender (binario sin firma, D-24) puede sumar segundos en la
PRIMERA ejecución en una máquina nueva; no se midió ese caso.

## Motor: ya óptimo según upstream

Verificado contra `llama-server --help` (build 10743) + docs master:

- `threads = 6` = núcleos físicos (Ryzen 5 7600: 6/12). El tip upstream dice
  exactamente eso (empezar en 1 y subir hasta físicos). ✓
- `--no-warmup` ya pasado (test lo fija): la puerta de aceptación calienta.
  El default upstream (`--warmup`) calentaría dos veces. ✓
- `poll = 0` en TOML: el motor no spinea en reposo. ✓
- `--jinja` presente (requisito para tool-calling y Responses nativo). ✓
- `--cache-ram`, `--spec-*`, `-ctk/-ctv` disponibles; KV en `q4_0`.

## Hallazgo grande (propuesta, NO ejecutado)

El motor 10743 es un fork (`PrismML-Eng/llama.cpp`, rama `prism`,
commit `adfffbe41` del 2026-09-25: kernels x86 SIMD para pesos ternarios
PTQ1_0/PQ2_0 — coincide con nuestras evals de Ternary Bonsai) y trae
**nativos** `/v1/messages` y `/v1/responses`
(verificado por strings en `llama-server-impl.dll` Y en vivo contra motor
real con Qwen3.8). Nuestro `translate.rs`
(932 líneas) emula esos dos dialectos por encima de chat/completions.
Oportunidad: reenvío directo al motor y **borrar translate.rs** + la
maquinaria Tee/Translate del proxy. Riesgo: los harnesses (pi/omp/opencode,
dsh) se verificaron contra NUESTRA traducción; la nativa puede diferir en
detalles (thinking blocks, tool_use). Requiere: matriz de compat por
harness antes de borrar. Estimación: −1500 líneas netas.

Evidencia en vivo (`max_tokens`/`max_output_tokens` 30, modelo pensando):

- Nativo `/v1/messages`: content con bloque `thinking` + firma, `usage` con
  `cache_read_input_tokens`, `model: "localmind"` (alias del motor).
- Gateway (nuestra traducción): `"content": []` vacío, sin `cache_read`,
  `model: "q"` sin reescribir.
- Nativo `/v1/responses`: `output` con item `reasoning` + texto, `usage`
  completo con `cached_tokens`.
- Gateway (nuestra traducción): `"output": []` vacío.

Un cliente real (Claude Code / Codex) recibe RESPUESTAS VACÍAS cuando el
modelo aún está pensando. La traducción pierde los bloques de thinking en
ambos dialectos: bug de fidelidad, no solo deuda. La contabilidad no se
pierde con el reenvío (el nativo trae `usage` completo).

## Costos conocidos, sin tocar (con motivo)

- Puerta de aceptación: 1×16 + 3×200 tokens ≈ 25 s a 25 t/s por arranque.
  Cambiar el muestreo debilita la mediana (D1); solo con A/B en vivo.
- Poller 500 ms: 1-2 HTTP localhost + ~5 clones de `AppConfig` por tick.
  Medido por diseño: ~1-3 ms CPU por tick, despreciable. No churn.
- `usage.jsonl`: append sin fsync por request. Correcto así.
- Thread-por-request (`tiny_http` + `ureq` bloqueante): bien para uso hogar
  (1 Oráculo + 1 Guest); no es un servidor multiusuario y no debe serlo.

## Research pendiente (web_fetch)

- `docs/development/token_generation_performance_tips.md`: traído (threads
  físicos + `-ngl` alto; ya aplicado).
- Comparar flags nuevos master vs 10743 (`--poll-batch`, `-cram`,
  `--kvu`, `--fit`) para el próximo upgrade de `bin/`.
- `web_search` del harness roto (endpoint opencode.ai sin sesión); workaround:
  `web_fetch` directo a URLs conocidas. Reportado al dueño.
