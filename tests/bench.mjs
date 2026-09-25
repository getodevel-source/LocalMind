#!/usr/bin/env node
// LocalMind performance benchmark harness (Node ESM, no dependencies, Node >= 18).
// Measures engine throughput per context profile so the "30 t/s at 262K" target
// (SRS LM-NF-1 / LM-NF-2) can be pursued with evidence instead of impressions.
//
// Contract: docs/SRS.md §3.2/§4 + src-rust/src/server.rs + src-rust/src/process.rs.
// Key resolution and timeout style copied from tests/smoke.mjs:
//   env LM_KEY, else %APPDATA%/LocalMind/gateway.key (trimmed); global fetch,
//   AbortSignal timeouts, no dependencies.
//
// Usage:
//   node tests/bench.mjs --profiles 32k,64k,128k,262k [--prompt-tokens 512]
//     [--runs 1] [--max-tokens 256] [--base-url http://127.0.0.1:17860]
//     [--out tests/bench-results] [--dry-run] [--self-test] [--stop-after]

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

// ---------------------------------------------------------------------------
// Pure helpers (no I/O; unit-asserted by --self-test)
// ---------------------------------------------------------------------------

// CLI shorthand -> real engine profile id (src-rust/src/config.rs
// built_in_profiles) + expected context size. The engine resolves unknown
// profile ids by silently falling back to the first profile (profiles.rs),
// so the harness MUST translate here and never send a raw "32k" id.
const PROFILE_MAP = {
  "32k": { id: "velocidad", context: 32768 },
  "64k": { id: "multi_doc", context: 65536 },
  "128k": { id: "libros", context: 131072 },
  "262k": { id: "max_contexto", context: 262144 },
};

function normalizeProfile(label) {
  const k = String(label || "").trim().toLowerCase().replace(/\s+/g, "");
  const short = k.endsWith("k") ? k : `${k}k`;
  const hit = PROFILE_MAP[short];
  if (!hit) throw new Error(`unknown profile "${label}" (expected one of: 32k, 64k, 128k, 262k)`);
  return { label: short, ...hit };
}

// Rough token estimate: ~4 chars per token (llama.cpp / OpenAI heuristic).
function approxTokens(s) {
  return Math.round(String(s).length / 4);
}

// Deterministic prompt whose approximate token count tracks the request.
// Built from a fixed sentence repeated to ~4 chars per requested token, so
// approxTokens(out) === target within rounding (well inside ±20%).
const PROMPT_SENTENCE =
  "Benchmark datum: the quick brown fox jumps over the lazy dog 0123456789. ";
function buildPrompt(targetTokens) {
  const target = Math.max(1, Math.floor(Number(targetTokens) || 0));
  const targetChars = target * 4;
  const reps = Math.max(1, Math.ceil(targetChars / PROMPT_SENTENCE.length));
  let out = PROMPT_SENTENCE.repeat(reps).slice(0, targetChars);
  return out;
}

function mean(xs) {
  if (!xs.length) return null;
  let s = 0;
  for (const x of xs) s += x;
  return s / xs.length;
}

// Nearest-rank percentile over a pre-sorted ascending array. null when empty.
function percentile(sorted, p) {
  if (!sorted.length) return null;
  const rank = Math.ceil((p / 100) * sorted.length) - 1;
  return sorted[Math.min(Math.max(rank, 0), sorted.length - 1)];
}

// Extract {prompt_tokens, completion_tokens} from one SSE `data:` payload or a
// plain JSON body. Returns nulls when no usable usage object is present.
function usageFromJsonObject(obj) {
  const u = obj && typeof obj === "object" ? obj.usage : null;
  if (!u || typeof u !== "object") return { prompt_tokens: null, completion_tokens: null };
  const p = Number(u.prompt_tokens);
  const c = Number(u.completion_tokens);
  return {
    prompt_tokens: Number.isFinite(p) ? p : null,
    completion_tokens: Number.isFinite(c) ? c : null,
  };
}

function parseSseUsage(text) {
  const lines = String(text).split(/\r?\n/);
  let prompt_tokens = null;
  let completion_tokens = null;
  for (const line of lines) {
    const t = line.trim();
    let payload = null;
    if (t.startsWith("data:")) {
      const data = t.slice(5).trim();
      if (!data || data === "[DONE]") continue;
      try {
        payload = JSON.parse(data);
      } catch {
        continue;
      }
    } else if (t.startsWith("{")) {
      try {
        payload = JSON.parse(t);
      } catch {
        continue;
      }
    }
    if (payload) {
      const u = usageFromJsonObject(payload);
      if (u.prompt_tokens !== null) prompt_tokens = u.prompt_tokens;
      if (u.completion_tokens !== null) completion_tokens = u.completion_tokens;
    }
  }
  return { prompt_tokens, completion_tokens };
}

// True when a data-chunk carries generated text (llama.cpp OpenAI SSE shape).
function chunkText(obj) {
  try {
    const ch = obj && obj.choices && obj.choices[0];
    if (!ch) return "";
    if (ch.delta && typeof ch.delta.content === "string") return ch.delta.content;
    if (typeof ch.text === "string") return ch.text;
    if (ch.message && typeof ch.message.content === "string") return ch.message.content;
  } catch {}
  return "";
}

// Cap for the raw per-read timing array kept in the JSON output.
const MAX_SAMPLES = 500;

// Pure reduction of a synthetic chunk timeline to probe metrics (no I/O;
// unit-asserted by --self-test). `samples` is [{t_ms, bytes, has_delta}].
// `buffered` is true when the whole response arrived in a single non-empty
// read — first/last cannot be separated honestly, so rates stay null.
function summarizeProbe(samples, { prompt_tokens = null, completion_tokens = null } = {}) {
  const nonEmpty = samples.filter((s) => s.bytes > 0);
  const deltas = samples.filter((s) => s.has_delta);
  const buffered = nonEmpty.length <= 1;
  const ttft_ms = deltas.length ? deltas[0].t_ms : null;
  const last_token_ms = deltas.length ? deltas[deltas.length - 1].t_ms : null;
  let decode_tps = null;
  let prefill_tps = null;
  if (!buffered && completion_tokens !== null && ttft_ms !== null && last_token_ms !== null) {
    const secs = (last_token_ms - ttft_ms) / 1000;
    if (secs > 0) decode_tps = completion_tokens / secs;
  }
  if (!buffered && prompt_tokens !== null && ttft_ms !== null && ttft_ms > 0) {
    prefill_tps = prompt_tokens / (ttft_ms / 1000);
  }
  return { ttft_ms, last_token_ms, decode_tps, prefill_tps, buffered };
}

// Never print undefined/NaN: missing values render as "n/a".
function fmt(v, digits = 2) {
  if (v === null || v === undefined) return "n/a";
  if (typeof v === "number") {
    if (!Number.isFinite(v)) return "n/a";
    return v.toFixed(digits);
  }
  return String(v);
}
function fmtInt(v) {
  if (v === null || v === undefined) return "n/a";
  if (typeof v === "number" && Number.isFinite(v)) return String(Math.round(v));
  return String(v);
}

// ---------------------------------------------------------------------------
// Compare mode (pure; unit-asserted by --self-test)
// ---------------------------------------------------------------------------

// Metrics compared by --compare: key in a bench row, label for the table,
// and which direction counts as "better".
const COMPARE_METRICS = [
  { key: "load_secs", label: "load_s", higherBetter: false, digits: 1 },
  { key: "ttft_ms", label: "ttft_ms", higherBetter: false, digits: 0 },
  { key: "decode_tps", label: "decode_tps", higherBetter: true, digits: 2 },
  { key: "prefill_tps", label: "prefill_tps", higherBetter: true, digits: 2 },
  { key: "engine_gen_tps", label: "engine_gen_tps", higherBetter: true, digits: 2 },
  { key: "acceptance_decode_tps", label: "acceptance_decode_tps", higherBetter: true, digits: 2 },
];

const TIE_FRACTION = 0.02; // ±2% counts as a tie

// Finite number or null — never NaN/undefined downstream.
function numOrNull(v) {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

// Compare one metric between rows A and B. Returns
// {a, b, delta, pct, verdict} where delta = b−a, pct = 100·(b−a)/|a|,
// verdict ∈ "A" | "B" | "tie" | "n/a" (n/a when either side is missing).
function compareMetric(aRow, bRow, spec) {
  const a = numOrNull(aRow ? aRow[spec.key] : null);
  const b = numOrNull(bRow ? bRow[spec.key] : null);
  if (a === null || b === null) return { a, b, delta: null, pct: null, verdict: "n/a" };
  const delta = b - a;
  let pct = null;
  let tie = false;
  if (a === 0) {
    tie = b === 0;
    pct = b === 0 ? 0 : null; // % change from zero is undefined
  } else {
    pct = (delta / Math.abs(a)) * 100;
    tie = Math.abs(delta) / Math.abs(a) <= TIE_FRACTION;
  }
  let verdict;
  if (tie) verdict = "tie";
  else if (delta === 0) verdict = "tie";
  else verdict = spec.higherBetter ? (delta > 0 ? "B" : "A") : (delta < 0 ? "B" : "A");
  return { a, b, delta, pct, verdict };
}

// Index bench rows by profile label; skips entries without a usable label.
function indexRowsByProfile(doc) {
  const map = new Map();
  const rows = doc && Array.isArray(doc.rows) ? doc.rows : [];
  for (const r of rows) {
    if (!r || typeof r.profile !== "string" || !r.profile) continue;
    if (!map.has(r.profile)) map.set(r.profile, r);
  }
  return map;
}

// Union of profiles across both docs: A's order first, then B-only extras.
// Robust to missing rows arrays and mismatched key orders.
function compareResults(aDoc, bDoc) {
  const aMap = indexRowsByProfile(aDoc);
  const bMap = indexRowsByProfile(bDoc);
  const profiles = [...aMap.keys()];
  for (const p of bMap.keys()) if (!aMap.has(p)) profiles.push(p);
  return profiles.map((profile) => {
    const aRow = aMap.get(profile) || null;
    const bRow = bMap.get(profile) || null;
    const metrics = {};
    for (const spec of COMPARE_METRICS) metrics[spec.key] = compareMetric(aRow, bRow, spec);
    return { profile, inA: aRow !== null, inB: bRow !== null, metrics };
  });
}

function fmtSigned(v, digits = 2) {
  if (v === null || v === undefined) return "n/a";
  if (typeof v !== "number" || !Number.isFinite(v)) return "n/a";
  return (v >= 0 ? "+" : "") + v.toFixed(digits);
}

function fmtPct(v) {
  if (v === null || v === undefined) return "n/a";
  if (typeof v !== "number" || !Number.isFinite(v)) return "n/a";
  return (v >= 0 ? "+" : "") + v.toFixed(1) + "%";
}

// ---------------------------------------------------------------------------
// --self-test: pure-helper assertions, no app needed
// ---------------------------------------------------------------------------

function selfTest() {
  let failures = 0;
  const check = (name, cond, detail = "") => {
    console.log(`${cond ? "PASS" : "FAIL"} ${name}${detail ? ` — ${detail}` : ""}`);
    if (!cond) failures += 1;
  };

  // 1-4. Profile -> context mapping.
  for (const [label, ctx] of [["32k", 32768], ["64k", 65536], ["128k", 131072], ["262k", 262144]]) {
    let got = null;
    try {
      got = normalizeProfile(label).context;
    } catch {}
    check(`profile ${label} -> context ${ctx}`, got === ctx, `got ${got}`);
  }
  // 5. Unknown profiles rejected.
  let rejected = false;
  try {
    normalizeProfile("turbo");
  } catch {
    rejected = true;
  }
  check("unknown profile rejected", rejected, rejected ? "threw as expected" : "did NOT throw");
  // 5b. Real engine ids are resolved (never sent raw to /api/start).
  check(
    "32k resolves to engine id velocidad",
    normalizeProfile("32k").id === "velocidad",
    `got ${normalizeProfile("32k").id}`
  );

  // 6. Prompt builder within ±20% of the requested size.
  for (const target of [64, 512]) {
    const p = buildPrompt(target);
    const est = approxTokens(p);
    const lo = target * 0.8;
    const hi = target * 1.2;
    check(
      `prompt ~${target} tokens within ±20%`,
      typeof p === "string" && est >= lo && est <= hi,
      `estimated ${est} tokens for target ${target}`
    );
  }

  // 7-8. Percentile/mean math on a canned sample.
  const sample = [10, 20, 30, 40, 50];
  check("mean([10,20,30,40,50]) === 30", mean(sample) === 30, `got ${mean(sample)}`);
  check("p50 nearest-rank === 30", percentile(sample, 50) === 30, `got ${percentile(sample, 50)}`);
  check("p100 === 50", percentile(sample, 100) === 50, `got ${percentile(sample, 100)}`);
  check("mean([]) === null", mean([]) === null, `got ${mean([])}`);

  // 9. SSE usage parser extracts tokens from a canned final chunk.
  const canned =
    `data: {"id":"chatcmpl-1","choices":[{"delta":{"content":"hi"}}]}\n\n` +
    `data: {"id":"chatcmpl-1","choices":[],"usage":{"prompt_tokens":512,"completion_tokens":256}}\n\n` +
    `data: [DONE]\n`;
  const u = parseSseUsage(canned);
  check(
    "SSE usage parser extracts prompt/completion tokens",
    u.prompt_tokens === 512 && u.completion_tokens === 256,
    `got prompt=${u.prompt_tokens} completion=${u.completion_tokens}`
  );

  // 10. Nulls when usage is absent.
  const empty = parseSseUsage(`data: {"choices":[{"delta":{"content":"hi"}}]}\n\ndata: [DONE]\n`);
  check(
    "SSE usage parser returns nulls when absent",
    empty.prompt_tokens === null && empty.completion_tokens === null,
    `got prompt=${empty.prompt_tokens} completion=${empty.completion_tokens}`
  );
  const nonJson = parseSseUsage("not json at all");
  check(
    "SSE usage parser returns nulls for non-JSON",
    nonJson.prompt_tokens === null && nonJson.completion_tokens === null,
    `got prompt=${nonJson.prompt_tokens} completion=${nonJson.completion_tokens}`
  );

  // 11. fmt never leaks undefined/NaN.
  check("fmt guards NaN/undefined", fmt(NaN) === "n/a" && fmt(undefined) === "n/a", `got ${fmt(NaN)}/${fmt(undefined)}`);

  // 12. Synthetic timeline: deltas at 1200/1500/1800 ms, 100 completion tokens.
  {
    const tl = [
      { t_ms: 1200, bytes: 120, has_delta: true },
      { t_ms: 1500, bytes: 140, has_delta: true },
      { t_ms: 1800, bytes: 60, has_delta: true },
    ];
    const m = summarizeProbe(tl, { prompt_tokens: 512, completion_tokens: 100 });
    const okDecode = m.decode_tps !== null && Math.abs(m.decode_tps - 100 / 0.6) < 0.01;
    const okPrefill = m.prefill_tps !== null && Math.abs(m.prefill_tps - 512 / 1.2) < 0.01;
    check(
      "timeline 1200/1500/1800ms x100tok -> ttft 1200, decode ~166.7 t/s",
      m.ttft_ms === 1200 && m.last_token_ms === 1800 && !m.buffered && okDecode && okPrefill,
      `got ttft=${m.ttft_ms} last=${m.last_token_ms} decode=${m.decode_tps} prefill=${m.prefill_tps} buffered=${m.buffered}`
    );
  }

  // 13. Single-shot response: buffered:true, null rates.
  {
    const m = summarizeProbe(
      [{ t_ms: 2500, bytes: 4096, has_delta: true }],
      { prompt_tokens: 512, completion_tokens: 100 }
    );
    check(
      "single read -> buffered:true, null rates",
      m.buffered === true && m.decode_tps === null && m.prefill_tps === null,
      `got buffered=${m.buffered} decode=${m.decode_tps} prefill=${m.prefill_tps}`
    );
  }

  // 14. Usage arriving in a later read after the deltas still counts,
  // and comment-only reads do not move first/last.
  {
    const tl = [
      { t_ms: 1000, bytes: 5, has_delta: false },
      { t_ms: 1200, bytes: 120, has_delta: true },
      { t_ms: 1800, bytes: 90, has_delta: true },
      { t_ms: 1850, bytes: 110, has_delta: false },
    ];
    const m = summarizeProbe(tl, { prompt_tokens: 64, completion_tokens: 50 });
    const okDecode = m.decode_tps !== null && Math.abs(m.decode_tps - 50 / 0.6) < 0.01;
    check(
      "usage-after-deltas counted, comments ignored",
      m.ttft_ms === 1200 && m.last_token_ms === 1800 && !m.buffered && okDecode,
      `got ttft=${m.ttft_ms} last=${m.last_token_ms} decode=${m.decode_tps} buffered=${m.buffered}`
    );
    const evt =
      `data: {"choices":[{"delta":{"content":"a"}}]}\n\n` +
      `data: {"choices":[],"usage":{"prompt_tokens":64,"completion_tokens":50}}\n\n` +
      `data: [DONE]\n`;
    const uu = parseSseUsage(evt);
    check(
      "usage chunk after deltas parsed",
      uu.prompt_tokens === 64 && uu.completion_tokens === 50,
      `got prompt=${uu.prompt_tokens} completion=${uu.completion_tokens}`
    );
  }

  // 15. Compare: clearly better run wins rate columns (gen 34 vs 30 t/s),
  // lower load/ttft wins the latency columns.
  {
    const a = { rows: [{ profile: "32k", load_secs: 24, ttft_ms: 2000, decode_tps: 30, prefill_tps: 10, engine_gen_tps: 30, acceptance_decode_tps: 32 }] };
    const b = { rows: [{ profile: "32k", load_secs: 20, ttft_ms: 1500, decode_tps: 35, prefill_tps: 12, engine_gen_tps: 34, acceptance_decode_tps: 36 }] };
    const [row] = compareResults(a, b);
    const m = row.metrics;
    const okDelta = m.engine_gen_tps.delta === 4 && Math.abs(m.engine_gen_tps.pct - (4 / 30) * 100) < 1e-9;
    check(
      "compare better run wins (gen 34 vs 30)",
      row.profile === "32k" && m.engine_gen_tps.verdict === "B" && m.decode_tps.verdict === "B" &&
        m.load_secs.verdict === "B" && m.ttft_ms.verdict === "B" && okDelta,
      `got gen=${m.engine_gen_tps.verdict}Δ${m.engine_gen_tps.delta} load=${m.load_secs.verdict} ttft=${m.ttft_ms.verdict}`
    );
  }

  // 16. Compare: within ±2% is a tie.
  {
    const a = { rows: [{ profile: "64k", load_secs: 20, ttft_ms: 1500, decode_tps: 30, prefill_tps: 10, engine_gen_tps: 30, acceptance_decode_tps: 32 }] };
    const b = { rows: [{ profile: "64k", load_secs: 20.2, ttft_ms: 1515, decode_tps: 30.3, prefill_tps: 10.1, engine_gen_tps: 30.3, acceptance_decode_tps: 32.3 }] };
    const [row] = compareResults(a, b);
    const allTie = COMPARE_METRICS.every((s) => row.metrics[s.key].verdict === "tie");
    check(
      "compare within 2% is a tie",
      allTie,
      `got ${COMPARE_METRICS.map((s) => `${s.key}=${row.metrics[s.key].verdict}`).join(" ")}`
    );
  }

  // 17. Compare: missing metric -> n/a verdict, never NaN; one-sided
  // profiles reported; key order does not matter.
  {
    const a = { rows: [{ profile: "32k", engine_gen_tps: 30 }, { profile: "only-a", engine_gen_tps: 10 }] };
    const b = { rows: [{ profile: "only-b", engine_gen_tps: 11 }, { profile: "32k" }] };
    const rows = compareResults(a, b);
    const r32 = rows.find((r) => r.profile === "32k");
    const rA = rows.find((r) => r.profile === "only-a");
    const rB = rows.find((r) => r.profile === "only-b");
    const g32 = r32.metrics.engine_gen_tps;
    check(
      "compare missing metric is n/a, one-sided profiles kept, order-proof",
      rows.length === 3 && rows[0].profile === "32k" && g32.verdict === "n/a" &&
        g32.delta === null && g32.pct === null && rA.inA && !rA.inB && !rB.inA && rB.inB &&
        rA.metrics.engine_gen_tps.verdict === "n/a",
      `got profiles=${rows.map((r) => r.profile).join(",")} gen32=${g32.verdict} onlyA=inA:${rA.inA}/inB:${rA.inB}`
    );
  }

  if (failures > 0) {
    console.log(`\nself-test: ${failures} assertion(s) FAILED`);
    process.exit(1);
  }
  console.log("\nself-test: all assertions PASS");
}

// ---------------------------------------------------------------------------
// HTTP layer (copied style from tests/smoke.mjs)
// ---------------------------------------------------------------------------

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

const TIMEOUT_MS = 10_000;
const POLL_INTERVAL_MS = 2000;
const START_WAIT_MS = 30 * 60 * 1000; // 30 min: cold 262K loads take minutes
const STOP_WAIT_MS = 5 * 60 * 1000;
const CHAT_TIMEOUT_MS = 30 * 60 * 1000; // generation of max_tokens takes minutes

function isUnreachable(err) {
  const msg = String((err && err.message) || err || "");
  const code = (err && err.cause && err.cause.code) || "";
  return (
    code === "ECONNREFUSED" ||
    code === "ECONNRESET" ||
    msg.includes("fetch failed") ||
    msg.includes("ECONNREFUSED")
  );
}

function unreachableExit(baseUrl) {
  console.log(`LocalMind is unreachable at ${baseUrl} (is LocalMind running?)`);
  console.log("Bench aborted: the app must be running. The engine was not touched.");
  process.exit(1);
}

function makeClient(baseUrl, apiKey) {
  const auth = apiKey ? { Authorization: `Bearer ${apiKey}` } : {};
  async function call(path, { method = "GET", body, timeout = TIMEOUT_MS } = {}) {
    const ctrl = new AbortController();
    const t = setTimeout(() => ctrl.abort(new Error(`timeout after ${timeout}ms`)), timeout);
    try {
      const res = await fetch(baseUrl + path, {
        method,
        signal: ctrl.signal,
        headers: {
          ...auth,
          ...(body !== undefined ? { "Content-Type": "application/json" } : {}),
        },
        body: body !== undefined ? JSON.stringify(body) : undefined,
      });
      const text = await res.text();
      let json = null;
      try {
        json = JSON.parse(text);
      } catch {}
      return { status: res.status, text, json };
    } finally {
      clearTimeout(t);
    }
  }
  return { call };
}

async function getStatus(client) {
  const r = await client.call("/api/status");
  if (r.status !== 200 || !r.json) throw new Error(`GET /api/status -> ${r.status} ${r.text.slice(0, 200)}`);
  return r.json;
}

// Poll every 2 s until done() returns a value, or throw on timeoutMs.
async function pollEvery(prompt, intervalMs, timeoutMs, fn) {
  const start = Date.now();
  for (;;) {
    const v = await fn();
    if (v !== null && v !== undefined) return v;
    if (Date.now() - start > timeoutMs) throw new Error(`${prompt}: timeout after ${Math.round(timeoutMs / 1000)}s`);
    await new Promise((r) => setTimeout(r, intervalMs));
  }
}

// Streaming chat completion read incrementally. The app proxy now streams
// upstream SSE chunk-by-chunk (server.rs `proxy::TeeLogReader`: pass-through
// reads, no full-body buffering), so each `reader.read()` arrival is timed
// and per-read samples [{t_ms, bytes, has_delta}] feed summarizeProbe().
// Returns token counts from the final usage chunk plus ttft/last/decode/prefill.
async function streamChat(baseUrl, apiKey, { prompt, maxTokens, timeoutMs }) {
  const auth = apiKey ? { Authorization: `Bearer ${apiKey}` } : {};
  const ctrl = new AbortController();
  const t = setTimeout(() => ctrl.abort(new Error(`timeout after ${timeoutMs}ms`)), timeoutMs);
  const body = JSON.stringify({
    model: "localmind",
    messages: [{ role: "user", content: prompt }],
    max_tokens: maxTokens,
    temperature: 0,
    stream: true,
    stream_options: { include_usage: true },
  });
  const sendAt = Date.now();
  const samples = [];
  let samples_truncated = false;
  const pushSample = (now, bytes, hasDelta) => {
    if (samples.length >= MAX_SAMPLES) {
      samples_truncated = true;
      return;
    }
    samples.push({ t_ms: now - sendAt, bytes, has_delta: hasDelta });
  };
  let prompt_tokens = null;
  let completion_tokens = null;
  let chunkCount = 0;
  let readCount = 0;
  // Parse complete lines; returns true if any carried a content delta.
  // Usage tokens are picked up wherever they arrive (deltas or final chunk).
  const scanLines = (lines) => {
    let sawDelta = false;
    for (const line of lines) {
      const tr = line.trim();
      if (!tr.startsWith("data:")) continue;
      const data = tr.slice(5).trim();
      if (!data || data === "[DONE]") continue;
      let obj = null;
      try {
        obj = JSON.parse(data);
      } catch {
        continue;
      }
      chunkCount += 1;
      if (chunkText(obj)) sawDelta = true;
      const u = usageFromJsonObject(obj);
      if (u.prompt_tokens !== null) prompt_tokens = u.prompt_tokens;
      if (u.completion_tokens !== null) completion_tokens = u.completion_tokens;
    }
    return sawDelta;
  };
  try {
    const res = await fetch(`${baseUrl}/v1/chat/completions`, {
      method: "POST",
      signal: ctrl.signal,
      headers: { ...auth, "Content-Type": "application/json", Accept: "text/event-stream" },
      body,
    });
    if (res.status === 401 || res.status === 403) {
      throw new Error(
        `POST /v1/chat/completions -> ${res.status} unauthorized — set LM_KEY or ensure %APPDATA%/LocalMind/gateway.key exists`
      );
    }
    if (res.status !== 200 || !res.body) {
      const text = await res.text().catch(() => "");
      throw new Error(`POST /v1/chat/completions -> ${res.status} ${String(text).slice(0, 200)}`);
    }
    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buf = "";
    for (;;) {
      const { done, value } = await reader.read();
      const now = Date.now();
      if (value && value.length) {
        readCount += 1;
        buf += decoder.decode(value, { stream: !done });
        const parts = buf.split("\n");
        buf = parts.pop();
        const sawDelta = scanLines(parts);
        pushSample(now, value.length, sawDelta);
      }
      if (done) break;
    }
    const endAt = Date.now();
    if (buf.trim()) {
      // Trailing bytes without a final newline: parse as one last read.
      readCount += 1;
      const sawDelta = scanLines([buf]);
      pushSample(endAt, buf.length, sawDelta);
      buf = "";
    }
    const m = summarizeProbe(samples, { prompt_tokens, completion_tokens });
    const decode_secs =
      m.ttft_ms !== null && m.last_token_ms !== null ? (m.last_token_ms - m.ttft_ms) / 1000 : null;
    return {
      sendAt,
      endAt,
      readCount,
      chunkCount,
      prompt_tokens,
      completion_tokens,
      ttft_ms: m.ttft_ms,
      last_token_ms: m.last_token_ms,
      decode_secs,
      decode_tps: m.decode_tps,
      prefill_tps: m.prefill_tps,
      buffered: m.buffered,
      samples,
      samples_truncated,
    };
  } finally {
    clearTimeout(t);
  }
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

function printHelp() {
  console.log(`LocalMind performance benchmark harness
Measures engine throughput per context profile (SRS LM-NF-1 / LM-NF-2).

Usage:
  node tests/bench.mjs --profiles 32k,64k,128k,262k [--prompt-tokens 512]
    [--runs 1] [--max-tokens 256] [--base-url http://127.0.0.1:17860]
    [--out tests/bench-results] [--dry-run] [--self-test] [--stop-after]
  node tests/bench.mjs --compare <a.json> <b.json>

Options:
  --profiles      Comma list of: 32k, 64k, 128k, 262k (run in the given order).
  --prompt-tokens Approximate prompt size in tokens (default 512).
  --runs          Streaming completions per profile (default 1; all raw
                  samples are stored, the table row shows the last run).
  --max-tokens    max_tokens for the probe completion (default 256).
  --base-url      LocalMind app URL (default http://127.0.0.1:17860).
  --out           Results directory (default tests/bench-results).
  --dry-run       Print the plan per profile; issue no requests, write nothing.
  --stop-after    Stop the engine when the run finishes (default: leave the
                  engine running with the LAST profile).
  --compare a b  Compare two bench result files (B−A deltas + % change per
                  profile, verdict per metric); needs exactly two files.
  --help          Print this help.

Per profile the harness: waits out "starting", stops a mismatched running
engine, POSTs /api/start with the resolved engine profile id, polls /api/status
until running + acceptance_ok, sends a streaming chat probe, reads /api/metrics,
and records honest rates (null when tokens/timings are missing).

The app proxy streams upstream SSE incrementally (server.rs TeeLogReader),
so per-read arrival times give real TTFT/decode numbers. If a response
genuinely arrives in one read, the row is flagged buffered and the rate
columns stay n/a instead of silently writing nulls.`);
}

function parseArgs(argv) {
  const o = {
    profiles: ["32k", "64k", "128k", "262k"],
    promptTokens: 512,
    runs: 1,
    maxTokens: 256,
    baseUrl: "http://127.0.0.1:17860",
    out: "tests/bench-results",
    dryRun: false,
    selfTest: false,
    stopAfter: false,
    compare: null,
    help: false,
  };
  const args = argv.slice(2);
  for (let i = 0; i < args.length; i += 1) {
    const a = args[i];
    const next = () => {
      const v = args[i + 1];
      if (v === undefined) throw new Error(`missing value for ${a}`);
      i += 1;
      return v;
    };
    if (a === "--profiles") o.profiles = next().split(",").map((s) => s.trim()).filter(Boolean);
    else if (a === "--prompt-tokens") o.promptTokens = Number(next());
    else if (a === "--runs") o.runs = Number(next());
    else if (a === "--max-tokens") o.maxTokens = Number(next());
    else if (a === "--base-url") o.baseUrl = next().replace(/\/+$/, "");
    else if (a === "--out") o.out = next();
    else if (a === "--dry-run") o.dryRun = true;
    else if (a === "--self-test") o.selfTest = true;
    else if (a === "--stop-after") o.stopAfter = true;
    else if (a === "--compare") {
      const x = next();
      const y = args[i + 1] !== undefined && !args[i + 1].startsWith("--") ? args[++i] : undefined;
      if (y === undefined) throw new Error("--compare needs exactly two files: --compare <a.json> <b.json>");
      o.compare = [x, y];
    }
    else if (a === "--help" || a === "-h") o.help = true;
    else throw new Error(`unknown argument "${a}" (see --help)`);
  }
  return o;
}

function stampName(date = new Date()) {
  const p = (n) => String(n).padStart(2, "0");
  return (
    `${date.getFullYear()}${p(date.getMonth() + 1)}${p(date.getDate())}-` +
    `${p(date.getHours())}${p(date.getMinutes())}${p(date.getSeconds())}`
  );
}

function printTable(rows) {
  const cols = [
    ["profile", (r) => r.profile],
    ["context", (r) => fmtInt(r.context)],
    ["load s", (r) => fmt(r.load_secs, 1)],
    ["prompt tok", (r) => fmtInt(r.prompt_tokens)],
    ["compl tok", (r) => fmtInt(r.completion_tokens)],
    ["TTFT ms", (r) => fmtInt(r.ttft_ms)],
    ["last ms", (r) => fmtInt(r.last_token_ms)],
    ["decode t/s", (r) => fmt(r.decode_tps)],
    ["prefill t/s", (r) => fmt(r.prefill_tps)],
    ["buf?", (r) => (r.buffered === null || r.buffered === undefined ? "n/a" : r.buffered ? "YES" : "no")],
    ["gen_tps", (r) => fmt(r.engine_gen_tps)],
    ["accept tps", (r) => fmt(r.acceptance_decode_tps)],
  ];
  const widths = cols.map(([h, f], i) =>
    Math.max(h.length, ...rows.map((r) => String(f(r)).length), i === 0 ? 7 : 0)
  );
  const line = (cells) => cells.map((c, i) => String(c).padEnd(widths[i])).join("  ");
  console.log(line(cols.map(([h]) => h)));
  console.log(line(cols.map((_, i) => "-".repeat(widths[i]))));
  for (const r of rows) console.log(line(cols.map(([, f]) => f(r))));
}

// Print a B−A comparison table plus a per-profile verdict line.
function printComparison(aPath, bPath, compared) {
  console.log(`compare A=${aPath} B=${bPath} (deltas B−A)`);
  const head = ["profile", ...COMPARE_METRICS.flatMap((s) => [`${s.label} Δ`, `${s.label} %`])];
  const bodyRows = compared.map((row) =>
    [row.profile, ...COMPARE_METRICS.flatMap((s) => {
      const m = row.metrics[s.key];
      return [fmtSigned(m.delta, s.digits), fmtPct(m.pct)];
    })]
  );
  const widths = head.map((h, i) => Math.max(h.length, ...bodyRows.map((r) => r[i].length)));
  const line = (cells) => cells.map((c, i) => String(c).padEnd(widths[i])).join("  ");
  console.log(line(head));
  console.log(line(head.map((_, i) => "-".repeat(widths[i]))));
  for (const r of bodyRows) console.log(line(r));
  for (const row of compared) {
    if (!row.inA || !row.inB) {
      console.log(`verdict ${row.profile}: only in ${row.inA ? "A" : "B"} — no comparison`);
      continue;
    }
    const wins = COMPARE_METRICS.map((s) => `${s.label}: ${row.metrics[s.key].verdict}`).join(", ");
    console.log(`verdict ${row.profile}: ${wins} (higher better for rates, lower better for load/ttft, tie within ±2%)`);
  }
}

function readResultFile(path) {
  let text;
  try {
    text = readFileSync(path, "utf8");
  } catch (e) {
    throw new Error(`cannot read ${path}: ${e.message}`);
  }
  try {
    return JSON.parse(text);
  } catch (e) {
    throw new Error(`cannot parse ${path}: ${e.message}`);
  }
}
// ---------------------------------------------------------------------------
// Main run
// ---------------------------------------------------------------------------

async function main() {
  let opts;
  try {
    opts = parseArgs(process.argv);
  } catch (e) {
    console.log(`error: ${e.message}`);
    process.exit(1);
  }
  if (opts.help) {
    printHelp();
    return;
  }
  if (opts.selfTest) {
    selfTest();
    return;
  }
  if (opts.compare) {
    const [aPath, bPath] = opts.compare;
    let aDoc;
    let bDoc;
    try {
      aDoc = readResultFile(aPath);
    } catch (e) {
      console.log(`error: ${e.message}`);
      process.exit(1);
    }
    try {
      bDoc = readResultFile(bPath);
    } catch (e) {
      console.log(`error: ${e.message}`);
      process.exit(1);
    }
    printComparison(aPath, bPath, compareResults(aDoc, bDoc));
    return;
  }

  // Resolve + validate profiles BEFORE any I/O (dry-run included).
  let profiles;
  try {
    profiles = opts.profiles.map(normalizeProfile);
    if (!profiles.length) throw new Error("--profiles is empty");
  } catch (e) {
    console.log(`error: ${e.message}`);
    process.exit(1);
  }
  if (!Number.isFinite(opts.promptTokens) || opts.promptTokens < 1) {
    console.log("error: --prompt-tokens must be a positive number");
    process.exit(1);
  }
  if (!Number.isFinite(opts.runs) || opts.runs < 1) {
    console.log("error: --runs must be >= 1");
    process.exit(1);
  }
  if (!Number.isFinite(opts.maxTokens) || opts.maxTokens < 1) {
    console.log("error: --max-tokens must be a positive number");
    process.exit(1);
  }

  const prompt = buildPrompt(opts.promptTokens);
  const promptEst = approxTokens(prompt);

  if (opts.dryRun) {
    console.log(`dry-run: base ${opts.baseUrl}, prompt ~${promptEst} tokens, max_tokens ${opts.maxTokens}, runs ${opts.runs}`);
    for (const p of profiles) {
      console.log(`- profile ${p.label} (engine id "${p.id}", context ${p.context}):`);
      console.log(`    1. GET /api/status (wait out "starting"; stop engine if running with another profile/context)`);
      console.log(`    2. POST /api/start {"profile":"${p.id}","context":${p.context}}`);
      console.log(`    3. poll GET /api/status every 2 s until running + acceptance_ok (timeout 30 min)`);
      console.log(`    4. POST /v1/chat/completions stream:true x${opts.runs} (temperature 0, max_tokens ${opts.maxTokens})`);
      console.log(`    5. GET /api/metrics right after the probe`);
      console.log(`    would append row to ${opts.out}/bench-<YYYYMMDD-HHMMSS>.json (new file, never overwrite)`);
    }
    console.log(opts.stopAfter ? "would POST /api/stop at the end (--stop-after)" : "would leave the engine running with the LAST profile");
    console.log("dry-run: no requests issued, nothing written.");
    return;
  }

  const { key: API_KEY, source: KEY_SOURCE } = resolveKey();
  console.log(`key source: ${KEY_SOURCE}`);
  const client = makeClient(opts.baseUrl, API_KEY);

  // First contact: fail fast with a clear message if the app is down.
  let initial;
  try {
    initial = await getStatus(client);
  } catch (e) {
    if (isUnreachable(e)) unreachableExit(opts.baseUrl);
    console.log(`error: cannot read /api/status: ${e.message}`);
    process.exit(1);
  }

  let settings = null;
  try {
    const r = await client.call("/api/settings");
    if (r.status === 200 && r.json) settings = r.json;
  } catch {}

  const startedAt = new Date().toISOString();
  const rows = [];
  const samples = [];
  let anyFailure = false;
  for (const p of profiles) {
    const row = {
      profile: p.label,
      engine_profile_id: p.id,
      context: p.context,
      load_secs: null,
      prompt_tokens: null,
      completion_tokens: null,
      ttft_ms: null,
      last_token_ms: null,
      decode_tps: null,
      prefill_tps: null,
      buffered: null,
      engine_gen_tps: null,
      engine_prompt_tps: null,
      acceptance_decode_tps: null,
      acceptance_ok: null,
      error: null,
      last_error: null,
    };
    console.log(`\n=== profile ${p.label} (engine id "${p.id}", context ${p.context}) ===`);
    try {
      // (1) Reconcile current engine state.
      let st = await getStatus(client);
      if (st.status === "starting") {
        console.log("engine is starting; waiting for it to settle...");
        st = await pollEvery("wait-for-settle", POLL_INTERVAL_MS, START_WAIT_MS, async () => {
          const s = await getStatus(client);
          return s.status === "starting" ? null : s;
        });
      }
      if (st.status === "running" && (st.profile !== p.id || Number(st.context) !== p.context)) {
        console.log(`engine running with profile="${st.profile ?? "?"}" context=${st.context ?? "?"}; stopping...`);
        await client.call("/api/stop", { method: "POST", body: {} });
        await pollEvery("wait-for-stop", POLL_INTERVAL_MS, STOP_WAIT_MS, async () => {
          const s = await getStatus(client);
          return s.status === "stopped" ? s : null;
        });
        st = await getStatus(client);
      }

      // (2) Start with the resolved engine profile id (+ explicit context).
      if (st.status !== "running" || st.profile !== p.id) {
        console.log(`POST /api/start {"profile":"${p.id}","context":${p.context}}`);
        const r = await client.call("/api/start", {
          method: "POST",
          body: { profile: p.id, context: p.context },
          timeout: TIMEOUT_MS,
        });
        if (r.status !== 200) throw new Error(`POST /api/start -> ${r.status} ${r.text.slice(0, 300)}`);

        // (3) Poll until running AND acceptance_ok (timeout 30 min).
        const loadStart = Date.now();
        st = await pollEvery("wait-for-running", POLL_INTERVAL_MS, START_WAIT_MS, async () => {
          const s = await getStatus(client);
          if (s.status === "error") throw new Error(`engine error: ${(s.last_error ?? "unknown").toString().slice(0, 500)}`);
          if (s.status === "running" && s.acceptance_ok === true) return s;
          return null;
        });
        row.load_secs = (Date.now() - loadStart) / 1000;
      } else {
        console.log("engine already running with this profile; reusing (load_secs n/a).");
        row.load_secs = null;
      }
      row.acceptance_decode_tps =
        typeof st.decode_tps === "number" && Number.isFinite(st.decode_tps) ? st.decode_tps : null;
      row.acceptance_ok = st.acceptance_ok === undefined ? null : st.acceptance_ok;

      // (4) Streaming probe(s): per-read arrival timing, honest buffered flag.
      for (let run = 0; run < opts.runs; run += 1) {
        console.log(`probe run ${run + 1}/${opts.runs}: streaming chat completion...`);
        const s = await streamChat(opts.baseUrl, API_KEY, {
          prompt,
          maxTokens: Math.floor(opts.maxTokens),
          timeoutMs: CHAT_TIMEOUT_MS,
        });
        samples.push({
          profile: p.label,
          run: run + 1,
          prompt_tokens: s.prompt_tokens,
          completion_tokens: s.completion_tokens,
          ttft_ms: s.ttft_ms,
          last_token_ms: s.last_token_ms,
          decode_secs: s.decode_secs,
          decode_tps: s.decode_tps,
          prefill_tps: s.prefill_tps,
          buffered: s.buffered,
          reads: s.readCount,
          samples_truncated: s.samples_truncated,
          samples: s.samples,
          ended_at: new Date(s.endAt).toISOString(),
        });
        // The table row shows the last run; nulls (never 0 or invented) when
        // tokens/timings are missing, plus an explicit buffered flag.
        row.prompt_tokens = s.prompt_tokens;
        row.completion_tokens = s.completion_tokens;
        row.ttft_ms = s.ttft_ms;
        row.last_token_ms = s.last_token_ms;
        row.decode_tps = s.decode_tps;
        row.prefill_tps = s.prefill_tps;
        row.buffered = s.buffered;
        if (s.buffered) {
          console.log(
            `  run ${run + 1}: prompt=${fmtInt(s.prompt_tokens)} completion=${fmtInt(s.completion_tokens)} ` +
              `BUFFERED in one read — no per-token timing (n/a columns)`
          );
        } else {
          console.log(
            `  run ${run + 1}: prompt=${fmtInt(s.prompt_tokens)} completion=${fmtInt(s.completion_tokens)} ` +
              `TTFT=${fmtInt(s.ttft_ms)}ms last=${fmtInt(s.last_token_ms)}ms decode=${fmt(s.decode_tps)} t/s`
          );
        }
      }

      // (5) Engine metrics right after the probe.
      try {
        const m = await client.call("/api/metrics");
        if (m.status === 200 && m.json) {
          row.engine_gen_tps =
            typeof m.json.gen_tps === "number" && Number.isFinite(m.json.gen_tps) ? m.json.gen_tps : null;
          row.engine_prompt_tps =
            typeof m.json.prompt_tps === "number" && Number.isFinite(m.json.prompt_tps) ? m.json.prompt_tps : null;
        } else {
          console.log(`GET /api/metrics -> ${m.status} (recorded as absent)`);
        }
      } catch (e) {
        console.log(`GET /api/metrics failed: ${e.message} (recorded as absent)`);
      }
    } catch (e) {
      // Never silently skip: record the failure verbatim, continue.
      row.error = String((e && e.message) || e);
      try {
        const s = await getStatus(client);
        row.last_error = s.last_error === undefined || s.last_error === null ? null : String(s.last_error);
      } catch {}
      anyFailure = true;
      console.log(`profile ${p.label} FAILED: ${row.error}`);
    }
    rows.push(row);
  }

  if (opts.stopAfter) {
    console.log("\n--stop-after: stopping the engine...");
    try {
      await client.call("/api/stop", { method: "POST", body: {} });
    } catch (e) {
      console.log(`POST /api/stop failed: ${e.message}`);
    }
  } else if (rows.length) {
    console.log(`\nLeaving the engine running with the LAST profile (${profiles[profiles.length - 1].label}).`);
  }

  console.log("\n--- results ---");
  printTable(rows);

  // Write the full JSON result (new file, never overwrite).
  mkdirSync(opts.out, { recursive: true });
  const base = `bench-${stampName()}.json`;
  let dest = join(opts.out, base);
  for (let n = 1; existsSync(dest); n += 1) {
    dest = join(opts.out, base.replace(/\.json$/, `-${String(n).padStart(2, "0")}.json`));
  }
  const payload = {
    tool: "tests/bench.mjs",
    started_at: startedAt,
    finished_at: new Date().toISOString(),
    base_url: opts.baseUrl,
    key_source: KEY_SOURCE,
    config: {
      profiles: opts.profiles,
      prompt_tokens_requested: opts.promptTokens,
      prompt_tokens_estimated: promptEst,
      runs: opts.runs,
      max_tokens: opts.maxTokens,
      stop_after: opts.stopAfter,
    },
    settings,
    rows,
    samples,
  };
  writeFileSync(dest, JSON.stringify(payload, null, 2), "utf8");
  console.log(`\nfull result written to ${dest}`);

  if (anyFailure) {
    console.log("bench finished WITH FAILURES (see rows with error above); exit 1.");
    process.exit(1);
  }
}

process.exit(await main().catch((e) => {
  if (isUnreachable(e)) unreachableExit("(base URL)");
  console.log(`fatal: ${(e && e.message) || e}`);
  process.exit(1);
}));
