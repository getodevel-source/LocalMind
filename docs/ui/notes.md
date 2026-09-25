# LocalMind UI — redesign notes (P17 bilingual + P18 full redesign)

Owner drivers: P18=C (complete redesign, not on the current structure) and
P17=C (bilingual es/en). The app shell below is rebuilt from tokens; the
behaviour contract in the `<script>` is frozen.

## What changed (ui.html, 3340 → ~3860 lines)

- `docs/ui/parts/p1_head.html` → `ui.html:1–~640`: design tokens
  (`--bg #0a0c0e`, `--surface #10141a`, `--surface-elevated #161c24`,
  single mint accent `#3ddc97`, text `#eef3f6/#b7c1cb/#9aa7b4`).
  Type scale 12/13/20 px + weights 400/500/650, spacing `--sp-1..6`,
  radii `--r-s/m/l/pill`, shadows `--shadow-card/pop`, `--focus-ring`.
  Contrast targets: body ≥ 7:1, secondary ≥ 4.5:1, hairlines ≥ 1.5:1,
  verified by `tests/ui-contrast.mjs` (17/17 PASS).
- `docs/ui/parts/p2_body.html` → `ui.html:~640–2000`: app shell —
  header (brand + `app-version` badge from `/api/version`, engine chip
  `header-status`, `lang-select`, 6-tab nav Panel/Chat/Modelos/Uso/Logs/Specs),
  persistent status strip (`status-strip`), toast region, content sections.
  Logs live ONLY in the Logs tab (`terminal-stream`); panel shows cards.
- `docs/ui/parts/p3_js.html` → `ui.html:~2000–3860`: `I18N = { es, en }`
  (181 keys) + `T(key)` + `setLanguage/applyI18n`, `localStorage`
  `localmind_lang` (default `es`), `<html lang>` switching, hash deep-link
  (`#tab=chat&lang=en`; server 404s query strings, so state travels in the
  fragment — used by screenshots). New views: `renderModelsTable`,
  `refreshVersion` (badge shows `v…` when the endpoint answers, `local`
  otherwise — never hidden, never invented), nav count badges, `toast()`
  replacing every `alert()`, thinking toggle is a real `<button>`,
  `role="status"` on strip/telemetry/chat-hint/download, `:focus-visible`
  ring, `aria-label`s on icon-only buttons, keyboard-reachable tabs.

## Frozen behaviour (verified, not re-decided)

- `getElementById` 63 targets → 0 missing; 20 `onclick` handlers → all defined
  (checked in `tests/ui-render.mjs`).
- State strings `stopped/starting/running/error`; polls 800 ms status,
  2 s metrics, 1500 ms download, 30 s lazy usage (dashboard OR usage tab).
- Tab persistence (hash + active classes), chat SSE streaming with
  `AbortController`, download-progress polling, `localmind_chat_v1` and
  `localmind_project_cwd` keys unchanged; `lm_key` cookie untouched
  (server-set `HttpOnly` on `GET /`).
- `modelLabel` badge strings come from `I18N` (` [verificado]` preserved).

## What was NOT changed, and why

- `src-rust/**`, `tests/smoke.mjs`, `tests/e2e.mjs`, `tests/bench.mjs`,
  `docs/SRS.md`, `docs/srs/**`: out of scope (file ownership).
- No crate rebuild, no app restart: `GET /` reads `ui.html` from disk.
- No terminal-looking main surface: logs stay inside Logs (LM-UI-1).
- No new backend endpoints: `/api/version` degrades to `local`.
- `temperature: 0.7` stays fixed (LM-UI-10 is P16, a separate decision).
- Chat stays text-only (P19 needs mmproj; P7 keeps it off — conflict open).
- Usage lazy-poll widened to usage tab too (was dashboard-only); cadence 30 s
  unchanged. Everything else byte-equivalent in behaviour.
