// LocalMind UI contrast check (Node, no dependencies).
// Verifies every text/background pair introduced by the redesign:
//   body text      >= 7:1   (AAA)
//   secondary text >= 4.5:1 (AA)
//   card separators (hairlines between sections) >= 1.5:1
// Usage: node docs/ui/contrast.mjs
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const here = path.dirname(fileURLToPath(import.meta.url));
const hereBase = path.basename(here);
const repoRoot = hereBase === 'tests' ? path.resolve(here, '..') : path.resolve(here, '..', '..');
const css = fs.readFileSync(path.join(repoRoot, 'ui.html'), 'utf8');

function lum(hex) {
  const c = hex.replace('#', '');
  const full = c.length === 3 ? c.split('').map(x => x + x).join('') : c.slice(0, 6);
  const v = [0, 2, 4].map(i => parseInt(full.slice(i, i + 2), 16) / 255)
    .map(u => (u <= 0.03928 ? u / 12.92 : Math.pow((u + 0.055) / 1.055, 2.4)));
  return 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
}
function ratio(a, b) {
  const x = lum(a); const y = lum(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}
function mix(fg, bg, alpha) {
  const p = h => { h = h.replace('#', ''); return [0, 2, 4].map(i => parseInt(h.slice(i, i + 2), 16)); };
  const F = p(fg); const B = p(bg);
  const M = F.map((f, i) => Math.round(f * alpha + B[i] * (1 - alpha)));
  return '#' + M.map(v => v.toString(16).padStart(2, '0')).join('');
}

const BG = '#0a0c0e';   // --bg
const S1 = '#10141a';   // --surface (cards)
const S2 = '#161c24';   // --surface-elevated (inset boxes)

const cases = [
  // [name, fg, bg, min]
  ['body text on app bg', '#eef3f6', BG, 7],
  ['body text on card', '#eef3f6', S1, 7],
  ['body text on inset', '#eef3f6', S2, 7],
  ['muted text on app bg', '#b7c1cb', BG, 4.5],
  ['muted text on card', '#b7c1cb', S1, 4.5],
  ['muted text on inset', '#b7c1cb', S2, 4.5],
  ['subtle text on app bg', '#9aa7b4', BG, 4.5],
  ['subtle text on card', '#9aa7b4', S1, 4.5],
  ['accent white on app bg', '#f5f7f7', BG, 4.5],
  ['accent white on card', '#f5f7f7', S1, 4.5],
  ['ok gray on app bg', '#c3cad2', BG, 4.5],
  ['warn gray on app bg', '#9aa3ab', BG, 4.5],
  ['danger white on app bg', '#ffffff', BG, 4.5],
  ['code gray on app bg', '#b7c1cb', BG, 4.5],
  ['button text on accent', '#0b0d0f', '#f5f7f7', 4.5],
  ['card hairline vs app bg', mix('#eef3f6', BG, 0.18), BG, 1.5],
  ['card hairline vs card', mix('#eef3f6', S1, 0.18), S1, 1.5],
];

let fails = 0;
for (const [name, fg, bg, min] of cases) {
  const r = ratio(fg, bg);
  const pass = r >= min;
  console.log(`${pass ? 'PASS' : 'FAIL'} ${name} :: fg=${fg} bg=${bg} ratio=${r.toFixed(2)} min=${min}`);
  if (!pass) fails++;
}
if (fails) { console.error(`${fails} contrast check(s) below target`); process.exit(1); }
console.log(`all ${cases.length} contrast pairs meet their targets`);
