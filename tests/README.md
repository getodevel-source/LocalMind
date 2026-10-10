# LocalMind smoke test (read-only)

`tests/smoke.mjs` — health check of the LocalMind HTTP API on loopback.
Read-only: the script never starts/stops the engine, and its only POST
(check 14) is a malformed `/api/start` body the server rejects with 400
before touching anything. Contract: `docs/SRS.md` §3.5 +
`src-rust/src/server.rs`. No external dependencies, Node >= 18 (global
`fetch`).

## What it checks

1. `GET /` → 200, `Content-Type` contains `text/html`, body contains
   `<html` (or the embedded fallback marker
   `LocalMind Studio: Cargando interfaz...` from
   `src-rust/ui_fallback.html`).
2. `GET /localmind.ico`, `GET /localmind.png`, `GET /omni.ico`,
   `GET /omni.png` → 200 or 404 (both accepted, the script prints which).
3. `GET /api/status` → 200 JSON with the `ServerStatus` shape
   (`process.rs:27-38`): requires `status` (string, one of
   `stopped|starting|running|error`); `idle_remaining_secs` is
   `skip_serializing_if None`, so it is accepted ABSENT (idle engine,
   printed as `absent (motor en reposo)`) or as a number — present and
   non-numeric → FAIL. Never fails for an idle engine.
4. `GET /api/hardware` → 200 JSON with a `gpus` array (`HardwareInfo`,
   `process.rs:48-52`); asserts array presence, length >= 0.
5. `GET /api/profiles` → 200 JSON array, length >= 1.
6. `GET /api/settings` → 200 JSON with `config_path`, `http_port`,
   `llama_port` (`server.rs:45-53`).
7. `GET /api/models` → 200 JSON array; each entry has `name: string`
   and `size_gb: number` (`ModelInfo`, `process.rs:19-24`).
8. `GET /api/logs` → 200 JSON array, length <= 250.
9. `GET /api/metrics` → reads `/api/status` first and asserts the
   matching branch: engine up → 200; engine down → 502
   `{"error":"engine_down"}` (`server.rs:328-333`). Prints the branch.
10. `OPTIONS /api/start` with `Origin: http://localhost` → preflight
    answered (2xx + `Access-Control-Allow-Origin`); prints status and
    the `Access-Control-*` headers.
11. `GET /definitely-not-a-route` → 404 `{"error":"not_found"}`
    (`server.rs:649`).
12. `GET /api/events` → within 8 s either a `data:` event line arrives
    or the stream is open as `text/event-stream` with no event yet;
    aborted after the check via `AbortController`. Prints the branch.
13. `GET /v1/models` → 200 JSON with a `data` array whose ids include
    the CLI aliases (`localmind`, `qwen3.8-27b`); when the server
    enforces the key and none is known, 401/403 is the expected branch.
    Prints the branch and the ids found.
14. `POST /api/start` with `{"context":"abc"}` (wrong type) → 400 with an
    `error` containing `JSON inválido`. This is the script's only POST: the
    request is rejected before the engine is touched, so it starts no engine
    and mutates no state. It guards the fix that stopped a malformed body from
    silently booting the engine with defaults and answering 200.
Every check prints `PASS/FAIL <check> — <observed>`. Exit code 0 only
if every check passed, else 1 plus a summary block of failures with
raw observed status/body (truncated to 400 chars).

Unreachable base URL prints `is LocalMind running?`.

## How to run it

```bash
node tests/smoke.mjs [baseUrl]   # default http://127.0.0.1:17860
```

At startup the script prints one line stating the key source in use:
`key source: LM_KEY | gateway.key | none`.

Sin clave no se envía credencial; los checks protegidos responden 401
(la app exige clave por la puerta D-7):
```bash
node tests/smoke.mjs
```

Env-key mode:

```bash
LM_KEY=<key> node tests/smoke.mjs
# Windows cmd: set LM_KEY=<key> && node tests/smoke.mjs
```

File-key mode (automatic fallback): when `LM_KEY` is unset, the script
reads `%APPDATA%/LocalMind/gateway.key` (`process.env.APPDATA`,
whitespace/newline trimmed, ignored if empty or absent).

A known key is sent as `Authorization: Bearer <key>` on every request.
If any check receives 401/403, its FAIL line says:
`401 unauthorized — set LM_KEY or ensure
%APPDATA%/LocalMind/gateway.key exists (the server now requires the
key)`.


## How to capture fixtures

Flag-based (preferred) — after the checks, captures the raw bodies of
`GET /api/status`, `/api/hardware`, `/api/profiles`, `/api/settings`,
`/api/models`, `/v1/models` into `tests/fixtures/<name>.json`
(`v1-models.json` for `/v1/models`), pretty-printed with 2 spaces:

```bash
node tests/smoke.mjs --write-fixtures [--force] [baseUrl]
# flags may come before or after baseUrl
```

- Refuses to overwrite an existing fixture unless `--force` is passed
  (prints `SKIP <file> — exists` otherwise).
- Each body is parsed with `JSON.parse` first; unparseable bodies are
  skipped and reported, never written (no partial/invalid files — the
  write goes to a temp file + atomic rename).
- 401/403 or non-200 responses are skipped with the reason printed.
- Fixture capture never affects the check results or the exit code.

Manual fallback (curl + python):

```bash
BASE=http://127.0.0.1:17860
for n in status hardware profiles settings models; do
  curl -s "$BASE/api/$n" | python -m json.tool > "tests/fixtures/$n.json"
done
curl -s "$BASE/v1/models" | python -m json.tool > tests/fixtures/v1-models.json
```

(`curl.exe` on Windows; `python -m json.tool` pretty-prints. Send the
key header too if the server enforces it, e.g.
`curl -s -H "Authorization: Bearer $LM_KEY" …`.)

## Known limitation

Read-only salvo el POST de contrato: el único POST es el check 14
(`POST /api/start` con cuerpo mal formado → 400, no toca el motor).
Los checks 9 y 10 solo leen estado / envían OPTIONS, sea cual sea el estado
del motor: el script afirma la rama observada, no la dirige.

## Performance benchmark

`tests/bench.mjs` — reproducible per-profile throughput harness for the
“30 t/s at 262K” target (SRS LM-NF-1 / LM-NF-2). Node ESM, no dependencies,
Node >= 18 (global `fetch`). Key resolution and timeout style copied from
`tests/smoke.mjs`: env `LM_KEY`, else `%APPDATA%/LocalMind/gateway.key`
(trimmed); every HTTP call carries an `AbortSignal` timeout (10 s default,
30 min for the start-wait poll and the streaming probe — a real generation
takes minutes, so a 10 s cap on the probe would abort every measurement).

```bash
node tests/bench.mjs --profiles 32k,64k,128k,262k [--prompt-tokens 512]
  [--runs 1] [--max-tokens 256] [--base-url http://127.0.0.1:17860]
  [--out tests/bench-results] [--dry-run] [--self-test] [--stop-after]
node tests/bench.mjs --compare <a.json> <b.json>
```

CLI shorthands `32k/64k/128k/262k` are translated to the real engine profile
ids (`velocidad`, `multi_doc`, `libros`, `max_contexto`) before calling
`POST /api/start` — the engine silently falls back on unknown ids
(`profiles.rs`), so the raw shorthand is never sent.

Per profile, in the given order: wait out `starting`; stop a `running` engine
whose `profile`/`context` differs; `POST /api/start {profile, context}`; poll
`GET /api/status` every 2 s until `running` + `acceptance_ok === true`
(30 min timeout; `error` records the failure verbatim and continues, exit 1
at the end); one streaming `POST /v1/chat/completions` (`temperature: 0`,
`stream_options.include_usage: true`); `GET /api/metrics` right after. The
engine is left running with the LAST profile unless `--stop-after` is passed.
`--dry-run` prints the plan and touches nothing; `--self-test` asserts the
pure helpers with no app needed.

Table columns: `profile` (CLI shorthand), `context` (expected tokens),
`load s` (start→`running`+`acceptance_ok`, `n/a` when the engine was reused),
`prompt tok` / `compl tok` (final SSE `usage` chunk), `TTFT ms`
(request start→first read carrying a content delta), `last ms`
(request start→last read carrying a content delta), `decode t/s`
(`completion_tokens / (last − TTFT)`), `prefill t/s`
(`prompt_tokens / TTFT`), `buf?` (`YES` = whole response arrived in one
read, rate columns honestly `n/a`; `no` = chunked, timings are real),
`gen_tps` (`GET /api/metrics`), `accept tps`
(`decode_tps` from the acceptance gate in `/api/status`).

Honesty rules: a rate is printed only from the tokens and timings that
justify it — otherwise `null` in JSON / `n/a` in the table, never `0` or an
estimate. A one-shot response is flagged explicitly (`"buffered": true` in
JSON, `YES` in the `buf?` column) instead of silently writing nulls. Failing
profiles are recorded (`error` + verbatim `last_error`), never skipped
silently. Full JSON (config + rows + raw per-run samples + timestamps +
`/api/settings` if reachable) goes to a new
`<out>/bench-<YYYYMMDD-HHMMSS>.json` file (dir created; existing files never
overwritten). Each entry in `samples` carries the probe metrics plus
`samples: [{t_ms, bytes, has_delta}]` — per-read arrival times in ms since
request start, byte size, and whether the read carried a content delta
(capped at 500 entries, `samples_truncated: true` when capped). Missing
status keys are treated as absent, never printed as `undefined`/`NaN`.

Streaming: the app proxy forwards upstream SSE incrementally (`server.rs`
`proxy::TeeLogReader`: pass-through reads, only a 64 KiB tail kept for the
usage log), so per-read arrival timing measures real first/last-token gaps
through the proxy.
A real run needs the app + engine running and takes minutes per profile
(cold 262K loads + up to `max_tokens` of generation each).

## Comparing two runs

`node tests/bench.mjs --compare <a.json> <b.json>` reads two result files
(same shape as `tests/bench-results/*.json`) and prints, per profile present
in either file, the B−A deltas and percentage changes for `load_s`,
`ttft_ms`, `decode_tps`, `prefill_tps`, `engine_gen_tps` and
`acceptance_decode_tps`, plus a verdict line per profile naming the winner
per metric (`A`/`B`/`tie`; higher wins the rate columns, lower wins
load/ttft; within ±2% is a tie). Missing keys print `n/a` (never
`NaN`/`undefined`); profiles present in one file only are listed with an
`only in A/B — no comparison` verdict; key order does not matter. Exit 0
when the table is produced, 1 with a clear message when either file cannot
be read or parsed. Runs no engine, needs no app:

```bash
node tests/bench.mjs --compare tests/bench-results/bench-a.json tests/bench-results/bench-b.json
```
