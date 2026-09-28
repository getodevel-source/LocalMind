#!/usr/bin/env node
// LocalMind pressure sampler (Node ESM, no dependencies).
// Samples every 5 s for a given duration and writes JSONL: free/committed RAM,
// page-file usage, CPU total + llama-server.exe CPU/RAM, GPU engine
// utilization, and gen_tps from /api/metrics when the app answers.
// Starts nothing itself: run ALONGSIDE a harness run.
//
// Usage: node tests/pressure-sampler.mjs --duration 600 [--out <file>]
//   [--base-url http://127.0.0.1:17860]  (default out: tests/harness-bench/results/pressure-<ts>.jsonl)

import { execSync } from "node:child_process";
import { appendFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const ARGS = process.argv.slice(2);
function flag(name, def) {
  const i = ARGS.indexOf(name);
  return i >= 0 && i + 1 < ARGS.length ? ARGS[i + 1] : def;
}
const DURATION_S = Number(flag("--duration", "600"));
const BASE = (flag("--base-url", "http://127.0.0.1:17860") || "").replace(/\/+$/, "");
const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const OUT =
  flag("--out", "") ||
  join(dirname(resolve(process.argv[1])), "harness-bench", "results", `pressure-${stamp}.jsonl`);

function resolveKey() {
  const env = (process.env.LM_KEY || "").trim();
  if (env) return env;
  try {
    const p = join(process.env.APPDATA || "", "LocalMind", "gateway.key");
    if (existsSync(p)) {
      const k = readFileSync(p, "utf8").trim();
      if (k) return k;
    }
  } catch {}
  return null;
}
const KEY = resolveKey();

function psCsv() {
  // One lightweight snapshot: Name,PID,CPU(s),WorkingSet(bytes) for all procs.
  try {
    const out = execSync(
      'powershell.exe -NoProfile -Command "Get-Process | Select-Object Name,Id,CPU,WorkingSet64 | ConvertTo-Csv -NoTypeInformation"',
      { timeout: 20000, windowsHide: true }
    ).toString("utf8");
    return out;
  } catch {
    return "";
  }
}

function memCounters() {
  const grab = (script) => {
    try {
      const out = execSync(`powershell.exe -NoProfile -Command "${script}"`, {
        timeout: 20000,
        windowsHide: true,
      }).toString("utf8");
      return out.trim().split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
    } catch {
      return [];
    }
  };
  const os = grab(
    "$o=Get-CimInstance Win32_OperatingSystem; 'FreeMB='+[math]::Round($o.FreePhysicalMemory/1KB)+' TotalMB='+[math]::Round($o.TotalVisibleMemorySize/1KB)"
  );
  const com = grab("(Get-Counter '\\Memory\\% Committed Bytes In Use').CounterSamples|ForEach-Object{'CommittedPct='+[math]::Round($_.CookedValue,1)}");
  const pg = grab("(Get-Counter '\\Paging File(_Total)\\% Usage').CounterSamples|ForEach-Object{'PagePct='+[math]::Round($_.CookedValue,1)}");
  const cpu = grab("(Get-Counter '\\Processor(_Total)\\% Processor Time' -SampleInterval 1 -MaxSamples 1).CounterSamples|ForEach-Object{'CpuPct='+[math]::Round($_.CookedValue,1)}");
  return [...os, ...com, ...pg, ...cpu];
}

function gpuTop() {
  try {
    const out = execSync(
      `powershell.exe -NoProfile -Command "(Get-Counter '\\GPU Engine(*)\\Utilization Percentage').CounterSamples | Sort-Object CookedValue -Descending | Select-Object -First 3 InstanceName,CookedValue | ForEach-Object{$_.InstanceName + '=' + [math]::Round($_.CookedValue,1)}"`,
      { timeout: 20000, windowsHide: true }
    ).toString("utf8");
    return out.trim().split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
  } catch {
    return [];
  }
}

async function metrics() {
  if (!KEY) return null;
  try {
    const ctl = new AbortController();
    const t = setTimeout(() => ctl.abort(), 8000);
    const r = await fetch(`${BASE}/api/metrics`, {
      headers: { Authorization: `Bearer ${KEY}` },
      signal: ctl.signal,
    });
    clearTimeout(t);
    if (r.status !== 200) return null;
    return await r.json();
  } catch {
    return null;
  }
}

mkdirSync(dirname(resolve(OUT)), { recursive: true });
console.log(`pressure: ${DURATION_S}s @5s -> ${OUT}`);
const end = Date.now() + DURATION_S * 1000;
let n = 0;
while (Date.now() < end) {
  const t0 = Date.now();
  const rec = { ts: new Date().toISOString(), n: n++ };
  rec.mem = memCounters();
  rec.gpu_top3 = gpuTop();
  const m = await metrics();
  rec.gen_tps = m && typeof m.gen_tps === "number" ? m.gen_tps : null;
  rec.cached = m && typeof m.prompt_tokens_cached === "number" ? m.prompt_tokens_cached : null;
  rec.requests_processing = m && typeof m.requests_processing === "number" ? m.requests_processing : null;
  // llama-server.exe CPU/RAM from one CSV snapshot (cheap parse, no per-proc calls).
  try {
    const csv = psCsv().split(/\r?\n/);
    for (const line of csv) {
      const low = line.toLowerCase();
      if (low.includes("llama-server")) rec.llama_server = line.trim();
    }
  } catch {}
  appendFileSync(OUT, JSON.stringify(rec) + "\n");
  const spent = Date.now() - t0;
  const wait = 5000 - spent;
  if (wait > 0 && Date.now() + wait < end + 1000) await new Promise((r) => setTimeout(r, Math.min(wait, Math.max(0, end - Date.now()))));
}
console.log(`pressure: wrote ${n} samples`);
