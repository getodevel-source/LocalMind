# OpenCode harness → LocalMind (modelo local)

## 1. Qué es el harness

- **Proyecto:** OpenCode (`opencode-ai`) — agent CLI/TUI de código abierto (terminal coding agent con sesiones,
  herramientas, subagentes y MCP).
- **URL:** https://github.com/sst/opencode
- **Licencia:** MIT.
- **Versión instalada:** `1.18.10` (npm `opencode-ai`; verificado: `opencode --version` → `1.18.10`,
  `npm ls -g --depth=0` → `opencode-ai@1.18.10`).
- **Qué es:** TUI interactiva (`opencode [project]`, comando por defecto) + modo no interactivo one-shot
  (`opencode run [message..]`, imprime el resultado y sale) + servidor headless / ACP / web. Soporta
  cualquier proveedor OpenAI-compatible mediante plugin `@ai-sdk/openai-compatible`, así que se puede
  apuntar al gateway LocalMind sin clave cloud.
- **Interactivo vs one-shot:** LocalMind usa `opencode --model localmind/<id>` (TUI) y
  `opencode --model localmind/<id> run "<task>"` (one-shot). La etiqueta en la app sigue siendo `OpenCode`.
- **No hay binario `opencode2`:** verificado con `where opencode2` → sin resultados. La API acepta `opencode2`
  como alias, pero en esta máquina solo existe `opencode` (+ shims `opencode.cmd`/`opencode.ps1`).

## 2. Instalación (ejecutado)

```bat
npm install -g opencode-ai
```

- `opencode --version` → `1.18.10` (observado en esta máquina).
- `npm ls -g --depth=0` → `opencode-ai@1.18.10` (observado).
- Binario: `%APPDATA%\npm\opencode` + shim `%APPDATA%\npm\opencode.cmd` (+ `opencode.ps1`).
- `opencode --help | head` muestra: `opencode [project]` (start TUI, default), `opencode run [message..]`
  (run with a message), `acp`, `mcp`, `serve`, `web`, `attach`, `debug`, `models`, `stats`, etc.

## 3. Configuración LocalMind

Mecanismo: **solo inyección por entorno, sin ficheros de config** — `OPENCODE_CONFIG_CONTENT` con este JSON
exacto (el launcher sustituye origen del gateway, puerto vivo y contexto):

```json
{"$schema":"https://opencode.ai/config.json","model":"localmind/<id>","small_model":"localmind/<id>","provider":{"localmind":{"npm":"@ai-sdk/openai-compatible","name":"LocalMind","options":{"baseURL":"http://127.0.0.1:<port>/v1","apiKey":"{env:LOCALMIND_API_KEY}"},"models":{"<id>":{"name":"<id>","limit":{"context":<ctx>,"output":8192},"modalities":{"input":["text"],"output":["text"]}}}}}}
```

Env que pone el lanzador (la clave se lee de `%APPDATA%\LocalMind\gateway.key`, nunca va en ficheros):

| Var | Valor |
|---|---|
| `OPENCODE_CONFIG_CONTENT` | JSON de arriba (origen del gateway + contexto vivo) |
| `LOCALMIND_API_KEY` | contenido de `%APPDATA%\LocalMind\gateway.key` (el launcher hace `set /p LOCALMIND_API_KEY<"%APPDATA%\LocalMind\gateway.key"`); `{env:LOCALMIND_API_KEY}` se resuelve en runtime, así que la var DEBE estar exportada |
| `XDG_DATA_HOME` / `XDG_CONFIG_HOME` / `XDG_CACHE_HOME` / `XDG_STATE_HOME` | dirs privados bajo `%APPDATA%\LocalMind\agents\opencode\{data,config,cache,state}` |

Diferencias verificadas frente a la receta de referencia inicial:

- `small_model` es OBLIGATORIO (sin él la config no basta).
- El id de modelo debe ser `localmind/<id>` de forma consistente en todas partes.
- La referencia no cubría el modo one-shot — LocalMind usa `opencode --model localmind/<id> run "<task>"`.
- Verificado con `opencode debug config` contra endpoint MUERTO (`http://127.0.0.1:1/v1`): la config se acepta
  y resuelve como se espera. `opencode debug paths` prueba que XDG_DATA/CONFIG/CACHE/STATE_HOME se respetan,
  excepto `tmp`, que queda en `%TEMP%\opencode` (solo caché).

## 4. Comando de lanzamiento (lo que ejecuta el launcher Rust, `cmd.exe`)

Interactivo (TUI):

```bat
cd /d "<CARPETA_PROYECTO>" && set "OPENCODE_CONFIG_CONTENT=<JSON §3>" && set /p LOCALMIND_API_KEY<"%APPDATA%\LocalMind\gateway.key" && set "XDG_DATA_HOME=%APPDATA%\LocalMind\agents\opencode\data" && set "XDG_CONFIG_HOME=%APPDATA%\LocalMind\agents\opencode\config" && set "XDG_CACHE_HOME=%APPDATA%\LocalMind\agents\opencode\cache" && set "XDG_STATE_HOME=%APPDATA%\LocalMind\agents\opencode\state" && opencode --model localmind/qwen3.8-27b
```

One-shot:

```bat
... && opencode --model localmind/qwen3.8-27b run "<TAREA>"
```

- Flags relevantes (`opencode run --help`, observado): `-m/--model provider/model`, `--format default|json`,
  `--agent`, `--variant`, `--dir`, `--attach`, `--auto`. TUI por defecto: `opencode [project]`.
- No existe flag `--thinking` (verificado: `opencode --help | grep thinking|variant` sin coincidencias en el
  nivel global); el esfuerzo sería `--variant`, que LocalMind NO cablea.
- Spawn por la ruta endurecida (verify + fallback `wt.exe`→`cmd start`, pid registrado, 500 si ambos fallan).

## 5. Ejecución verificada (a través del GATEWAY, instancia scratch en puerto no945389 por defecto, motor vivo a 32K)

**Run one-shot:** `opencode --model localmind/qwen3.8-27b run "Responde exactamente: HOLA-LOCAL"` →
stdout `HOLA-LOCAL`, exit `0`, wall ≈10,5 s.

Líneas ganadas en `%APPDATA%\LocalMind\usage.jsonl` (endpoint/modelo/tokens/ms — contabilizado en gateway):

```json
{"model":"qwen3.8-27b","prompt_tokens":597,"completion_tokens":242,"ms":7324,...}
{"model":"qwen3.8-27b","prompt_tokens":12281,"completion_tokens":38,"ms":8349,...}
```

(2 requests: 12.878 prompt / 280 completion en total; segunda pata ≈32,9 t/s completion.)

Contraste medido el mismo día: un run contra el puerto CRUDO del motor no produjo NINGUNA línea de uso —
solo cuenta cuando apunta al gateway (comportamiento actual de la app para todos los CLI agents).

**Rendimiento (mismo motor, 32K):**

| Harness | wall | prompt | completion | t/s completion | overhead |
|---|---|---|---|---|---|
| DeepSeek run A | 55 s | 9.709 | 53 | ~1,0 | system prompt grande |
| OpenCode one-shot | ≈10,5 s | 12.878 (2 req) | 280 (2 req) | ~32,9 (pata completion) | menor |

## 6. Límites honestos

- **Sin gateway no hay cuentas:** si un cliente evita el gateway (puerto crudo), no hay conteo de tokens
  disponible — verificado por ausencia (cero líneas).
- **`--variant` (effort) no cableado** por LocalMind; no hay flag `--thinking`.
- Las funciones cloud propias del harness son irrelevantes/desactivadas aquí (todo pasa por el proveedor
  `localmind` OpenAI-compatible; no se necesita ni se usa clave cloud).
- Comportamiento versionado: lo anterior vale para `1.18.10`; fijar/registrar la versión ante cambios.
- **No medido aquí** (del worker lanzador, se cita sin re-verificar): detalles del fallback `wt.exe`→`cmd start`,
  contenido exacto del pid-log y salida `debug config`/`debug paths` más allá de lo citado en §3.
