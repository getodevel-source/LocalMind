# tests/ui-tabs.mjs — live tab-paint harness (opt-in)

## Why it exists

The stub-DOM suite (`tests/ui-render.mjs`) builds fake elements on demand, so it
**cannot** catch a panel that gets reparented or nested by broken real HTML.
That is exactly the audit P0 of `docs/ui/audit-2026-09-28.md`: an unclosed
`<div>` in the Panel launcher card swallowed the six sibling tabs — only Panel
painted while Chat/Modelos/Uso/Logs/Specs/Ajustes were black viewports
(`panelH=0`), yet all 55 stub checks stayed green.

This harness closes gap **D-40**: it drives the **real served page** in a
headless Chromium browser via CDP and asserts each panel paints.

## Requirements

- The LocalMind GUI running (default `http://127.0.0.1:17860`).
- A Chromium-based browser: Chrome or Edge (auto-detected at the usual Windows
  paths, or pass `--browser <path>` / set `CHROME_PATH`).
- Node ≥ 18. No npm dependencies (global `WebSocket` + `fetch` only).

## Run

```text
node tests/ui-tabs.mjs [baseUrl] [--browser <path>]
node tests/ui-tabs.mjs --help
```

Per tab it prints `PASS/FAIL <tab> — panelH=<n> parent=<id>` and exits non-zero
on any failure. `parent=main` means the panel is a direct `<main>` child (not
nested); `panelH=0` means a black viewport. If the app or the browser is
missing you get one clear `FAIL` line (e.g. `app unreachable…`, `no Chromium
browser found`) instead of a stack dump.

## Not a replacement

`ui-render.mjs` stays the fast offline suite (runs everywhere, no app needed).
`ui-tabs.mjs` is **opt-in** and live-only: run it after structural HTML edits,
before calling a UI change done.
