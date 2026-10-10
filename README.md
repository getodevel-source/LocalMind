# OMNI — Estación de IA local para Windows

Un único ejecutable que administra un motor `llama.cpp` sobre GPU, expone
una API OpenAI-compatible en loopback, ofrece chat/telemetría en ventana
nativa y lanza agentes CLI externos con el modelo local ya configurado.
Sin terminal: todo se opera desde la app.

## Empezar

1. Descarga el ZIP `OMNI-portable-<versión>-<fecha>.zip` de
   [GitHub Releases](../../releases) y extráelo donde quieras
   (p. ej. `C:\OMNI`), o instala con `packaging/install.ps1` en
   `%LOCALAPPDATA%\Programs\OMNI` (no requiere administrador).
2. Copia tus `.gguf` a `models/` junto a `OMNI.exe`
   (o impórtalos desde Ajustes → Modelos).
3. Ejecuta `OMNI.exe`. En SmartScreen pulsa
   «Más información → Ejecutar de todos modos» (sin firma de código).
4. En el Panel pulsa **Arrancar** y chatea en la tab Chat.

## Docs

- [`docs/MANUAL.md`](docs/MANUAL.md) — manual de usuario (instalación,
  primer arranque, actualización, datos y desinstalación; con quick start
  en inglés).
- [`packaging/README.md`](packaging/README.md) — layout del ZIP,
  instalador/desinstalador, canal de auto-update, troubleshooting.
- [`tests/README.md`](tests/README.md) — harnesses y contrato de tests.
- [`CHANGELOG.md`](CHANGELOG.md) — cambios por versión (Keep a Changelog).
- [`docs/SRS.md`](docs/SRS.md) — especificación del sistema.

## Estado

Versión actual: ver [`src-rust/Cargo.toml`](src-rust/Cargo.toml) y
[`CHANGELOG.md`](CHANGELOG.md). `cargo test` 212/0 + `cargo fmt` +
`cargo clippy`, `tests/ui-render.mjs` 97 PASS, `tests/smoke.mjs` 17/17
en vivo. CI en `.github/workflows/ci.yml`; release por tag `vX.Y.Z`.
