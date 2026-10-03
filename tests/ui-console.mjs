#!/usr/bin/env node
// tests/ui-console.mjs — errores de consola/JS de la pagina servida (opt-in, live).
// Uso: node tests/ui-console.mjs [baseUrl] [#tab]
// Abre Chromium headless por CDP, carga la pagina (con tab opcional) y reporta
// console.error + excepciones. Exit != 0 si hay errores.

import { execFile } from 'node:child_process';
import { existsSync } from 'node:fs';

const base = process.argv[2] || 'http://127.0.0.1:17860';
const tab = process.argv[3] || '#tab=settings';

const candidates = [
  process.env.CHROME_PATH,
  'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
].filter(Boolean);
const browser = candidates.find((p) => existsSync(p));
if (!browser) {
  console.log('FAIL no Chromium browser found');
  process.exit(2);
}

const port = 19391 + Math.floor(Math.random() * 1000);
const child = execFile(browser, [
  '--headless=new', '--disable-gpu', '--no-sandbox',
  `--remote-debugging-port=${port}`, 'about:blank',
], { windowsHide: true });

async function waitDebugger(ms = 15000) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    try {
      const r = await fetch(`http://127.0.0.1:${port}/json/version`);
      if (r.ok) return await r.json();
    } catch (e) {}
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('debugger timeout');
}

const errors = [];
let ws;
try {
  await waitDebugger();
  // Chrome moderno exige PUT en /json/new (GET responde 405 en texto).
  const t = await (await fetch(`http://127.0.0.1:${port}/json/new?${encodeURIComponent(base + '/' + tab)}`, { method: 'PUT' })).json();
  ws = new WebSocket(t.webSocketDebuggerUrl, { maxPayload: 64 * 1024 * 1024 });
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  let id = 0;
  const send = (method, params = {}) => new Promise((res) => {
    const cur = ++id;
    const h = (ev) => {
      try {
        const m = JSON.parse(ev.data.toString());
        if (m.id === cur) { ws.removeEventListener('message', h); res(m.result); }
      } catch (e) {}
    };
    ws.addEventListener('message', h);
    ws.send(JSON.stringify({ id: cur, method, params }));
  });
  ws.addEventListener('message', (ev) => {
    try {
      const m = JSON.parse(ev.data.toString());
      if (m.method === 'Runtime.consoleAPICalled' && m.params && m.params.type === 'error') {
        errors.push('console.error: ' + (m.params.args || []).map((a) => a.value || a.description || '').join(' '));
      }
      if (m.method === 'Runtime.exceptionThrown') {
        const d = m.params.exceptionDetails || {};
        errors.push('exception: ' + (d.text || '') + ' ' + ((d.exception && d.exception.description) || ''));
      }
    } catch (e) {}
  });
  await send('Runtime.enable');
  await send('Page.enable');
  await new Promise((r) => setTimeout(r, 6000));
} catch (e) {
  console.log('FAIL harness: ' + e.message);
  process.exitCode = 2;
} finally {
  try { ws && ws.close(); } catch (e) {}
  try { child.kill(); } catch (e) {}
}
if (!process.exitCode) {
  if (errors.length) {
    for (const e of errors) console.log('FAIL ' + e);
    process.exitCode = 1;
  } else {
    console.log('PASS sin errores de consola en ' + tab);
  }
}
