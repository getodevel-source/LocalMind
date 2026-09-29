#!/usr/bin/env node
// LocalMind read-only smoke test (Node ESM, no dependencies, Node >= 18).
// Usage: node tests/smoke.mjs [baseUrl] [--write-fixtures] [--force]
//   baseUrl defaults to http://127.0.0.1:17860
//   --write-fixtures: after the checks, capture raw bodies of
//     /api/status, /api/hardware, /api/profiles, /api/settings,
//     /api/models, /v1/models into tests/fixtures/<name>.json
//     (pretty-printed, 2 spaces). Refuses to overwrite an existing
//     fixture unless --force is also passed. Bodies are parsed with
//     JSON.parse first; unparseable bodies are skipped, never written.
//   Key resolution (automatic, in order): env LM_KEY, else
//   %APPDATA%/LocalMind/gateway.key (trimmed; ignored if empty/absent).
//   A known key is sent as `Authorization: Bearer <key>` on every request.
// Contract: docs/SRS.md §3.5 + src-rust/src/server.rs. GET-only, salvo el
// preflight OPTIONS y UN POST de contrato: `POST /api/start` con el tipo de
// `context` equivocado, que el servidor debe rechazar con 400 (check 14).
// Ese POST no enciende el motor ni cambia estado: se rechaza antes de arrancar.
// Never starts/stops the engine.

import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const ARGS = process.argv.slice(2);
const WRITE_FIXTURES = ARGS.includes("--write-fixtures");
const FORCE = ARGS.includes("--force");
const BASE = (ARGS.find((a) => !a.startsWith("--")) || "http://127.0.0.1:17860").replace(/\/+$/, "");
const FIXTURE_DIR = join(dirname(resolve(process.argv[1])), "fixtures");
const TIMEOUT_MS = 5000;
const SSE_TIMEOUT_MS = 8000;
const MAX_BODY = 400;
const UNAUTH_MSG = "401 unauthorized — set LM_KEY or ensure %APPDATA%/LocalMind/gateway.key exists (the server now requires the key)";

function resolveKey() {
  const env = (process.env.LM_KEY || "").trim();
  if (env) return { key: env, source: "LM_KEY" };
  try {
    const appdata = process.env.APPDATA;
    if (appdata) {
      const p = join(appdata, "LocalMind", "gateway.key");
      if (existsSync(p)) {
        const k = readFileSync(p, "utf8").trim();
        if (k) return { key: k, source: "gateway.key" };
      }
    }
  } catch {}
  return { key: null, source: "none" };
}

const { key: API_KEY, source: KEY_SOURCE } = resolveKey();
console.log(`key source: ${KEY_SOURCE}`);

const authHeaders = () => (API_KEY ? { Authorization: `Bearer ${API_KEY}` } : {});
const isUnauth = (res) => res.status === 401 || res.status === 403;

const trunc = (s) =>
  s.length > MAX_BODY ? s.slice(0, MAX_BODY) + `…[+${s.length - MAX_BODY} chars]` : s;

const results = [];
function report(ok, check, observed) {
  // Future-proof key enforcement: any check answered 401/403 MUST carry the remedy.
  if (/\bstatus=40[13]\b/.test(observed) && !observed.includes(UNAUTH_MSG)) {
    observed += ` ${UNAUTH_MSG}`;
  }
  results.push({ ok, check, observed });
  console.log(`${ok ? "PASS" : "FAIL"} ${check} — ${observed}`);
}

// Fetch with an AbortSignal timeout. Returns { res, text } or throws.
async function get(path, { timeout = TIMEOUT_MS, headers = {} } = {}) {
  const ctrl = new AbortController();
  const t = setTimeout(() => ctrl.abort(new Error(`timeout after ${timeout}ms`)), timeout);
  try {
    const res = await fetch(BASE + path, {
      signal: ctrl.signal,
      headers: { ...authHeaders(), ...headers },
    });
    const text = await res.text();
    return { res, text };
  } finally {
    clearTimeout(t);
  }
}

// POST con cuerpo. Único uso en el script (ver check 14): un cuerpo que el
// servidor RECHAZA con 400, así que no muta estado ni toca el motor.
async function post(path, body, { timeout = TIMEOUT_MS, headers = {} } = {}) {
  const ctrl = new AbortController();
  const t = setTimeout(() => ctrl.abort(new Error(`timeout after ${timeout}ms`)), timeout);
  try {
    const res = await fetch(BASE + path, {
      method: "POST",
      signal: ctrl.signal,
      headers: { "Content-Type": "application/json", ...authHeaders(), ...headers },
      body,
    });
    const text = await res.text();
    return { res, text };
  } finally {
    clearTimeout(t);
  }
}

function parseJson(text) {
  try {
    return { ok: true, value: JSON.parse(text) };
  } catch (e) {
    return { ok: false, error: String(e) };
  }
}

let fatal = false;
async function safeGet(path, check, opts) {
  try {
    return await get(path, opts);
  } catch (e) {
    report(false, check, `request failed: ${e?.cause?.code || e.message}`);
    fatal = e?.cause?.code === "ECONNREFUSED" || String(e.message).includes("fetch failed");
    return null;
  }
}

async function main() {
  // --- 1. GET / ---
  {
    const r = await safeGet("/", "GET /");
    if (r) {
      const ct = r.res.headers.get("content-type") || "";
      const hasHtml = r.text.includes("<html");
      const hasFallback = r.text.includes("LocalMind Studio: Cargando interfaz...");
      const ok = r.res.status === 200 && ct.includes("text/html") && (hasHtml || hasFallback);
      report(ok, "GET /", `status=${r.res.status} ct=${ct} hasHtml=${hasHtml} fallback=${hasFallback}`);
    }
    if (fatal && results.length === 1 && !results[0].ok) {
      console.log(`\n✖ Unreachable: ${BASE} — is LocalMind running? Start LocalMind.exe and retry.`);
      return 1;
    }
  }

  // --- 2. favicons (200 or 404 both acceptable) ---
  for (const p of ["/localmind.ico", "/localmind.png"]) {
    const r = await safeGet(p, `GET ${p}`);
    if (r) {
      const ok = r.res.status === 200 || r.res.status === 404;
      report(ok, `GET ${p}`, `status=${r.res.status} (${r.res.status === 200 ? "served" : r.res.status === 404 ? "missing" : "unexpected"})`);
    }
  }

  // --- 3. GET /api/status (need parsed body for check 9) ---
  let statusBody = null;
  {
    const r = await safeGet("/api/status", "GET /api/status");
    if (r) {
      if (isUnauth(r.res)) {
        report(false, "GET /api/status", `status=${r.res.status} ${UNAUTH_MSG} body=${trunc(r.text)}`);
      } else {
        const j = parseJson(r.text);
        const keys = j.ok ? Object.keys(j.value) : [];
        // Documented engine states (process.rs): stopped | starting | running | error.
        const stateOk = j.ok && typeof j.value.status === "string" && ["stopped", "starting", "running", "error"].includes(j.value.status);
        const hasIdle = j.ok && Object.prototype.hasOwnProperty.call(j.value, "idle_remaining_secs");
        // idle_remaining_secs is skip_serializing_if None: ABSENT when the engine is
        // stopped/idle (not null). Accept absent OR a number; present non-number → FAIL.
        const idleVal = j.ok && hasIdle ? j.value.idle_remaining_secs : undefined;
        const idleOk = !hasIdle || typeof idleVal === "number";
        const idleNote = !j.ok ? "n/a" : !hasIdle ? "absent (motor en reposo)" : JSON.stringify(idleVal);
        if (j.ok) statusBody = j.value;
        report(
          r.res.status === 200 && stateOk && idleOk,
          "GET /api/status",
          `status=${r.res.status} keys=[${keys.join(",")}] state=${j.ok ? JSON.stringify(j.value.status) : "n/a"}${stateOk ? "" : " (expected one of stopped|starting|running|error)"} idle_remaining_secs=${idleNote}${!idleOk ? " (expected absent or number)" : ""}${j.ok ? "" : " jsonError=" + j.error}`
        );
      }
    }
  }

  // --- 4. GET /api/hardware ---
  {
    const r = await safeGet("/api/hardware", "GET /api/hardware");
    if (r) {
      const j = parseJson(r.text);
      const gpus = j.ok ? j.value.gpus : undefined;
      const ok = r.res.status === 200 && j.ok && Array.isArray(gpus);
      report(ok, "GET /api/hardware", `status=${r.res.status} gpus=${Array.isArray(gpus) ? "len=" + gpus.length : typeof gpus} cpu=${j.ok ? JSON.stringify(j.value.cpu_name ?? null) : "n/a"}${j.ok ? "" : " jsonError=" + j.error}`);
    }
  }

  // --- 5. GET /api/profiles ---
  {
    const r = await safeGet("/api/profiles", "GET /api/profiles");
    if (r) {
      const j = parseJson(r.text);
      const ok = r.res.status === 200 && j.ok && Array.isArray(j.value) && j.value.length >= 1;
      report(ok, "GET /api/profiles", `status=${r.res.status} len=${j.ok && Array.isArray(j.value) ? j.value.length : "n/a"}${j.ok ? "" : " jsonError=" + (j.error || "")}`);
    }
  }

  // --- 6. GET /api/settings ---
  {
    const r = await safeGet("/api/settings", "GET /api/settings");
    if (r) {
      const j = parseJson(r.text);
      const keys = j.ok ? Object.keys(j.value) : [];
      const ok =
        r.res.status === 200 &&
        j.ok &&
        typeof j.value.config_path === "string" &&
        typeof j.value.http_port === "number" &&
        typeof j.value.llama_port === "number";
      report(ok, "GET /api/settings", `status=${r.res.status} keys=[${keys.join(",")}] config_path=${j.ok ? JSON.stringify(j.value.config_path ?? null) : "n/a"} http_port=${j.ok ? JSON.stringify(j.value.http_port ?? null) : "n/a"} llama_port=${j.ok ? JSON.stringify(j.value.llama_port ?? null) : "n/a"}`);
    }
  }

  // --- 7. GET /api/models ---
  {
    const r = await safeGet("/api/models", "GET /api/models");
    if (r) {
      const j = parseJson(r.text);
      const entriesOk =
        j.ok &&
        Array.isArray(j.value) &&
        j.value.every((m) => typeof m?.name === "string" && typeof m?.size_gb === "number");
      const ok = r.res.status === 200 && entriesOk;
      report(ok, "GET /api/models", `status=${r.res.status} len=${j.ok && Array.isArray(j.value) ? j.value.length : "n/a"} entryKeys=${j.ok && Array.isArray(j.value) && j.value.length ? Object.keys(j.value[0]).join(",") : "n/a"}`);
    }
  }

  // --- 8. GET /api/logs ---
  {
    const r = await safeGet("/api/logs", "GET /api/logs");
    if (r) {
      const j = parseJson(r.text);
      const ok = r.res.status === 200 && j.ok && Array.isArray(j.value) && j.value.length <= 250;
      report(ok, "GET /api/logs", `status=${r.res.status} len=${j.ok && Array.isArray(j.value) ? j.value.length : "n/a"}`);
    }
  }

  // --- 9. GET /api/metrics (branch on observed engine state) ---
  {
    // Mirrors server.rs:328-333: down unless (running-ish and healthy) with a port.
    const engineUp =
      statusBody !== null &&
      (statusBody.status === "running" || statusBody.is_healthy === true) &&
      typeof statusBody.port === "number" &&
      statusBody.port !== 0;
    const r = await safeGet("/api/metrics", "GET /api/metrics");
    if (r) {
      const j = parseJson(r.text);
      if (engineUp) {
        const ok = r.res.status === 200 && j.ok;
        report(ok, "GET /api/metrics", `branch=engine-up(status=${JSON.stringify(statusBody.status)} port=${statusBody.port}) status=${r.res.status} keys=${j.ok ? Object.keys(j.value).join(",") : "n/a"}`);
      } else {
        const ok = r.res.status === 502 && j.ok && j.value.error === "engine_down";
        report(ok, "GET /api/metrics", `branch=engine-down(status=${statusBody ? JSON.stringify(statusBody.status) : "unknown"}) status=${r.res.status} body=${trunc(r.text)}`);
      }
    }
  }

  // --- 10. OPTIONS /api/start (CORS preflight) ---
  {
    let r = null;
    try {
      const ctrl = new AbortController();
      const t = setTimeout(() => ctrl.abort(new Error(`timeout after ${TIMEOUT_MS}ms`)), TIMEOUT_MS);
      try {
        const res = await fetch(BASE + "/api/start", {
          method: "OPTIONS",
          signal: ctrl.signal,
          headers: { ...authHeaders(), Origin: "http://localhost" },
        });
        await res.text();
        r = res;
      } finally {
        clearTimeout(t);
      }
    } catch (e) {
      report(false, "OPTIONS /api/start", `request failed: ${e?.cause?.code || e.message}`);
    }
    if (r) {
      const acao = r.headers.get("access-control-allow-origin");
      const acam = r.headers.get("access-control-allow-methods");
      const acah = r.headers.get("access-control-allow-headers");
      const ok = r.status >= 200 && r.status < 300 && acao !== null;
      report(ok, "OPTIONS /api/start", `status=${r.status} ACAO=${acao} ACAM=${acam} ACAH=${acah}`);
    }
  }

  // --- 11. GET /definitely-not-a-route ---
  {
    const r = await safeGet("/definitely-not-a-route", "GET /definitely-not-a-route");
    if (r) {
      const j = parseJson(r.text);
      const ok = r.res.status === 404 && j.ok && j.value.error === "not_found";
      report(ok, "GET /definitely-not-a-route", `status=${r.res.status} body=${trunc(r.text)}`);
    }
  }

  // --- 12. GET /api/events (SSE, aborted after check) ---
  {
    const ctrl = new AbortController();
    const t = setTimeout(() => ctrl.abort(), SSE_TIMEOUT_MS);
    let outcome = "";
    let ok = false;
    try {
      const res = await fetch(BASE + "/api/events", {
        signal: ctrl.signal,
        headers: { ...authHeaders(), Accept: "text/event-stream" },
      });
      const ct = res.headers.get("content-type") || "";
      if (res.status !== 200 || !ct.includes("text/event-stream")) {
        outcome = `status=${res.status} ct=${ct} (not an SSE stream)`;
        ok = false;
        ctrl.abort();
      } else {
        const reader = res.body.getReader();
        const dec = new TextDecoder();
        let buf = "";
        let gotEvent = false;
        for (;;) {
          const { done, value } = await reader.read();
          if (value) buf += dec.decode(value, { stream: true });
          if (buf.includes("data:")) {
            gotEvent = true;
            break;
          }
          if (done) break;
        }
        ctrl.abort();
        try {
          await reader.cancel();
        } catch {}
        if (gotEvent) {
          const first = buf.slice(buf.indexOf("data:")).split("\n")[0];
          outcome = `branch=event-received status=200 ct=${ct} first=${trunc(first)}`;
          ok = true;
        } else {
          outcome = `branch=open-no-event-yet status=200 ct=${ct} (stream open, no data: line within ${SSE_TIMEOUT_MS}ms)`;
          ok = true;
        }
      }
    } catch (e) {
      if (e?.name === "AbortError" || String(e?.message).includes("aborted")) {
        outcome = `branch=open-no-event-yet (stream open, no data: line within ${SSE_TIMEOUT_MS}ms, aborted)`;
        ok = true;
      } else {
        outcome = `request failed: ${e?.cause?.code || e.message}`;
        ok = false;
      }
    } finally {
      clearTimeout(t);
      ctrl.abort();
    }
    report(ok, "GET /api/events", outcome);
  }

  // --- 13. GET /v1/models (OpenAI model list for CLIs) ---
  {
    const r = await safeGet("/v1/models", "GET /v1/models");
    if (r) {
      const j = parseJson(r.text);
      const ids = j.ok && Array.isArray(j.value?.data) ? j.value.data.map((m) => m?.id) : null;
      if (isUnauth(r.res)) {
        // Key-enforced server + no usable key: 401/403 is the expected branch.
        report(!API_KEY && (r.res.status === 401 || r.res.status === 403), "GET /v1/models", `branch=no-key-unauthenticated status=${r.res.status} ${UNAUTH_MSG} body=${trunc(r.text)}`);
      } else {
        const hasAliases =
          Array.isArray(ids) && ids.includes("localmind") && ids.includes("qwen3.8-27b");
        report(
          r.res.status === 200 && hasAliases,
          "GET /v1/models",
          `branch=model-list status=${r.res.status} ids=${ids ? JSON.stringify(ids) : "n/a"}${hasAliases ? "" : " (expected ids to include localmind + qwen3.8-27b)"}${j.ok ? "" : " jsonError=" + j.error}`
        );
      }
    }
  }

  // --- 14. POST /api/start con cuerpo mal formado (debe ser 400) ---
  {
    // Único POST del script. `{"context":"abc"}` tiene el tipo equivocado: el
    // servidor lo rechaza ANTES de arrancar nada, así que no enciende el motor
    // ni cambia estado. Antes de este check arrancaba con defaults y 200.
    let r = null;
    try {
      r = await post("/api/start", JSON.stringify({ context: "abc" }));
    } catch (e) {
      report(false, "POST /api/start (cuerpo mal formado)", `request failed: ${e?.cause?.code || e.message}`);
    }
    if (r) {
      const j = parseJson(r.text);
      const msg = j.ok && typeof j.value.error === "string" ? j.value.error : "";
      const ok = r.res.status === 400 && msg.includes("JSON inválido");
      report(ok, "POST /api/start (cuerpo mal formado)", `status=${r.res.status} error=${JSON.stringify(trunc(msg))}${ok ? "" : " (expected 400 + error containing 'JSON inválido')"}`);
    }
  }

  if (WRITE_FIXTURES) {
    await writeFixtures();
  }

  const failed = results.filter((r) => !r.ok);
  console.log(`\n== SUMMARY: ${results.length - failed.length}/${results.length} passed ==`);
  let code = 0;
  if (failed.length) {
    code = 1;
    console.log("Failures (raw observed, bodies truncated to 400 chars):");
    for (const f of failed) console.log(`- ${f.check}: ${trunc(f.observed)}`);
  }
  return code;
}

// Capture raw bodies into tests/fixtures/<name>.json (pretty, 2 spaces).
// Never writes partial/invalid files: body must JSON.parse and return 200
// first; write goes to a temp file + atomic rename. Refuses to overwrite
// an existing fixture unless --force. Fixture capture never affects the
// check results or the exit code.
async function writeFixtures() {
  const targets = [
    { path: "/api/status", file: "status.json" },
    { path: "/api/hardware", file: "hardware.json" },
    { path: "/api/profiles", file: "profiles.json" },
    { path: "/api/settings", file: "settings.json" },
    { path: "/api/models", file: "models.json" },
    { path: "/v1/models", file: "v1-models.json" },
  ];
  console.log(`\n== FIXTURES (${FORCE ? "--force: overwriting" : "no overwrite without --force"}) ==`);
  for (const t of targets) {
    const dest = join(FIXTURE_DIR, t.file);
    if (existsSync(dest) && !FORCE) {
      console.log(`SKIP ${t.file} — exists (pass --force to overwrite)`);
      continue;
    }
    let body;
    try {
      const r = await get(t.path);
      if (isUnauth(r.res)) {
        console.log(`SKIP ${t.file} — status=${r.res.status} ${UNAUTH_MSG}`);
        continue;
      }
      if (r.res.status !== 200) {
        console.log(`SKIP ${t.file} — status=${r.res.status} body=${trunc(r.text)}`);
        continue;
      }
      body = r.text;
    } catch (e) {
      console.log(`SKIP ${t.file} — request failed: ${e?.cause?.code || e.message}`);
      continue;
    }
    let parsed;
    try {
      parsed = JSON.parse(body);
    } catch (e) {
      console.log(`SKIP ${t.file} — invalid JSON: ${e.message} body=${trunc(body)}`);
      continue;
    }
    try {
      mkdirSync(FIXTURE_DIR, { recursive: true });
      const tmp = dest + ".tmp";
      writeFileSync(tmp, JSON.stringify(parsed, null, 2) + "\n");
      renameSync(tmp, dest);
      console.log(`WROTE ${dest}`);
    } catch (e) {
      console.log(`SKIP ${t.file} — write failed: ${e.message}`);
    }
  }
}

process.exit(await main());
