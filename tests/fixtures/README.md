# Fixtures — captured live

Captured with `node tests/smoke.mjs --write-fixtures` against the live
app (see `tests/README.md`). Each file is the pretty-printed (2 spaces)
body of its endpoint at capture time:

| file | endpoint |
|---|---|
| `status.json` | `GET /api/status` |
| `hardware.json` | `GET /api/hardware` |
| `profiles.json` | `GET /api/profiles` |
| `settings.json` | `GET /api/settings` |
| `models.json` | `GET /api/models` |
| `v1-models.json` | `GET /v1/models` |

To re-capture: `node tests/smoke.mjs --write-fixtures --force`.
Without `--force`, existing files are kept (script prints `SKIP`).
