# OMNI packaging

Scripts to ship the app as a portable ZIP and to install/uninstall it on
Windows. Releases go out through GitHub Releases (workflow `release.yml`
on tag `vX.Y.Z`); the in-app updater (Ajustes → Actualización) polls that
channel. No code signing yet (known limit, see below).

Product name is OMNI. The crate/dir names keep `localmind` for compat
(cargo emits `localmind.exe`; user data stays in `%APPDATA%\LocalMind`).
## Scripts

- `packaging/build-portable.ps1` — `cargo build --release`, then assembles
  `dist/OMNI-portable-<version>-<YYYYMMDD>.zip` (`-SkipBuild` reuses the
  existing `src-rust/target/release/localmind.exe`). Prints the ZIP path,
  entry count and size, writes `<zip>.sha256.txt`, and refuses a ZIP without
  `OMNI.exe` + `ui.html` (the updater requires both). Never packs
  `models/**`, `tests/**`, `.git/**`, `target/**` or `*.bak*`.
  `version.txt` carries version + git commit + engine build (traceable).
- `packaging/install.ps1` — copies the runtime files from the repo tree (or
  `-SourceDir`, e.g. an extracted portable dir) into
  `%LOCALAPPDATA%\Programs\OMNI` (overridable via `-TargetDir` for
  testing), creates the Start Menu `OMNI.lnk` + `Uninstall OMNI`
  shortcuts, refuses to overwrite a running `OMNI.exe` or a pending update
  (`%TEMP%\omni-update`), and never deletes `models/` (existing weights
  are kept and reported).
- `packaging/uninstall.ps1` — removes installed files + shortcuts. Refuses
  while `OMNI.exe` runs unless `-Force` (stops it). Preserves `models/`
  unless `-RemoveModels`; preserves `%APPDATA%\LocalMind` (config, keys,
  logs) unless `-RemoveData`.

## ZIP layout

```text
OMNI.exe               # renamed from cargo's localmind.exe
ui.html
ui_fallback.html       # compiled INTO the binary too (include_str!); shipped for reference
omni.ico / omni.png
bin/**                 # llama.cpp runtime (~106 MB)
models/                # EMPTY dir (so base_dir resolves to the portable folder)
version.txt            # app version (Cargo.toml) + llama.cpp build string
LEEME.txt              # Spanish quickstart
```

## Where models live

`models/` sits next to `OMNI.exe` (the "installed" layout). Runtime
resolution (`get_base_dir`, `src-rust/src/main.rs:25-44`):

1. exe dir, if it contains `models/` (portable + installed);
2. else the current dir, if it contains `models/` (dev / `--server-only`);
3. else the exe dir as-is.

`ProcessManager` then uses `<base_dir>/bin/llama-server.exe` and
`<base_dir>/models/`. Config and the gateway key live outside the app dir:
`%APPDATA%\LocalMind\localmind.toml` and `%APPDATA%\LocalMind\gateway.key`
(unchanged by the OMNI rename, so existing installs keep data/keys).

## Auto-update channel

`.github/workflows/release.yml` builds and tests on tag `vX.Y.Z`, checks
`Cargo.toml` matches the tag, runs `build-portable.ps1`, and publishes a
GitHub Release with the ZIP + `.sha256.txt`. The release body MUST contain
a line `SHA256 <asset> <hex>` (the workflow writes it): the app rejects any
package without a checksum match. The app polls `releases/latest` on the
configured feed (`[update].feed`, default `getodevel-source/LocalMind`),
only accepts assets named `OMNI-portable-<semver>-<YYYYMMDD>.zip` newer
than the running version, and installs on restart via a directory swap with
`.prev-<ver>` rollback. `models/` and `%APPDATA%\LocalMind` are never
touched by an update.

## Known limits

- No code signing: Windows SmartScreen will warn on first run.
- Single instance is enforced via `%TEMP%\localmind.lock` (name unchanged:
  an old and a new build still exclude each other).
- The app binds `127.0.0.1:17860` (+10 fallbacks) unless `[lan].enabled`
  (then `0.0.0.0`, peer-gated); the engine port comes from
  config (`llama_port`, default 8080, next-free on clash).
