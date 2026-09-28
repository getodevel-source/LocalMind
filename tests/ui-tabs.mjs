#!/usr/bin/env node
// tests/ui-tabs.mjs — opt-in live tab-paint harness (closes testing gap D-40).
//
// Why it exists: the stub-DOM suite (tests/ui-render.mjs) cannot catch a panel
// that gets reparented/nested in real HTML — exactly the audit P0 of
// docs/ui/audit-2026-09-28.md, where an unclosed <div> in the Panel launcher
// card swallowed the six sibling tabs (only Panel painted; every other tab was
// a black viewport with panelH=0). This harness drives the REAL served page in
// a headless Chromium browser via CDP and asserts each panel paints.
//
// Requirements: the LocalMind app running (default http://127.0.0.1:17860) and
// a Chromium-based browser (Chrome or Edge) on PATH or at the well-known
// Windows install locations. Dependency-free (Node >= 18, global WebSocket).
//
// Usage:
//   node tests/ui-tabs.mjs [baseUrl] [--browser <path>] [--help]
//   baseUrl defaults to http://127.0.0.1:17860
//
// Exit code: 0 when all 7 tabs paint, 1 on any failure (incl. app/browser
// unavailable — with a clear message, never a stack dump as the verdict).

import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';

const TABS = ['dashboard', 'chat', 'models', 'usage', 'terminal', 'info', 'settings'];

function usage() {
  console.log(`usage: node tests/ui-tabs.mjs [baseUrl] [--browser <path>] [--help]
  baseUrl defaults to http://127.0.0.1:17860
  Drives headless Chrome/Edge via CDP, clicks each nav item, asserts its panel
  exists, is not nested inside another panel, and has offsetHeight > 0.
  Requires the app running and a Chromium-based browser installed.`);
}

function findBrowser(explicit) {
  const cands = [
    explicit,
    process.env.CHROME_PATH,
    'C:/Program Files/Google/Chrome/Application/chrome.exe',
    'C:/Program Files (x86)/Google/Chrome/Application/chrome.exe',
    'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
    'C:/Program Files/Microsoft/Edge/Application/msedge.exe',
  ].filter(Boolean);
  for (const c of cands) {
    if (existsSync(c)) return c;
    if (!c.includes('/') && !c.includes('\\')) return c; // PATH lookup, try it
  }
  return null;
}

export async function runTabs({ baseUrl, browser, timeoutMs = 25000, width = 1180, height = 820, collectShots = null } = {}) {
  const results = [];
  const port = 19300 + Math.floor(Math.random() * 500);
  const srv = spawn(browser,
    ['--headless=new', '--no-sandbox', '--disable-gpu', `--window-size=${width},${height}`,
      `--remote-debugging-port=${port}`, 'about:blank'],
    { stdio: 'ignore', detached: true });
  srv.unref();
  const kill = () => { try { process.kill(srv.pid); } catch {} };
  try {
    await new Promise(r => setTimeout(r, 3000));
    let targets;
    try {
      targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    } catch {
      return { ok: false, fatal: `browser CDP unreachable on 127.0.0.1:${port} (${browser})` };
    }
    const page = targets.find(t => t.type === 'page');
    if (!page) return { ok: false, fatal: 'no CDP page target found' };
    const ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((res, rej) => { ws.onopen = res; ws.onerror = () => rej(new Error('cdp ws')); });
    let id = 0; const pend = new Map();
    ws.onmessage = (e) => {
      let raw; try { raw = JSON.parse(e.data); } catch { return; }
      if (raw.id && pend.has(raw.id)) { pend.get(raw.id)(raw); pend.delete(raw.id); }
    };
    const send = (method, params = {}) => new Promise(res => {
      const i = ++id; pend.set(i, res);
      ws.send(JSON.stringify({ id: i, method, params }));
    });
    const ev = (expr) => send('Runtime.evaluate', { expression: expr, returnByValue: true })
      .then(r => r.result?.result?.value);
    await send('Page.navigate', { url: baseUrl.replace(/\/+$/, '') + '/' });
    await new Promise(r => setTimeout(r, 12000));
    let ok = true;
    for (const t of TABS) {
      let info;
      try {
        info = await ev(`(()=>{const b=document.querySelector('[data-tab="${t}"]');`
          + ` if(!b) return 'NO_BTN'; b.click();`
          + ` const p=document.querySelector('.tab-panel.active'); if(!p) return 'NO_PANEL';`
          + ` const r=p.getBoundingClientRect();`
          + ` const nested=!!p.parentElement.closest('.tab-panel');`
          + ` return p.id+'|panelH='+Math.round(r.height)+'|nested='+nested;})()`);
      } catch (e) {
        results.push({ tab: t, pass: false, detail: 'evaluate failed: ' + (e?.message || e) });
        ok = false;
        continue;
      }
      const m = /^(\S+)\|panelH=(\d+)\|nested=(true|false)$/.exec(String(info || ''));
      const pass = !!m && m[1] === `tab-${t}` && Number(m[2]) > 0 && m[3] === 'false';
      if (!pass) ok = false;
      const parent = pass ? 'main' : '?';
      results.push({ tab: t, pass, detail: `panelH=${m ? m[2] : '?'} parent=${m ? (m[3] === 'false' ? 'main' : 'NESTED') : parent}` });
      if (collectShots) {
        try {
          const shot = await send('Page.captureScreenshot', { format: 'png' });
          if (shot?.result?.data) collectShots(t, Buffer.from(shot.result.data, 'base64'));
        } catch {}
      }
      await new Promise(r => setTimeout(r, 300));
    }
    try { ws.close(); } catch {}
    return { ok, results };
  } finally {
    kill();
  }
}

async function main() {
  const args = process.argv.slice(2);
  if (args.includes('--help') || args.includes('-h')) { usage(); process.exit(0); }
  let baseUrl = 'http://127.0.0.1:17860';
  let browserArg = null;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--browser' && args[i + 1]) { browserArg = args[++i]; }
    else if (!args[i].startsWith('--') && baseUrl === 'http://127.0.0.1:17860') baseUrl = args[i].replace(/\/+$/, '');
  }
  const browser = findBrowser(browserArg);
  if (!browser) {
    console.log('FAIL tabs — no Chromium browser found (Chrome/Edge; pass --browser <path>)');
    process.exit(1);
  }
  try {
    const res = await fetch(baseUrl.replace(/\/+$/, '') + '/', { signal: AbortSignal.timeout(8000) });
    if (!res.ok) throw new Error('HTTP ' + res.status);
  } catch (e) {
    console.log(`FAIL tabs — app unreachable at ${baseUrl} (${e?.cause?.code || e.message}); start the LocalMind GUI first`);
    process.exit(1);
  }
  const { ok, results, fatal } = await runTabs({ baseUrl, browser });
  if (fatal) { console.log(`FAIL tabs — ${fatal}`); process.exit(1); }
  let fails = 0;
  for (const r of results) {
    console.log(`${r.pass ? 'PASS' : 'FAIL'} ${r.tab} — ${r.detail}`);
    if (!r.pass) fails++;
  }
  process.exit(fails ? 1 : 0);
}

const isMain = process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/\\/g, '/').split('/').pop());
if (isMain) await main();
