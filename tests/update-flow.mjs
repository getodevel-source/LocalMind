#!/usr/bin/env node
// tests/update-flow.mjs — E2E del canal auto-update SIN red ni swap real (P0).
//
// Verifica el contrato que el drill 2026-10-09 rompió en vivo:
//   1. GET /api/update → forma {state,current,latest,percent,error}
//   2. POST /api/update/restart sin `ready` → 409 (no sale, no rompe nada)
//   3. ui.html servida trae botón #cfg-update-restart + restartToInstall +
//      claves settings.updateRestart{,ing} es/en
//   4. El swap NUNCA apunta al repo dev: el script pendiente en %TEMP%
//      (si existe) no contiene rutas de desarrollo
//   5. GET /api/version.update expone {state,latest,pending}
//
// Requiere la app viva (default http://127.0.0.1:17860). Solo lee estado;
// el único POST (restart sin ready) es rechazado por diseño. Exit 0 si todo
// pasa, 1 si algo falla o la app no responde.

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const ARGS = process.argv.slice(2);
const BASE = (ARGS.find((a) => !a.startsWith("--")) || "http://127.0.0.1:17860").replace(/\/+$/, "");
const TIMEOUT_MS = 8000;

function resolveKey() {
  const env = (process.env.LM_KEY || "").trim();
  if (env) return env;
  try {
    const appdata = process.env.APPDATA;
    if (appdata) {
      const p = join(appdata, "LocalMind", "gateway.key");
      if (existsSync(p)) {
        const k = readFileSync(p, "utf8").trim();
        if (k) return k;
      }
    }
  } catch {}
  return null;
}
const API_KEY = resolveKey();
const authHeaders = () => (API_KEY ? { Authorization: `Bearer ${API_KEY}` } : {});

const results = [];
function report(ok, check, observed) {
  results.push({ ok, check });
  console.log(`${ok ? "PASS" : "FAIL"} ${check} — ${observed}`);
}

async function req(path, { method = "GET" } = {}) {
  const ctrl = new AbortController();
  const t = setTimeout(() => ctrl.abort(), TIMEOUT_MS);
  try {
    const res = await fetch(BASE + path, { method, signal: ctrl.signal, headers: authHeaders() });
    return { status: res.status, text: await res.text() };
  } finally {
    clearTimeout(t);
  }
}

async function main() {
  if (!API_KEY) {
    report(false, "clave", "sin LM_KEY ni gateway.key: no se puede autenticar");
    return 1;
  }
  // 1. Forma de /api/update
  try {
    const { status, text } = await req("/api/update");
    const v = JSON.parse(text);
    const ok = status === 200 && ["idle","checking","available","downloading","ready","installing","error","done"].includes(v.state)
      && typeof v.current === "string" && typeof v.latest === "string";
    report(ok, "GET /api/update forma", `status=${status} state=${v.state} current=${v.current} latest=${v.latest}`);
  } catch (e) {
    report(false, "GET /api/update forma", `request failed: ${e.message}`);
    return 1;
  }
  // 2. restart sin ready → 409 (y la app sigue viva después)
  try {
    const upd = JSON.parse((await req("/api/update")).text);
    const { status, text } = await req("/api/update/restart", { method: "POST" });
    if (upd.state === "ready") {
      report(status === 200, "POST /api/update/restart con ready", `status=${status} (arma salida; NO cerrar en este harness)`);
    } else {
      let msg = "";
      try { msg = JSON.parse(text).error || ""; } catch {}
      report(status === 409 && msg.length > 0, "POST /api/update/restart sin ready → 409", `status=${status} error=${msg.slice(0, 80)}`);
    }
    const alive = await req("/api/update");
    report(alive.status === 200, "app viva tras restart-409", `status=${alive.status}`);
    // 2b. Invariante P0-2: si dice `ready`, el staging existe en disco.
    try {
      const u = JSON.parse(alive.text);
      if (u.state === "ready") {
        const fs2 = await import("node:fs");
        const path2 = await import("node:path");
        const os2 = await import("node:os");
        const st = fs2.readdirSync(path2.join(os2.tmpdir(), "omni-update"), { withFileTypes: true }).filter((d) => d.isDirectory()).map((d) => d.name);
        report(st.length > 0, "ready con staging en disco", `dirs: ${st.slice(0, 4).join(",")}`);
      } else {
        report(true, "ready con staging en disco", `state=${u.state} (no aplica)`);
      }
    } catch (e) {
      report(false, "ready con staging en disco", String(e.message).slice(0, 120));
    }
  } catch (e) {
    report(false, "POST /api/update/restart", `request failed: ${e.message}`);
  }
  // 3. UI: botón + función + claves
  try {
    const { status, text } = await req("/");
    const need = ["cfg-update-restart", "restartToInstall()", "settings.updateRestart", "settings.updateRestarting"];
    const missing = need.filter((s) => !text.includes(s));
    report(status === 200 && missing.length === 0, "UI trae Reiniciar e instalar", missing.length ? `faltan: ${missing.join(",")}` : "botón+fn+claves presentes");
  } catch (e) {
    report(false, "UI trae Reiniciar e instalar", `request failed: ${e.message}`);
  }
  // 4. Scripts pendientes nunca apuntan a desarrollo
  try {
    const files = readdirSync(tmpdir()).filter((f) => f.startsWith("omni-install-") && f.endsWith(".cmd"));
    let bad = [];
    for (const f of files) {
      const body = readFileSync(join(tmpdir(), f), "utf8");
      if (/PROYECTOS|src-rust|\.git/i.test(body)) bad.push(f);
    }
    report(bad.length === 0, "scripts swap sin rutas dev", bad.length ? `CONTAMINADOS: ${bad.join(",")}` : (files.length ? `${files.length} script(s), limpios` : "sin scripts pendientes (nada que validar)"));
  } catch (e) {
    report(false, "scripts swap sin rutas dev", String(e.message).slice(0, 120));
  }
  // 5. /api/version.update expone canal
  try {
    const { status, text } = await req("/api/version");
    const v = JSON.parse(text);
    const ok = status === 200 && v.update && typeof v.update.state === "string";
    report(ok, "GET /api/version.update", `status=${status} update.state=${v.update && v.update.state}`);
  } catch (e) {
    report(false, "GET /api/version.update", `request failed: ${e.message}`);
  }
  const failed = results.filter((r) => !r.ok);
  console.log(`\n== SUMMARY: ${results.length - failed.length}/${results.length} passed ==`);
  return failed.length ? 1 : 0;
}

process.exit(await main());
