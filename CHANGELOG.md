# Changelog OMNI

Todos los cambios publicables de la app. El formato sigue
[Keep a Changelog](https://keepachangelog.com/es-ES/1.0.0/) y el versionado
[SemVer](https://semver.org/lang/es/).

## [Unreleased]

### Añadido
- Actualización automática dentro de la app: chequeo silencioso contra
  GitHub Releases al arrancar (cada 6 h), tarjeta «Actualización» en
  Ajustes (buscar / descargar / cancelar + aviso al reiniciar) y banner en
  el Panel. Descarga con resume, verificación obligatoria por tamaño +
  `SHA256 <asset> <hex>` del body de la release (sin checksum no se
  instala), staging validado y swap de directorios al salir con respaldo
  `.prev-<ver>`. `models/` y `%APPDATA%\LocalMind` jamás se tocan.
  Endpoints: `GET /api/update`, `POST /api/update/check`,
  `POST /api/update/download`, `POST /api/update/cancel`
  (`GET /api/version` expone además `update:{state,latest,pending}`).
  Config `[update]`: `feed` (vacío = repo por defecto), `check_on_startup`
  (default true), `auto_download` (default false).
- Pipeline de release: workflow `release.yml` (tag `vX.Y.Z` → build →
  `SHA256SUMS.txt` → GitHub Release con el ZIP portable + body con línea
  `SHA256 <asset> <hex>`), `CHANGELOG.md` y `version.txt` trazable
  (versión + commit + fecha UTC).

## [2.0.0] - 2026-10-03

### Añadido
- ZIP portable (`packaging/build-portable.ps1`), instalador
  (`packaging/install.ps1`) y desinstalador (`packaging/uninstall.ps1`)
  con preservación de `models/`.
- Gateway local + motor `llama-server`, 4 tabs (Panel, Chat, Conexión,
  Ajustes), modo Oráculo/Guest, pairing LAN por QR, descarga verificada
  de modelos desde Hugging Face.
