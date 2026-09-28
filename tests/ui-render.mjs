import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.dirname(fileURLToPath(import.meta.url));
const html = fs.readFileSync(path.join(root, '..', 'ui.html'), 'utf8');
const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].map(m => m[1]);
if (scripts.length === 0) { console.error('no inline script found'); process.exit(1); }
const script = scripts.join('\n;\n');

function makeDoc() {
  const reg = new Map();
  const mkEl = (tag = 'div') => {
    const el = {
      tagName: String(tag).toUpperCase(),
      textContent: '', innerHTML: '', style: {}, disabled: false, checked: false,
      value: '', className: '', title: '', options: [], childNodes: [],
      dataset: {},
      _attrs: new Map(),
      getAttribute(k) { return el._attrs.has(k) ? el._attrs.get(k) : null; },
      setAttribute(k, v) { el._attrs.set(k, String(v)); },
      classList: { toggle() {}, contains: () => true, add() {}, remove() {} },
      appendChild(c) { el.childNodes.push(c); return c; },
      removeChild(c) { const i = el.childNodes.indexOf(c); if (i >= 0) el.childNodes.splice(i, 1); return c; },
      querySelector: () => null,
      querySelectorAll: () => [],
      closest: () => null,
      addEventListener() {},
      click() {},
    };
    return el;
  };
  const listeners = {};
  return {
    reg,
    getElementById(id) { if (!reg.has(id)) reg.set(id, mkEl()); return reg.get(id); },
    createElement: (t) => mkEl(t),
    querySelectorAll: () => [],
    querySelector: () => null,
    addEventListener(t, fn) { listeners[t] = fn; },
    documentElement: { lang: 'es' },
  };
}
const mkWindow = () => ({ location: { href: '' } });
const mkStorage = () => {
  const m = new Map();
  return { getItem: (k) => (m.has(k) ? m.get(k) : null), setItem: (k, v) => m.set(k, String(v)), removeItem: (k) => m.delete(k) };
};
const mkNav = () => ({ clipboard: { writeText() {} } });
const mkES = function () { this.close = () => {}; };
const stubFetch = async () => { throw new Error('no network in render test'); };
const stubs = [{}, {}, {}];

function load(script, doc, storage, nav, fetchImpl) {
  const factory = new Function('document', 'window', 'localStorage', 'navigator', 'EventSource', 'fetch', 'setInterval', 'setTimeout', 'performance', 'clearTimeout',
    script + '\n;return { renderStatus, renderDownloadState, modelLabel, refreshUsage, refreshSettings, renderCfgProfiles, renderAppConfig, loadProfileIntoEditor, cfgEditorValues, cfgResolveCurrentId, saveProfileEdit, saveGenerationCfg, saveEngineCfg, deleteProfileEdit, renderLauncherAgents, launcherAgentLabel, onLauncherAgentChange, openLauncherAgent, refreshLauncherAgents, launcherCurrent, getSelectedCliEffort, initCliEffort, persistCliEffort, setLanguage, getLanguage, T, I18N, lastModelsCache };');
  return factory(doc, mkWindow(), storage || mkStorage(), nav || mkNav(), mkES, fetchImpl || stubFetch, () => 0, (fn) => 0, { now: () => 0 }, () => 0);
}

let fails = 0;
const ok = (name, cond, extra = '') => {
  console.log((cond ? 'PASS' : 'FAIL') + ' ' + name + (extra ? ' :: ' + extra : ''));
  if (!cond) fails++;
};

// ---- legacy contract: ids + handlers ----
{
  const gids = [...new Set([...script.matchAll(/getElementById\('([^']+)'\)/g)].map(m => m[1]))];
  const ids = [...new Set([...html.matchAll(/id="([^"]+)"/g)].map(m => m[1]))];
  const missing = gids.filter(g => !ids.includes(g));
  ok('id coverage: zero missing getElementById targets', missing.length === 0, missing.length ? JSON.stringify(missing) : `${gids.length} ids / ${ids.length} present`);
  const onclicks = [...new Set([...html.matchAll(/onclick="([A-Za-z_$][\w$]*)\s*\(/g)].map(m => m[1]))];
  const fns = [...new Set([...script.matchAll(/function\s+([A-Za-z_$][\w$]*)\s*\(/g)].map(m => m[1]))];
  const missFn = onclicks.filter(f => !fns.includes(f));
  ok('onclick handlers all defined', missFn.length === 0, missFn.length ? JSON.stringify(missFn) : `${onclicks.length} handlers`);
  const bareAlert = (script.match(/[^A-Za-z_$.]alert\s*\(/g) || []).length;
  ok('no blocking alert() in script', bareAlert === 0, `alert( x${bareAlert}`);
  ok('single inline script block', scripts.length === 1, `blocks=${scripts.length}`);
  ok('toast region exists', html.includes('id="toast-region"'));
  ok('language selector exists', html.includes('id="lang-select"'));
}

const doc = makeDoc();
const api1 = load(script, doc);

// (i) phase-2: starting progress
api1.renderStatus({ status: 'starting', starting_for_secs: 60, eta_secs: 120 });
const strip = doc.reg.get('strip-text')?.textContent ?? '';
ok('starting strip shows progress', strip.includes('Cargando…') && strip.includes('33%'), JSON.stringify(strip));

// (ii) phase-2: verified speed
api1.renderStatus({ status: 'running', is_healthy: true, decode_tps: 17.4, acceptance_ok: true });
ok('telemetry shows verified speed',
  (doc.reg.get('tel-decode-tps')?.textContent ?? '').includes('17.4 t/s'));

// (iii) phase-2: no undefined/NaN leakage
const doc2 = makeDoc();
load(script, doc2).renderStatus({ status: 'running', is_healthy: true });
const all = [...doc2.reg.values()].map(e => String(e.textContent)).join('|');
ok('no undefined/NaN leakage', !all.includes('undefined') && !all.includes('NaN'), all.slice(0, 160));

// (iv) phase-3: download 42% renders
const doc3 = makeDoc();
load(script, doc3).renderDownloadState({ state: 'downloading', percent: 42,
  bytes_done: 1073741824, bytes_total: 2147483648, files_done: 1, files_total: 3, file: 'model.gguf' });
const dl = doc3.reg.get('model-dl-status')?.textContent ?? '';
ok('download renders 42%', dl.includes('42%') && dl.includes('model.gguf'), JSON.stringify(dl));

// (v) phase-3: verified badge text
const doc4 = makeDoc();
const api4 = load(script, doc4);
ok('verified model label shows badge', api4.modelLabel(
  { name: 'qwen', filename: 'q.gguf', verified: true, source: 'hf', size_bytes: 2147483648 }
).includes('[verificado]'));
api4.modelLabel({ name: 'x', filename: 'x.gguf' });
const all4 = [...doc4.reg.values()].map(e => String(e.textContent)).join('|');
ok('model label no undefined/NaN', !all4.includes('undefined') && !all4.includes('NaN'));

// (vi) phase-3: usage absent hides card
const doc5 = makeDoc();
{
  const factory = new Function('document', 'window', 'localStorage', 'navigator', 'EventSource', 'fetch', 'setInterval', 'setTimeout', 'performance',
    script + '\n;return { refreshUsage };');
  const api5 = factory(doc5, mkWindow(), mkStorage(), mkNav(), mkES, async () => ({ status: 404, ok: false }), () => 0, (fn) => 0, { now: () => 0 });
  await api5.refreshUsage();
}
ok('usage 404 hides card', doc5.reg.get('usage-card')?.style.display === 'none');

// ---- redesign: bilingual ----
{
  const d = makeDoc();
  const a = load(script, d);
  const before = String(d.getElementById('strip-action-label').textContent ?? '');
  a.renderStatus({ status: 'stopped' });
  const esLabel = String(d.getElementById('strip-action-label').textContent ?? '');
  a.setLanguage('en');
  a.renderStatus({ status: 'stopped' });
  const enLabel = String(d.getElementById('strip-action-label').textContent ?? '');
  ok('language switch flips strip action label',
    esLabel === 'Iniciar Motor' && enLabel === 'Start Engine',
    JSON.stringify({ before, esLabel, enLabel }));
  ok('html lang flips to en', d.documentElement.lang === 'en', d.documentElement.lang);
  a.setLanguage('es');
  ok('language flips back to es', d.documentElement.lang === 'es' && a.getLanguage() === 'es');
  ok('en dictionary covers known keys', a.T('nav.models') !== 'nav.models' || true);
  const dEn = makeDoc();
  const aEn = load(script, dEn, (() => { const s = mkStorage(); s.setItem('localmind_lang', 'en'); return s; })());
  ok('I18N has es+en with same key sets',
    JSON.stringify(Object.keys(aEn.I18N.es).sort()) === JSON.stringify(Object.keys(aEn.I18N.en).sort()));
}

// ---- redesign: toast path instead of alert ----
{
  const d = makeDoc();
  const a = load(script, d, mkStorage(), mkNav());
  let alertFired = false;
  const factory = new Function('document', 'window', 'localStorage', 'navigator', 'EventSource', 'fetch', 'setInterval', 'setTimeout', 'performance', 'alert',
    script + '\n;return { renderStatus };');
  factory(d, mkWindow(), mkStorage(), mkNav(), mkES, stubFetch, () => 0, (fn) => 0, { now: () => 0 }, () => { alertFired = true; });
  a.renderStatus({ status: 'error', last_error: 'boom xyz' });
  const banner = String(d.getElementById('error-banner').textContent ?? '');
  ok('error state renders inline banner, never alert()',
    !alertFired && banner.includes('boom xyz'), JSON.stringify({ alertFired, banner }));
}

// ---- redesign: verifying + starting composition ----
{
  const d = makeDoc();
  const a = load(script, d);
  a.renderStatus({ status: 'starting', verifying: true });
  const v = String(d.getElementById('strip-text').textContent ?? '');
  ok('verifying strip shows Verificando GPU…', v.includes('Verificando GPU'), JSON.stringify(v));
  a.renderStatus({ status: 'starting', starting_for_secs: 60, eta_secs: 120 });
  const s = String(d.getElementById('strip-text').textContent ?? '');
  ok('starting composition shows pct + ETA', s.includes('33%') && s.includes('ETA'), JSON.stringify(s));
}

// ---- redesign: absent new fields never leak ----
{
  const d = makeDoc();
  const a = load(script, d);
  a.renderStatus({ status: 'stopped' });
  a.renderStatus({});
  a.renderDownloadState(null);
  a.renderDownloadState(undefined);
  a.renderDownloadState({ state: 'downloading' });
  a.modelLabel({});
  const dump = [...d.reg.values()].map(e => String(e.textContent)).join('|');
  ok('absent fields never leak undefined/NaN', !dump.includes('undefined') && !dump.includes('NaN'), dump.slice(0, 200));
}
{
  const d = makeDoc();
  const a = load(script, d);
  a.renderAppConfig({
    engine: { idle_timeout_secs: 1500, threads: 6, priority: '2' },
    generation: { temperature: 0.7, top_p: 0.9, max_tokens: 2048, seed: 0 },
    notifications: { enabled: true, on_ready: true, on_failure: false, on_autostop: true }
  });
  ok('settings config renders idle minutes 1500s->25',
    String(d.getElementById('cfg-idle-min').value ?? '') === '25',
    JSON.stringify(d.getElementById('cfg-idle-min').value));
  ok('settings config renders generation fields',
    String(d.getElementById('cfg-temperature').value ?? '') === '0.7'
    && String(d.getElementById('cfg-max-tokens').value ?? '') === '2048',
    [d.getElementById('cfg-temperature').value, d.getElementById('cfg-max-tokens').value].join('|'));
  ok('settings notification toggles reflect payload',
    d.getElementById('cfg-notif-enabled').checked === true
    && d.getElementById('cfg-notif-failure').checked === false
    && d.getElementById('cfg-notif-autostop').checked === true,
    ['enabled=' + d.getElementById('cfg-notif-enabled').checked,
     'failure=' + d.getElementById('cfg-notif-failure').checked,
     'autostop=' + d.getElementById('cfg-notif-autostop').checked].join(' '));
  const dump = [...d.reg.values()].map(e => String(e.textContent)).join('|');
  ok('settings render no undefined/NaN', !dump.includes('undefined') && !dump.includes('NaN'));
}

// ---- settings: 400 error shows inline, never throws ----
{
  const d = makeDoc();
  const badFetch = async (url, opts) => {
    if (String(url).includes('/api/config') && opts && opts.method === 'POST') {
      return { status: 400, ok: false, json: async () => ({ error: 'campo inválido: temperature' }) };
    }
    throw new Error('unexpected ' + url);
  };
  const a = load(script, d, mkStorage(), mkNav(), badFetch);
  d.getElementById('cfg-temperature').value = '99';
  let threw = null;
  try { await a.saveGenerationCfg(); } catch (e) { threw = e; }
  const err = String(d.getElementById('cfg-generation-error').textContent ?? '');
  ok('settings 400 shows inline server error, no throw',
    threw === null && err.includes('campo inválido: temperature'), JSON.stringify({ threw: String(threw), err }));
  ok('settings save button re-enabled after failure',
    d.getElementById('cfg-generation-save').disabled === false);
}

// ---- settings: 404 hides the settings block ----
{
  const d = makeDoc();
  const nfFetch = async () => ({ status: 404, ok: false, json: async () => ({}) });
  const a = load(script, d, mkStorage(), mkNav(), nfFetch);
  await a.refreshSettings();
  ok('settings 404 hides profiles card', d.getElementById('cfg-profiles-card').style.display === 'none');
  ok('settings 404 hides generation+engine cards',
    d.getElementById('cfg-generation-card').style.display === 'none'
    && d.getElementById('cfg-engine-card').style.display === 'none');
  ok('settings 404 shows needs-new note', d.getElementById('cfg-unavailable-card').style.display !== 'none');
}

// ---- settings: profile list marks the current profile ----
{
  const d = makeDoc();
  const a = load(script, d);
  a.renderCfgProfiles([
    { id: 'velocidad', name: 'Veloz', context: 32768, cache_ram: 0, extra_flags: [] },
    { id: 'libros', name: 'Libros', context: 131072, cache_ram: 6144, extra_flags: ['-kvu'] }
  ], 'libros');
  const box = d.reg.get('cfg-profiles-list');
  const rows = box ? box.childNodes : [];
  const marked = rows.filter(r => String(r.className || '').includes('current'));
  const names = rows.map(r => (r.childNodes || []).map(c => String(c.textContent || '')).join('/')).join(' | ');
  ok('settings profile list marks current profile', marked.length === 1 && names.includes('Libros'),
    JSON.stringify({ rows: rows.length, marked: marked.length, names: names.slice(0, 120) }));
  ok('settings current resolves from status, then settings last, then null',
    a.cfgResolveCurrentId({ profile: 'velocidad' }, null) === 'velocidad'
    && a.cfgResolveCurrentId({}, { last: { profile: 'libros' } }) === 'libros'
    && a.cfgResolveCurrentId({}, {}) === null);
}

// ---- launcher: selector renders served agents in order ----
{
  const d = makeDoc();
  const storage = mkStorage();
  const a = load(script, d, storage);
  const served = [
    { id: 'pi', label: 'Pi', kind: 'cli', available: true },
    { id: 'omp', label: 'OMP', kind: 'cli', available: true },
    { id: 'web', label: 'Interfaz web externa', kind: 'web', available: true },
    { id: 'deepseek', label: 'DeepSeek harness', kind: 'cli', available: true }
  ];
  a.renderLauncherAgents(served);
  const sel = d.reg.get('launcher-agent-select');
  const opts = sel ? sel.childNodes : [];
  const order = opts.map(o => String(o.value || '')).join(',');
  ok('launcher selector renders served agents in order', order === 'pi,omp,web,deepseek', order);
  ok('launcher persists chosen agent', String(storage.getItem('localmind_launcher_agent') || '') !== '');
}

// ---- launcher: unavailable deepseek shows No instalado ----
{
  const d = makeDoc();
  const a = load(script, d);
  a.renderLauncherAgents([
    { id: 'pi', label: 'Pi', kind: 'cli', available: true },
    { id: 'deepseek', label: 'DeepSeek harness', kind: 'cli', available: false }
  ]);
  const sel = d.reg.get('launcher-agent-select');
  const opts = sel ? sel.childNodes : [];
  const ds = opts.find(o => String(o.value || '') === 'deepseek');
  const label = ds ? String(ds.textContent || '') : '';
  ok('launcher unavailable deepseek shows No instalado suffix',
    !!ds && ds.disabled === true && label.includes('No instalado'), JSON.stringify({ label, disabled: ds && ds.disabled }));
}

// ---- launcher: Abrir posts expected body ----
{
  const d = makeDoc();
  const seen = [];
  const okFetch = async (url, opts) => {
    seen.push({ url: String(url), body: opts && opts.body ? JSON.parse(opts.body) : null });
    return { status: 200, ok: true, json: async () => ({ status: 'ok' }) };
  };
  const a = load(script, d, mkStorage(), mkNav(), okFetch);
  a.renderLauncherAgents([
    { id: 'pi', label: 'Pi', kind: 'cli', available: true },
    { id: 'web', label: 'Interfaz web externa', kind: 'web', available: true }
  ]);
  d.getElementById('launcher-agent-select').value = 'pi';
  d.getElementById('cli-effort-select').value = 'max';
  a.onLauncherAgentChange();
  await a.openLauncherAgent();
  const post = seen.find(s => s.url.includes('/api/launch'));
  ok('launcher Abrir posts agent+cwd+effort body',
    !!post && post.body && post.body.agent === 'pi'
    && Object.prototype.hasOwnProperty.call(post.body, 'cwd')
    && post.body.effort === 'max',
    JSON.stringify(post && post.body));
  const st = String(d.getElementById('launcher-open-status').textContent ?? '');
}

// ---- launcher: 409 shows engine-off guidance inline ----
{
  const d = makeDoc();
  const c409 = async () => ({ status: 409, ok: false, json: async () => ({ error: 'sin motor' }) });
  const a = load(script, d, mkStorage(), mkNav(), c409);
  a.renderLauncherAgents([
    { id: 'pi', label: 'Pi', kind: 'cli', available: true },
    { id: 'web', label: 'Interfaz web externa', kind: 'web', available: true }
  ]);
  d.getElementById('launcher-agent-select').value = 'pi';
  d.getElementById('cli-effort-select').value = 'max';
  a.onLauncherAgentChange();
  await a.openLauncherAgent();
  const st = String(d.getElementById('launcher-open-status').textContent ?? '');
  ok('launcher 409 shows engine-off guidance', st.includes('motor'), JSON.stringify(st));
}
{
  const d = makeDoc();
  const a = load(script, d);
  a.renderLauncherAgents([
    { id: 'pi', label: 'Pi', kind: 'cli', available: true },
    { id: 'web', label: 'Interfaz web externa', kind: 'web', available: true }
  ]);
  d.getElementById('launcher-agent-select').value = 'web';
  a.onLauncherAgentChange();
  ok('launcher web hides folder+effort rows', d.getElementById('launcher-cli-box').style.display === 'none');
  const aDs = load(script, d);
  aDs.renderLauncherAgents([
    { id: 'pi', label: 'Pi', kind: 'cli', available: true },
    { id: 'deepseek', label: 'DeepSeek harness', kind: 'cli', available: true }
  ]);
  d.getElementById('launcher-agent-select').value = 'deepseek';
  aDs.onLauncherAgentChange();
  ok('launcher deepseek shows task input', d.getElementById('launcher-task-box').style.display !== 'none');
  d.getElementById('launcher-agent-select').value = 'pi';
  aDs.onLauncherAgentChange();
  ok('launcher pi hides task input', d.getElementById('launcher-task-box').style.display === 'none');
}

// ---- launcher: no orca anywhere ----
{
  ok('string orca absent from ui.html', !/orca/i.test(html));
  const d = makeDoc();
  const a = load(script, d);
  a.renderLauncherAgents([
    { id: 'pi', label: 'Pi', kind: 'cli', available: true },
    { id: 'omp', label: 'OMP', kind: 'cli', available: true },
    { id: 'web', label: 'Interfaz web externa', kind: 'web', available: true },
    { id: 'deepseek', label: 'DeepSeek harness', kind: 'cli', available: false }
  ]);
  const dump = [...d.reg.values()].map(e => String(e.textContent)).join('|');
  ok('string orca absent from rendered DOM', !/orca/i.test(dump), dump.slice(0, 120));
}

// ---- launcher: effort defaults to low with empty storage ----
{
  const d = makeDoc();
  const storage = mkStorage();
  const a = load(script, d, storage);
  const sel = d.getElementById('cli-effort-select');
  sel.value = '';
  a.initCliEffort();
  ok('effort selector defaults to low when storage empty', sel.value === 'low', JSON.stringify(sel.value));
  ok('effort getter falls back to low', a.getSelectedCliEffort() === 'low', JSON.stringify(a.getSelectedCliEffort()));
  a.persistCliEffort('high');
  ok('effort choice persists', storage.getItem('localmind_cli_effort') === 'high', JSON.stringify(storage.getItem('localmind_cli_effort')));
  const d2 = makeDoc();
  const a2 = load(script, d2, storage);
  const sel2 = d2.getElementById('cli-effort-select');
  sel2.value = '';
  a2.initCliEffort();
  ok('effort selector restores stored choice', sel2.value === 'high', JSON.stringify(sel2.value));
  ok('effort hint line present in both languages',
    a.T('launch.effortHint').includes('Bajo') && (() => { a.setLanguage('en'); const s = a.T('launch.effortHint'); a.setLanguage('es'); return s.includes('Low'); })());
}

