#!/usr/bin/env node
// LocalMind harness benchmark (Node ESM, no dependencies, Node >= 18).
// Compares the four supported harnesses (pi, omp, opencode, deepseek) on one
// shared engine load: reaction time (spawn -> first output byte), prompt /
// completion tokens from usage.jsonl, and sustained tokens/s.
//
// Usage:
//   node tests/harness-bench.mjs --harnesses pi,omp,opencode,deepseek \
//     --profile libros --context 131072 [--repeats 2] [--reaction]
//     [--throughput] [--dry-run] [--stop-after] [--fair [--thinking low|off]
//     [--warmup]] [--no-control] [--base-url http://127.0.0.1:17860]
//
//   --reaction / --throughput select which phases to run (default: both).
//   --repeats N: throughput repetitions when a run finishes in <180 s
//     (default 2). Slower runs execute once. Ignored under --fair (always 1).
//   --dry-run: print the exact commands/env (key redacted) without starting
//     the engine, writing configs, or spawning anything (never touches net).
//   --stop-after: POST /api/stop when the campaign ends (default: off; the
//     caller stops the engine so one load can serve several invocations).
//   --fair: ~10-minute apples-to-apples window: reversed harness order
//     (deepseek,opencode,omp,pi), explicit pi/omp --thinking (default low),
//     one reaction + one throughput per harness, per-request legs recorded.
//     NOTE: --thinking low|off deviates from the app default (max); fair
//     numbers compare harnesses, not the shipped configuration.
//   --thinking low|off: reasoning level for pi/omp under --fair (default low).
//     opencode/deepseek have no equivalent CLI flag; their runs keep defaults.
//   --warmup: one tiny thinking-off request before each harness so the
//     pipeline is not cold. This CHANGES what the numbers mean (warm TTFT).

import { spawn } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  writeFileSync,
  renameSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const REACTION_PROMPT = "Responde exactamente: OK";
const THROUGHPUT_PROMPT =
  "Escribe los números del 1 al 300, uno por línea, sin texto adicional.";
const REPEAT_WALL_SECS = 180; // runs faster than this are repeated once
const REACTION_TIMEOUT_MS = 10 * 60 * 1000;
const THROUGHPUT_TIMEOUT_MS = 15 * 60 * 1000;
const START_WAIT_MS = 30 * 60 * 1000;
const START_POLL_MS = 15 * 1000;
const MAX_OUT = 128 * 1024; // captured stdout/stderr cap per run
const HARNESS_ORDER = ["pi", "omp", "opencode", "deepseek"];

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

function parseArgs(argv) {
  const o = {
    harnesses: [...HARNESS_ORDER],
    profile: "libros",
    context: 131072,
    repeats: 2,
    reaction: false,
    throughput: false,
    dryRun: false,
    stopAfter: false,
    noControl: false,
    fair: false,
    thinking: "low",
    warmup: false,
    reactionTimeout: 600,
    throughputTimeout: 900,
    baseUrl: "http://127.0.0.1:17860",
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const next = () => argv[++i];
    if (a === "--harnesses") {
      o.harnesses = String(next())
        .split(",")
        .map((s) => s.trim().toLowerCase())
        .filter(Boolean);
      for (const h of o.harnesses) {
        if (!HARNESS_ORDER.includes(h)) throw new Error(`unknown harness "${h}"`);
      }
    } else if (a === "--profile") o.profile = String(next());
    else if (a === "--context") o.context = Number(next());
    else if (a === "--repeats") o.repeats = Math.max(1, Number(next()) | 0);
    else if (a === "--reaction") o.reaction = true;
    else if (a === "--throughput") o.throughput = true;
    else if (a === "--dry-run") o.dryRun = true;
    else if (a === "--stop-after") o.stopAfter = true;
    else if (a === "--no-control") o.noControl = true;
    else if (a === "--fair") o.fair = true;
    else if (a === "--thinking") {
      const v = String(next()).toLowerCase();
      if (!["low", "off"].includes(v)) throw new Error(`--thinking must be low|off, got "${v}"`);
      o.thinking = v;
    } else if (a === "--warmup") o.warmup = true;
    else if (a === "--reaction-timeout") {
      o.reactionTimeout = Math.max(1, Number(next()) | 0);
      if (!Number.isFinite(o.reactionTimeout)) throw new Error("--reaction-timeout needs seconds");
    } else if (a === "--throughput-timeout") {
      o.throughputTimeout = Math.max(1, Number(next()) | 0);
      if (!Number.isFinite(o.throughputTimeout)) throw new Error("--throughput-timeout needs seconds");
    } else if (a === "--base-url") o.baseUrl = String(next()).replace(/\/+$/, "");
  }
  if (!o.reaction && !o.throughput) {
    o.reaction = true;
    o.throughput = true;
  }
  return o;
}

// ---------------------------------------------------------------------------
// Key (same resolution as tests/smoke.mjs; never printed)
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

// Redact anything that looks like the key (or that IS the key) from records.
function redact(s, key) {
  let out = String(s);
  if (key) out = out.split(key).join("[REDACTED]");
  return out;
}

// ---------------------------------------------------------------------------
// Gateway HTTP helpers (global fetch, AbortSignal timeouts)
// ---------------------------------------------------------------------------

const authHeaders = (key) => ({ Authorization: `Bearer ${key}` });

async function apiGet(base, key, path, timeout = 15000) {
  const ctl = new AbortController();
  const t = setTimeout(() => ctl.abort(), timeout);
  try {
    const res = await fetch(base + path, {
      headers: authHeaders(key),
      signal: ctl.signal,
    });
    const text = await res.text();
    return { status: res.status, text };
  } finally {
    clearTimeout(t);
  }
}

async function apiPost(base, key, path, body, timeout = 30000) {
  const ctl = new AbortController();
  const t = setTimeout(() => ctl.abort(), timeout);
  try {
    const res = await fetch(base + path, {
      method: "POST",
      headers: { ...authHeaders(key), "Content-Type": "application/json" },
      body: JSON.stringify(body),
      signal: ctl.signal,
    });
    const text = await res.text();
    return { status: res.status, text };
  } finally {
    clearTimeout(t);
  }
}

function parseJson(text) {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------
// Engine: reuse running load at the requested context; start only if stopped;
// wait (never double-start) while another load is `starting`.
// Returns { reused: bool, load_secs: number|null, decode_tps }.
// ---------------------------------------------------------------------------

async function ensureEngine(base, key, profile, context) {
  const st0 = parseJson((await apiGet(base, key, "/api/status")).text);
  if (
    st0 &&
    st0.status === "running" &&
    Number(st0.context) === Number(context)
  ) {
    return {
      reused: true,
      load_secs: null,
      status: st0,
      note: "reused running load (no restart)",
    };
  }
  if (st0 && st0.status === "starting") {
    // Another worker (or a previous call) is mid-load: wait, never POST.
    const t0 = Date.now();
    for (;;) {
      await sleep(START_POLL_MS);
      const s = parseJson((await apiGet(base, key, "/api/status")).text);
      if (s && s.status === "running" && s.acceptance_ok === true) {
        return { reused: true, load_secs: null, status: s, note: "waited for in-flight load" };
      }
      if (s && s.status !== "starting" && s.status !== "running") {
        throw new Error(`engine left starting state unexpectedly: ${JSON.stringify(s)}`);
      }
      if (Date.now() - t0 > START_WAIT_MS) throw new Error("timeout waiting for engine load (30 min)");
    }
  }
  if (st0 && st0.status === "running") {
    throw new Error(
      `engine already running at context ${st0.context}, requested ${context}; refusing to restart (run with matching --context)`
    );
  }
  // stopped (or unknown): this campaign owns the single load.
  const t0 = Date.now();
  const r = await apiPost(base, key, "/api/start", { profile, context }, 30000);
  if (r.status !== 200) throw new Error(`POST /api/start -> ${r.status}: ${r.text.slice(0, 200)}`);
  for (;;) {
    await sleep(START_POLL_MS);
    const s = parseJson((await apiGet(base, key, "/api/status")).text);
    if (s && s.status === "running" && s.acceptance_ok === true) {
      return { reused: false, load_secs: (Date.now() - t0) / 1000, status: s, note: "started by this campaign" };
    }
    if (s && s.last_error) throw new Error(`engine load failed: ${s.last_error}`);
    if (Date.now() - t0 > START_WAIT_MS) throw new Error("timeout waiting for engine load (30 min)");
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------------------
// Private dirs + per-harness launch specs (mirror launcher.rs / agents.rs)
// ---------------------------------------------------------------------------

function agentsBase() {
  const appdata = process.env.APPDATA;
  if (appdata) return join(appdata, "LocalMind", "agents");
  return join(tmpdir(), "LocalMind-agents");
}

function npmShim(name) {
  const appdata = process.env.APPDATA;
  if (appdata) {
    const p = join(appdata, "npm", `${name}.cmd`);
    if (existsSync(p)) return p;
  }
  return name; // fall back to PATH
}

// agents.rs pi_models_json (vision=false): live port/context/key.
function piModelsJson(httpPort, context, key) {
  const maxToks = Math.min(Math.floor(context / 2), 16384);
  const entry = (id, name) => ({
    id,
    name,
    contextWindow: context,
    maxTokens: maxToks,
    reasoning: true,
    input: ["text"],
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    thinkingLevelMap: { minimal: "low", low: "low", medium: "medium", high: "xhigh", xhigh: "xhigh", max: null },
    compat: { supportsDeveloperRole: false },
  });
  return (
    JSON.stringify(
      {
        providers: {
          localmind: {
            name: "LocalMind",
            baseUrl: `http://127.0.0.1:${httpPort}/v1`,
            apiKey: key,
            api: "openai-completions",
            models: [entry("localmind", "LocalMind Active Model"), entry("qwen3.8-27b", "Qwen 3.8 27B (LocalMind)")],
          },
        },
      },
      null,
      2
    ) + "\n"
  );
}

// agents.rs omp_models_yml (vision=false) + omp_config_yml.
function ompModelsYml(httpPort, context, key) {
  const m = (id, name) =>
    `      - id: ${id}\n        name: ${name}\n        reasoning: true\n        input: [text]\n        contextWindow: ${context}\n        maxTokens: 16384\n        cost:\n          input: 0\n          output: 0\n          cacheRead: 0\n          cacheWrite: 0\n        thinkingLevelMap:\n          minimal: low\n          low: low\n          medium: medium\n          high: xhigh\n          xhigh: xhigh\n          max: null\n        compat:\n          supportsReasoningEffort: false\n          reasoningContentField: reasoning_content\n          supportsDeveloperRole: false\n`;
  return (
    `providers:\n  localmind:\n    baseUrl: http://127.0.0.1:${httpPort}/v1\n    apiKey: ${key}\n    api: openai-completions\n    models:\n` +
    m("qwen3.8-27b", "Qwen 3.8 27B (LocalMind)") +
    m("localmind", "LocalMind Active Model")
  );
}
const OMP_CONFIG_YML = "modelRoles:\n  default: localmind/localmind\n";

// launcher.rs deepseek_profile_patch (model alias `localmind`, maxTokens 4096).
function deepseekPatch(httpPort, context) {
  return (
    `# LocalMind: punto del harness dsh contra el modelo local (generado).\n` +
    `# - Proveedor OpenAI-compatible en http://127.0.0.1:${httpPort}/v1 (protocolo openai-completions).\n` +
    `# - Clave via env LOCALMIND_API_KEY (el lanzador la lee de %APPDATA%\\LocalMind\\gateway.key).\n` +
    `# - Compat: el gateway/proxy LocalMind solo habla chat/completions clasico:\n` +
    `#   sin rol "developer" (usa "system") y cap de salida como "max_tokens".\n` +
    `- id: agent-default-model\n  config:\n    provider: localmind\n    model: localmind\n` +
    `- id: llm-pi-ai\n  config:\n    providers:\n      localmind:\n` +
    `        displayName: LocalMind\n        apiKeyEnv: LOCALMIND_API_KEY\n` +
    `        api: openai-completions\n        baseURL: http://127.0.0.1:${httpPort}/v1\n` +
    `        compat:\n          supportsDeveloperRole: false\n          maxTokensField: max_tokens\n` +
    `          supportsReasoningEffort: false\n        models:\n          - id: localmind\n` +
    `            name: LocalMind local model\n            contextWindow: ${context}\n            maxTokens: 4096\n`
  );
}

// launcher.rs opencode_config_json (inline live key, alias corto).
function opencodeConfigJson(httpPort, context, modelId, key) {
  return (
    JSON.stringify({
      $schema: "https://opencode.ai/config.json",
      model: `localmind/${modelId}`,
      small_model: `localmind/${modelId}`,
      provider: {
        localmind: {
          npm: "@ai-sdk/openai-compatible",
          name: "LocalMind",
          options: { baseURL: `http://127.0.0.1:${httpPort}/v1`, apiKey: key },
          models: {
            [modelId]: {
              name: modelId,
              limit: { context, output: 8192 },
              modalities: { input: ["text"], output: ["text"] },
            },
          },
        },
      },
    }) + "\n"
  );
}

// Write file only when content differs (tmp+rename, like the Rust launcher).
function writeIfChanged(path, content) {
  mkdirSync(dirname(path), { recursive: true });
  try {
    if (existsSync(path) && readFileSync(path, "utf8") === content) return false;
  } catch {}
  const tmp = `${path}.tmp`;
  writeFileSync(tmp, content, "utf8");
  renameSync(tmp, path);
  return true;
}

// Build the per-harness spec: binary, argv tail, env additions, config files.
// `phase` is "reaction" | "throughput" (only selects the prompt).
function harnessSpec(id, prompt, ctx) {
  // ctx: { httpPort, liveContext, key, agentsBase, thinking }
  // thinking: "max" (app default, matches launcher.rs effort_flag default),
  // "low"|"off" (--fair: explicit level for pi/omp --thinking, which accepts
  // off/minimal/low/medium/high/xhigh/max).
  const thinking = ctx.thinking || "max";
  const home = join(ctx.agentsBase, id === "deepseek" ? "deepseek" : id);
  switch (id) {
    case "pi": {
      // launcher.rs cli_inner_cmd(Pi) default: --thinking max; --fair uses low/off.
      const files = [{ path: join(home, "models.json"), content: piModelsJson(ctx.httpPort, ctx.liveContext, ctx.key) }];
      return {
        id,
        bin: npmShim("pi"),
        argv: ["--provider", "localmind", "--model", "localmind/localmind", "--thinking", thinking, "-p", prompt],
        env: {
          OPENAI_BASE_URL: `http://127.0.0.1:${ctx.httpPort}/v1`,
          OPENAI_API_KEY: ctx.key,
          PI_CODING_AGENT_DIR: home,
        },
        files,
        shell: true, // .cmd shim needs a shell (bare spawn -> EINVAL/ENOENT)
      };
    }
    case "omp": {
      // launcher.rs cli_inner_cmd(Omp): --model localmind/qwen3.8-27b
      // --thinking max (+ -p for one-shot).
      const files = [
        { path: join(home, "models.yml"), content: ompModelsYml(ctx.httpPort, ctx.liveContext, ctx.key) },
        { path: join(home, "config.yml"), content: OMP_CONFIG_YML },
      ];
      return {
        id,
        // omp is a real exe (%LOCALAPPDATA%\omp\omp.exe); prefer it, else PATH.
        bin: existsSync(join(process.env.LOCALAPPDATA || "", "omp", "omp.exe"))
          ? join(process.env.LOCALAPPDATA, "omp", "omp.exe")
          : "omp",
        argv: ["--model", "localmind/qwen3.8-27b", "--thinking", thinking, "-p", prompt],
        env: {
          OPENAI_BASE_URL: `http://127.0.0.1:${ctx.httpPort}/v1`,
          OPENAI_API_KEY: ctx.key,
          PI_CODING_AGENT_DIR: home,
        },
        files,
        shell: false,
      };
    }
    case "opencode": {
      // server.rs launch_opencode: config FILE with inline live key
      // (<home>/config/opencode/opencode.json) + XDG isolation + key via env.
      const modelId = "qwen3.8-27b";
      const files = [
        {
          path: join(home, "config", "opencode", "opencode.json"),
          content: opencodeConfigJson(ctx.httpPort, ctx.liveContext, modelId, ctx.key),
        },
      ];
      return {
        id,
        bin: npmShim("opencode"),
        argv: ["--model", `localmind/${modelId}`, "run", prompt],
        env: {
          XDG_DATA_HOME: join(home, "data"),
          XDG_CONFIG_HOME: join(home, "config"),
          XDG_CACHE_HOME: join(home, "cache"),
          XDG_STATE_HOME: join(home, "state"),
          LOCALMIND_API_KEY: ctx.key,
        },
        files,
        shell: true,
      };
    }
    case "deepseek": {
      // server.rs launch_deepseek: generated Cordis patch (live port/ctx) +
      // DSH_HOME isolation + key via env; dsh --profile headless "<task>".
      const files = [
        {
          path: join(home, "profiles", "headless", "cordis.patch.yml"),
          content: deepseekPatch(ctx.httpPort, ctx.liveContext),
        },
      ];
      return {
        id,
        bin: npmShim("dsh"),
        argv: ["--profile", "headless", prompt],
        env: {
          DSH_HOME: home,
          DSH_TELEMETRY_MODE: "DISABLED",
          DSH_PERMISSION_MODE: "workspace-write",
          LOCALMIND_API_KEY: ctx.key,
        },
        files,
        shell: true,
      };
    }
    default:
      throw new Error(`unknown harness "${id}"`);
  }
}

// ---------------------------------------------------------------------------
// usage.jsonl delta helpers
// ---------------------------------------------------------------------------

function usagePath() {
  const appdata = process.env.APPDATA;
  if (appdata) return join(appdata, "LocalMind", "usage.jsonl");
  return null;
}

function usageLineCount() {
  try {
    const p = usagePath();
    if (!p || !existsSync(p)) return 0;
    const t = readFileSync(p, "utf8");
    if (!t) return 0;
    const lines = t.split("\n").filter((l) => l.trim());
    return lines.length;
  } catch {
    return 0;
  }
}

function usageNewLines(since) {
  try {
    const p = usagePath();
    if (!p || !existsSync(p)) return [];
    const lines = readFileSync(p, "utf8").split("\n").filter((l) => l.trim());
    return lines.slice(since).map((l) => {
      try {
        return JSON.parse(l);
      } catch {
        return { _raw: l.slice(0, 200) };
      }
    });
  } catch {
    return [];
  }
}

function sumUsage(lines) {
  let prompt = 0,
    completion = 0,
    ms = 0;
  for (const l of lines) {
    prompt += Number(l.prompt_tokens) || 0;
    completion += Number(l.completion_tokens) || 0;
    ms += Number(l.ms) || 0;
  }
  return { prompt_tokens: prompt, completion_tokens: completion, ms };
}

// ---------------------------------------------------------------------------
// Child runner: spawn -> first output byte (hrtime on chunks, not lines)
// ---------------------------------------------------------------------------

function runChild(spec, cwd, timeoutMs) {
  return new Promise((resolve) => {
    const t0 = process.hrtime.bigint();
    let firstByteMs = null;
    let outBytes = 0,
      errBytes = 0;
    const cap = { out: "", err: "" };
    let child;
    try {
      child = spawn(spec.bin, spec.argv, {
        cwd,
        env: { ...process.env, ...spec.env },
        stdio: ["ignore", "pipe", "pipe"],
        shell: spec.shell,
        windowsHide: true,
      });
    } catch (e) {
      resolve({
        ok: false,
        error: `spawn threw: ${e.message}`,
        wall_ms: Number(process.hrtime.bigint() - t0) / 1e6,
      });
      return;
    }
    const mark = () => {
      if (firstByteMs === null) firstByteMs = Number(process.hrtime.bigint() - t0) / 1e6;
    };
    child.stdout.on("data", (c) => {
      mark();
      outBytes += c.length;
      if (cap.out.length < MAX_OUT) cap.out += c.toString("utf8").slice(0, MAX_OUT - cap.out.length);
    });
    child.stderr.on("data", (c) => {
      mark();
      errBytes += c.length;
      if (cap.err.length < MAX_OUT) cap.err += c.toString("utf8").slice(0, MAX_OUT - cap.err.length);
    });
    child.on("error", (e) => {
      resolve({
        ok: false,
        error: `spawn error: ${e.message}`,
        wall_ms: Number(process.hrtime.bigint() - t0) / 1e6,
        first_byte_ms: firstByteMs,
      });
    });
    const timer = setTimeout(() => {
      try {
        child.kill();
      } catch {}
      resolve({
        ok: false,
        timedOut: true, // wall time is DATA: the harness was still working
        error: `timeout after ${timeoutMs} ms`,
        wall_ms: Number(process.hrtime.bigint() - t0) / 1e6,
        first_byte_ms: firstByteMs,
        stdout_bytes: outBytes,
        stderr_bytes: errBytes,
        stdout_head: cap.out.slice(0, 2000),
        stderr_head: cap.err.slice(0, 2000),
      });
    }, timeoutMs);
    child.on("close", (code, signal) => {
      clearTimeout(timer);
      resolve({
        ok: code === 0,
        exit_code: code,
        signal: signal || null,
        wall_ms: Number(process.hrtime.bigint() - t0) / 1e6,
        first_byte_ms: firstByteMs,
        stdout_bytes: outBytes,
        stderr_bytes: errBytes,
        stdout_head: cap.out.slice(0, 2000),
        stderr_head: cap.err.slice(0, 2000),
      });
    });
  });
}

async function harnessVersion(id, spec) {
  return new Promise((resolve) => {
    let out = "";
    let child;
    try {
      child = spawn(spec.bin, ["--version"], {
        env: { ...process.env },
        stdio: ["ignore", "pipe", "pipe"],
        shell: spec.shell,
        windowsHide: true,
      });
    } catch (e) {
      resolve(`spawn-error: ${e.message}`);
      return;
    }
    child.stdout.on("data", (c) => (out += c.toString("utf8")));
    child.stderr.on("data", (c) => (out += c.toString("utf8")));
    child.on("error", (e) => resolve(`spawn-error: ${e.message}`));
    const timer = setTimeout(() => {
      try {
        child.kill();
      } catch {}
      resolve("timeout");
    }, 60000);
    child.on("close", () => {
      clearTimeout(timer);
      resolve(out.trim().split("\n")[0].slice(0, 80) || "unknown");
    });
  });
}

// ---------------------------------------------------------------------------
// Control: direct streaming request through the gateway (bench.mjs technique)
// ---------------------------------------------------------------------------

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

async function runControl(base, key) {
  const t0 = process.hrtime.bigint();
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), THROUGHPUT_TIMEOUT_MS);
  try {
    const res = await fetch(`${base}/v1/chat/completions`, {
      method: "POST",
      headers: { ...authHeaders(key), "Content-Type": "application/json" },
      body: JSON.stringify({
        model: "qwen3.8-27b",
        messages: [{ role: "user", content: THROUGHPUT_PROMPT }],
        stream: true,
        max_tokens: 4096,
        temperature: 0,
      }),
      signal: ctl.signal,
    });
    if (res.status !== 200) {
      const t = await res.text().catch(() => "");
      return { ok: false, error: `HTTP ${res.status}: ${t.slice(0, 200)}` };
    }
    const reader = res.body.getReader();
    const dec = new TextDecoder();
    let buf = "",
      ttftMs = null,
      lastMs = null,
      promptTokens = null,
      completionTokens = null,
      textChars = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      const nowMs = Number(process.hrtime.bigint() - t0) / 1e6;
      buf += dec.decode(value, { stream: true });
      const parts = buf.split("\n");
      buf = parts.pop();
      for (const line of parts) {
        const t = line.trim();
        if (!t.startsWith("data:")) continue;
        const data = t.slice(5).trim();
        if (!data || data === "[DONE]") continue;
        let obj;
        try {
          obj = JSON.parse(data);
        } catch {
          continue;
        }
        const txt = chunkText(obj);
        if (txt) {
          if (ttftMs === null) ttftMs = nowMs;
          lastMs = nowMs;
          textChars += txt.length;
        }
        if (obj.usage && typeof obj.usage === "object") {
          if (Number.isFinite(Number(obj.usage.prompt_tokens))) promptTokens = Number(obj.usage.prompt_tokens);
          if (Number.isFinite(Number(obj.usage.completion_tokens))) completionTokens = Number(obj.usage.completion_tokens);
        }
      }
    }
    const wallMs = Number(process.hrtime.bigint() - t0) / 1e6;
    const out = {
      ok: true,
      wall_ms: wallMs,
      ttft_ms: ttftMs,
      last_token_ms: lastMs,
      prompt_tokens: promptTokens,
      completion_tokens: completionTokens,
      text_chars: textChars,
    };
    if (ttftMs !== null && lastMs !== null && completionTokens !== null && lastMs > ttftMs) {
      out.decode_tps = completionTokens / ((lastMs - ttftMs) / 1000);
    } else out.decode_tps = null;
    if (ttftMs !== null && ttftMs > 0 && promptTokens !== null) {
      out.prefill_tps = promptTokens / (ttftMs / 1000);
    } else out.prefill_tps = null;
    return out;
  } catch (e) {
    return { ok: false, error: String(e && e.message ? e.message : e) };
  } finally {
    clearTimeout(timer);
  }
}

// ---------------------------------------------------------------------------
// Campaign
// ---------------------------------------------------------------------------

async function main() {
  const OPTS = parseArgs(process.argv.slice(2));
  const { key: KEY, source: KEY_SOURCE } = resolveKey();
  console.log(`key source: ${KEY_SOURCE}`);
  if (!KEY) {
    console.error("no gateway key (LM_KEY or %APPDATA%/LocalMind/gateway.key); aborting");
    process.exit(2);
  }
  const HTTP_PORT = Number(new URL(OPTS.baseUrl).port || 17860);
  const ABase = agentsBase();

  // Live context comes from the engine (single source of truth) — but a
  // --dry-run must NEVER touch the network (machine may be busy / app down).
  let stPre = null;
  if (!OPTS.dryRun) {
    stPre = parseJson((await apiGet(OPTS.baseUrl, KEY, "/api/status")).text);
  }
  const liveContext = stPre && Number(stPre.context) > 0 ? Number(stPre.context) : OPTS.context;

  const launchCtx = { httpPort: HTTP_PORT, liveContext, key: KEY, agentsBase: ABase, thinking: OPTS.fair ? OPTS.thinking : "max" };

  // --dry-run: print exact commands/env (key redacted), touch nothing.
  if (OPTS.dryRun) {
    console.log(`dry-run: base=${OPTS.baseUrl} profile=${OPTS.profile} context=${liveContext} fair=${OPTS.fair} thinking=${OPTS.thinking} warmup=${OPTS.warmup}`);
    console.log(`dry-run: engine state NOT queried (dry-run never touches the network)`);
    for (const h of OPTS.harnesses) {
      for (const phase of ["reaction", "throughput"]) {
        if (phase === "reaction" && !OPTS.reaction) continue;
        if (phase === "throughput" && !OPTS.throughput) continue;
        const spec = harnessSpec(h, phase === "reaction" ? REACTION_PROMPT : THROUGHPUT_PROMPT, launchCtx);
        const cwd = `<TEMP>\\harness-bench\\${h}-${phase}-<n>`;
        console.log(`cwd: ${cwd}`);
        for (const f of spec.files) console.log(`write-if-changed: ${f.path} (${f.content.length} bytes)`);
        console.log(`env:`);
        for (const [k, v] of Object.entries(spec.env)) {
          const shown = k.toLowerCase().includes("key") ? "[REDACTED]" : v;
          console.log(`  ${k}=${redact(shown, KEY)}`);
        }
        console.log(`argv: ${spec.bin} ${spec.argv.map((a) => JSON.stringify(a)).join(" ")}`);
      }
    }
    console.log(`\ncontrol: POST ${OPTS.baseUrl}/v1/chat/completions (streaming, model qwen3.8-27b, 300-number prompt)`);
    return 0;
  }

  // Engine: reuse or single start (never two engines).
  console.log(`engine: ensuring profile=${OPTS.profile} context=${OPTS.context} ...`);
  const eng = await ensureEngine(OPTS.baseUrl, KEY, OPTS.profile, OPTS.context);
  const order = OPTS.fair ? [...OPTS.harnesses].reverse() : OPTS.harnesses;
  if (OPTS.fair) {
    OPTS.repeats = 1;
    console.log(`fair mode: reversed order (${order.join(",")}), thinking=${OPTS.thinking}, warmup=${OPTS.warmup}, one reaction + one throughput per harness, per-request legs`);
  }
  const liveCtx = {
    ...launchCtx,
    liveContext: Number(eng.status.context) || liveContext,
    thinking: OPTS.fair ? OPTS.thinking : "max",
  };

  // --warmup: one tiny thinking-off request so the pipeline is not cold.
  // This CHANGES what the numbers mean: warm TTFT, not cold-start latency.
  async function warmupOnce(tag) {
    if (!OPTS.warmup) return;
    try {
      await fetch(`${OPTS.baseUrl}/v1/chat/completions`, {
        method: "POST",
        headers: { Authorization: `Bearer ${KEY}`, "Content-Type": "application/json" },
        body: JSON.stringify({
          model: "qwen3.8-27b", stream: false, temperature: 0, max_tokens: 8,
          chat_template_kwargs: { enable_thinking: false },
          messages: [{ role: "user", content: "Responde exactamente: OK" }],
        }),
      });
      console.log(`${tag}: warmup sent (thinking off, 8 tokens)`);
    } catch (e) {
      console.log(`${tag}: warmup failed (ignored): ${String(e).slice(0, 80)}`);
    }
  }

  const runs = [];
  let failures = 0;
  const metricsAfter = async () => parseJson((await apiGet(OPTS.baseUrl, KEY, "/api/metrics")).text);

  for (const h of order) {
    // Config files (write-if-changed, same as the Rust launcher).
    const specProbe = harnessSpec(h, REACTION_PROMPT, liveCtx);
    for (const f of specProbe.files) {
      const changed = writeIfChanged(f.path, f.content);
      console.log(`${h}: config ${f.path} ${changed ? "wrote" : "unchanged"}`);
    }
    const version = await harnessVersion(h, specProbe);
    console.log(`${h}: version: ${version}`);
    await warmupOnce(h);

    if (OPTS.reaction) {
      const spec = harnessSpec(h, REACTION_PROMPT, liveCtx);
      const cwd = mkdtempSync(join(tmpdir(), `harness-bench-${h}-reaction-`));
      console.log(`${h} reaction: spawning...`);
      const before = usageLineCount();
      const r = await runChild(spec, cwd, OPTS.reactionTimeout * 1000);
      const ulines = usageNewLines(before);
      const agg = sumUsage(ulines);
      // Per-request legs: each gateway request decoded separately (fair mode
      // reports these INSTEAD of aggregates; always recorded for honesty).
      const legs = ulines.map((u) => ({
        model: u.model ?? null,
        endpoint: u.endpoint ?? null,
        prompt_tokens: Number(u.prompt_tokens) || 0,
        completion_tokens: Number(u.completion_tokens) || 0,
        ms: Number(u.ms) || 0,
        leg_tps:
          Number(u.ms) > 0 && Number(u.completion_tokens) > 0
            ? Number(u.completion_tokens) / (Number(u.ms) / 1000)
            : null,
        stream: u.stream ?? null,
      }));
      runs.push({
        harness: h,
        version,
        phase: "reaction",
        rep: 1,
        fair: !!OPTS.fair,
        thinking: liveCtx.thinking,
        bin: spec.bin,
        argv: [spec.bin, ...spec.argv],
        env_redacted: Object.fromEntries(
          Object.entries(spec.env).map(([k, v]) => [k, k.toLowerCase().includes("key") ? "[REDACTED]" : redact(v, KEY)])
        ),
        cwd,
        ok: r.ok,
        timedOut: r.timedOut === true,
        exit_code: r.exit_code ?? null,
        wall_ms: r.wall_ms,
        first_byte_ms: r.first_byte_ms ?? null,
        stdout_bytes: r.stdout_bytes ?? null,
        stderr_bytes: r.stderr_bytes ?? null,
        stdout_head: r.stdout_head ? redact(r.stdout_head, KEY) : null,
        stderr_head: r.stderr_head ? redact(r.stderr_head, KEY) : null,
        usage_lines: ulines,
        legs,
        prompt_tokens: agg.prompt_tokens,
        completion_tokens: agg.completion_tokens,
        gateway_ms: agg.ms,
        effective_tps: agg.ms > 0 && agg.completion_tokens > 0 ? agg.completion_tokens / (agg.ms / 1000) : null,
      });
      if (!r.ok) failures++;
      console.log(
        `${h} reaction: ok=${r.ok} wall=${(r.wall_ms / 1000).toFixed(1)}s ` +
          `first=${r.first_byte_ms !== null && r.first_byte_ms !== undefined ? (r.first_byte_ms / 1000).toFixed(1) + "s" : "n/a"} ` +
          `tok=${agg.prompt_tokens}+${agg.completion_tokens} reqs=${ulines.length}${r.error ? " ERR=" + redact(r.error, KEY) : ""}`
      );
    }

    if (OPTS.throughput) {
      for (let rep = 1; rep <= OPTS.repeats; rep++) {
        const spec = harnessSpec(h, THROUGHPUT_PROMPT, liveCtx);
        const cwd = mkdtempSync(join(tmpdir(), `harness-bench-${h}-tput${rep}-`));
        const before = usageLineCount();
        console.log(`${h} throughput rep${rep}: spawning...`);
        const r = await runChild(spec, cwd, OPTS.throughputTimeout * 1000);
        const ulines = usageNewLines(before);
        const agg = sumUsage(ulines);
        const legs = ulines.map((u) => ({
          model: u.model ?? null,
          endpoint: u.endpoint ?? null,
          prompt_tokens: Number(u.prompt_tokens) || 0,
          completion_tokens: Number(u.completion_tokens) || 0,
          ms: Number(u.ms) || 0,
          leg_tps:
            Number(u.ms) > 0 && Number(u.completion_tokens) > 0
              ? Number(u.completion_tokens) / (Number(u.ms) / 1000)
              : null,
          stream: u.stream ?? null,
        }));
        const m = await metricsAfter();
        runs.push({
          harness: h,
          version,
          phase: "throughput",
          rep,
          thinking: liveCtx.thinking,
          bin: spec.bin,
          argv: [spec.bin, ...spec.argv],
          env_redacted: Object.fromEntries(
            Object.entries(spec.env).map(([k, v]) => [k, k.toLowerCase().includes("key") ? "[REDACTED]" : redact(v, KEY)])
          ),
          cwd,
          ok: r.ok,
          timedOut: r.timedOut === true,
          exit_code: r.exit_code ?? null,
          wall_ms: r.wall_ms,
          first_byte_ms: r.first_byte_ms ?? null,
          stdout_bytes: r.stdout_bytes ?? null,
          stderr_bytes: r.stderr_bytes ?? null,
          stdout_head: r.stdout_head ? redact(r.stdout_head, KEY) : null,
          stderr_head: r.stderr_head ? redact(r.stderr_head, KEY) : null,
          usage_lines: ulines,
          legs,
          prompt_tokens: agg.prompt_tokens,
          completion_tokens: agg.completion_tokens,
          gateway_ms: agg.ms,
          effective_tps: agg.ms > 0 && agg.completion_tokens > 0 ? agg.completion_tokens / (agg.ms / 1000) : null,
          engine_gen_tps: m && typeof m.gen_tps === "number" ? m.gen_tps : null,
          engine_metrics: m,
        });
        if (!r.ok) failures++;
        console.log(
          `${h} throughput rep${rep}: ok=${r.ok} wall=${(r.wall_ms / 1000).toFixed(1)}s ` +
            `first=${r.first_byte_ms !== null && r.first_byte_ms !== undefined ? (r.first_byte_ms / 1000).toFixed(1) + "s" : "n/a"} ` +
            `tok=${agg.prompt_tokens}+${agg.completion_tokens} reqs=${ulines.length} ` +
            `eff_tps=${agg.ms > 0 && agg.completion_tokens > 0 ? (agg.completion_tokens / (agg.ms / 1000)).toFixed(1) : "n/a"} ` +
            `gen_tps=${m && typeof m.gen_tps === "number" ? m.gen_tps.toFixed(1) : "n/a"}${r.error ? " ERR=" + redact(r.error, KEY) : ""}`
        );
        if (rep === 1 && (r.wall_ms >= REPEAT_WALL_SECS * 1000 || !r.ok)) {
          console.log(`${h} throughput: single run (wall >= 180 s or failed); skipping repeat`);
          break;
        }
      }
    }
  }

  // Control (engine ceiling, same prompt, direct streaming via gateway).
  // Skipped with --no-control (already measured separately).
  let control = { ok: true, skipped: true };
  if (!OPTS.noControl) {
    console.log("control: direct streaming request through gateway...");
    const cBefore = usageLineCount();
    control = await runControl(OPTS.baseUrl, KEY);
    control.usage_lines = usageNewLines(cBefore);
    const cAgg = sumUsage(control.usage_lines);
    control.gateway_prompt_tokens = cAgg.prompt_tokens;
    control.gateway_completion_tokens = cAgg.completion_tokens;
    control.gateway_ms = cAgg.ms;
    control.engine_metrics = await metricsAfter();
    console.log(
      `control: ok=${control.ok} wall=${control.wall_ms ? (control.wall_ms / 1000).toFixed(1) + "s" : "n/a"} ` +
        `ttft=${control.ttft_ms !== null && control.ttft_ms !== undefined ? (control.ttft_ms / 1000).toFixed(1) + "s" : "n/a"} ` +
        `decode_tps=${control.decode_tps !== null && control.decode_tps !== undefined ? control.decode_tps.toFixed(1) : "n/a"}`
    );
    if (!control.ok) failures++;
  } else {
    console.log("control: skipped (--no-control)");
  }

  // Aggregate table.
  const table = HARNESS_ORDER.filter((h) => OPTS.harnesses.includes(h)).map((h) => {
    const hr = runs.filter((r) => r.harness === h && r.phase === "reaction");
    const ht = runs.filter((r) => r.harness === h && r.phase === "throughput");
    const aggT = { prompt_tokens: 0, completion_tokens: 0, ms: 0 };
    for (const r of ht) {
      aggT.prompt_tokens += r.prompt_tokens;
      aggT.completion_tokens += r.completion_tokens;
      aggT.ms += r.gateway_ms;
    }
    return {
      harness: h,
      version: (hr[0] || ht[0] || {}).version || null,
      reaction_first_byte_ms: hr[0] ? hr[0].first_byte_ms : null,
      reaction_wall_ms: hr[0] ? hr[0].wall_ms : null,
      throughput_reps: ht.length,
      throughput_wall_ms: ht.map((r) => r.wall_ms),
      throughput_first_byte_ms: ht.map((r) => r.first_byte_ms),
      throughput_prompt_tokens: ht.map((r) => r.prompt_tokens),
      throughput_completion_tokens: ht.map((r) => r.completion_tokens),
      throughput_gateway_ms: ht.map((r) => r.gateway_ms),
      throughput_effective_tps: ht.map((r) => r.effective_tps),
      throughput_engine_gen_tps: ht.map((r) => r.engine_gen_tps),
      throughput_requests: ht.map((r) => r.usage_lines.length),
      exceeds_30tps: ht.map((r) => (typeof r.effective_tps === "number" ? r.effective_tps > 30 : null)),
      ok: hr.every((r) => r.ok) && ht.every((r) => r.ok) && ht.length > 0,
    };
  });

  const doc = {
    ts: new Date().toISOString(),
    base_url: OPTS.baseUrl,
    profile: OPTS.profile,
    context_requested: OPTS.context,
    context_live: liveCtx.liveContext,
    engine: { ...eng.status, reused: eng.reused, load_note: eng.note, load_secs: eng.load_secs },
    key_source: KEY_SOURCE,
    runs,
    control,
    table,
  };

  const outDir = join(dirname(resolve(process.argv[1])), "harness-bench", "results");
  mkdirSync(outDir, { recursive: true });
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const outPath = join(outDir, `${stamp}.json`);
  writeFileSync(outPath, JSON.stringify(doc, null, 2), "utf8");
  console.log(`\nwrote ${outPath}`);

  // Readable table.
  console.log("\nharness            | react 1stB | tput wall      | prompt+compl (agg) | eff t/s      | gen_tps      | reqs | >30t/s");
  console.log("-------------------|------------|----------------|--------------------|--------------|--------------|------|-------");
  for (const t of table) {
    const ef = t.throughput_effective_tps.map((v) => (typeof v === "number" ? v.toFixed(1) : "n/a")).join(",");
    const gt = t.throughput_engine_gen_tps.map((v) => (typeof v === "number" ? v.toFixed(1) : "n/a")).join(",");
    const w = t.throughput_wall_ms.map((v) => (v / 1000).toFixed(0) + "s").join(",");
    const over = t.exceeds_30tps.map((v) => (v === null ? "n/a" : v ? "SI" : "NO")).join(",");
    console.log(
      `${t.harness.padEnd(18)} | ${t.reaction_first_byte_ms !== null ? (t.reaction_first_byte_ms / 1000).toFixed(1) + "s" : "n/a".padEnd(7)} | ${w.padEnd(14)} | ${(t.throughput_prompt_tokens.reduce((a, b) => a + b, 0) + "+" + t.throughput_completion_tokens.reduce((a, b) => a + b, 0)).padEnd(18)} | ${ef.padEnd(12)} | ${gt.padEnd(12)} | ${t.throughput_requests.join(",")}    | ${over}`
    );
  }

  if (OPTS.stopAfter) {
    const s = await apiPost(OPTS.baseUrl, KEY, "/api/stop", {});
    console.log(`POST /api/stop -> ${s.status}: ${s.text.slice(0, 120)}`);
  }

  if (failures > 0) {
    console.error(`${failures} run(s) failed`);
    process.exitCode = 1;
  }
  return 0;
}

process.exit(await main());
