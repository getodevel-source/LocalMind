# DeepSeek harness → LocalMind (modelo local)

## 1. Qué es el harness

- **Proyecto:** DeepSeek Harness (`dsh`) — harness/cli de agentes oficial de DeepSeek AI.
- **URL:** https://github.com/deepseek-ai/deepseek-harness
- **Licencia:** MIT.
- **Versión instalada:** `0.1.5-rc.3` (npm `@deepseek-ai/dsh`).
- **Qué es:** runtime de agentes "todo es un plugin" (Cordis) con perfiles `headless` (una tarea, imprime y sale),
  `web`, `acp`, `sdk`. El perfil `headless` es un agent CLI no interactivo: crea un agente, ejecuta la tarea con
  herramientas (ficheros, pwsh, subagentes) y sale 0/1.
- **Por qué este:** (a) es el oficial de `deepseek-ai`; (b) SÍ se puede apuntar a un endpoint OpenAI-compatible
  arbitrario: el adaptador `dsh-llm-pi-ai` acepta rutas manuales con `baseURL` + `api: openai-completions`
  (verificado en `docs/user/guide/providers.md` y en el código instalado `dsh-llm-pi-ai`), y lo verifiqué contra
  `http://127.0.0.1:17860/v1`.
- **Rechazadas:**
  - `Doriandarko/deepseek-engineer` (MIT): lleva `base_url="https://api.deepseek.com"` cableado en
    `deepseek-eng.py`, sin mecanismo de configuración ni modo no interactivo.
  - `aider` (comunidad, maduro, soporta `OPENAI_API_BASE`): no es proyecto DeepSeek; queda como plan B.

## 2. Instalación (ejecutado)

```bat
npm install -g @deepseek-ai/dsh
```

- Salida: `added 520 packages in 2m` (+1 aviso `deprecated node-domexception`, irrelevante).
- `dsh --version` → `0.1.5-rc.3`.
- Binario: `%APPDATA%\npm\dsh.cmd` (real: `C:\Users\juans\AppData\Roaming\npm\dsh.cmd`).

## 3. Configuración LocalMind

Aislamiento: `DSH_HOME = %APPDATA%\LocalMind\agents\deepseek\` (no toca `~/.dsh` ni otras configs).
Perfil usado: `%DSH_HOME%\profiles\headless\cordis.patch.yml` (verificado con `dsh --profile headless --dump-config`):

```yaml
- id: agent-default-model
  config:
    provider: localmind
    model: localmind
- id: llm-pi-ai
  config:
    providers:
      localmind:
        displayName: LocalMind
        apiKeyEnv: LOCALMIND_API_KEY
        api: openai-completions
        baseURL: http://127.0.0.1:17860/v1
        compat:
          supportsDeveloperRole: false
          maxTokensField: max_tokens
          supportsReasoningEffort: false
        models:
          - id: localmind
            name: LocalMind local model
            contextWindow: 32768
            maxTokens: 4096
```

Env que pone el lanzador (la clave se lee de `%APPDATA%\LocalMind\gateway.key`, nunca va en ficheros):

| Var | Valor |
|---|---|
| `LOCALMIND_API_KEY` | contenido de `%APPDATA%\LocalMind\gateway.key` (Bearer del gateway) |
| `DSH_HOME` | `%APPDATA%\LocalMind\agents\deepseek` |
| `DSH_TELEMETRY_MODE` | `DISABLED` (apaga el OTel a `harness-telemetry.deepseeksvc.com`) |
| `DSH_PERMISSION_MODE` | `workspace-write` (escritura limitada al cwd de sesión; verificado: crea/compila dentro del cwd sin preguntas) |

Notas: `compat` necesario porque el proxy LocalMind solo habla chat/completions clásico (sin rol `developer`,
cap con `max_tokens`). Modelo `localmind` (alias aceptado; el proxy lo reescribe al servido). Contexto ≤ 32K.

## 4. Comando de lanzamiento (para el launcher Rust, `cmd.exe`, copiar-pegar)

```bat
cd /d "<CARPETA_PROYECTO>" && set "DSH_HOME=%APPDATA%\LocalMind\agents\deepseek" && set "DSH_TELEMETRY_MODE=DISABLED" && set "DSH_PERMISSION_MODE=workspace-write" && set /p LOCALMIND_API_KEY<"%APPDATA%\LocalMind\gateway.key" && dsh --profile headless "<TAREA>"
```

- El launcher debe leer `%APPDATA%\LocalMind\gateway.key` él mismo y exportarlo como `LOCALMIND_API_KEY` si
  prefiere no usar el `set /p` (equivalente). `dsh` está en `%APPDATA%\npm` (PATH habitual de npm global).
- Verificado funcional mediante el `.cmd` equivalente (mismo contenido, invocado con `call`); la línea literal
  de un solo `&&` no se pudo probar tal cual porque mi shell intermedia rompía el entrecomillado, no por `cmd`.
- El cwd donde se invoca es el workspace del agente (lectura/escritura confinada ahí).

## 5. Ejecución verificada (motor local, perfil `velocidad`, ctx 32768, `acceptance_ok=true`)

**Run A — humo sin herramientas:** `dsh --profile headless "Responde con exactamente esta palabra y nada mas: HOLA-LOCAL. No uses ninguna herramienta, responde directamente."` → stdout `HOLA-LOCAL`, exit `0`, reasoning a stderr.

**Run B — agente real con herramientas** (cwd scratch, 4 turnos): crear `reverse.rs` con
`fn reverse(s: &str) -> String`, compilar con `rustc --crate-type lib` → stdout:
`Creé reverse.rs con fn reverse(s: &str) -> String y compiló correctamente con rustc --crate-type lib (exit 0, solo un warning de código no usado).`, exit `0`. El fichero existía y `libreverse.rlib` también.

Líneas coincidentes de `%APPDATA%\LocalMind\usage.jsonl` (endpoint/modelo/tokens/ms — servido local):

```json
{"completion_tokens":53,"endpoint":"chat.completions","model":"localmind","ms":51067,"prompt_tokens":9709,"stream":true,"ts":1790321011}
{"completion_tokens":64,"endpoint":"chat.completions","model":"localmind","ms":3003,"prompt_tokens":232,"stream":true,"ts":1790321040}
{"completion_tokens":160,"endpoint":"chat.completions","model":"localmind","ms":54843,"prompt_tokens":9749,"stream":true,"ts":1790321092}
{"completion_tokens":130,"endpoint":"chat.completions","model":"localmind","ms":4324,"prompt_tokens":9965,"stream":true,"ts":1790321097}
{"completion_tokens":80,"endpoint":"chat.completions","model":"localmind","ms":3364,"prompt_tokens":10193,"stream":true,"ts":1790321100}
```

**Rendimiento (Qwen3.8-27B-IQ4_XS, perfil velocidad):**

| Run | wall | prompt | completion | ms motor | t/s completion | streaming |
|---|---|---|---|---|---|---|
| A (1 turno) | 55 s | 9709 | 53 | 51067 | ~1,0 | sí (SSE + `stream_options.include_usage`) |
| B (4 turnos) | 65 s | 30139 total | 434 total | 65534 total | ~6,6 | sí |

- Cada request lleva ~9,6k prompt tokens de system prompt + definiciones de herramientas (overhead fijo del harness).
- `GET /api/metrics` tras B: `prompt_tokens_total 9972, tokens_predicted 333, spec draft 249/accepted 203`.
- Motor parado al final (`POST /api/stop` → `stopped`); `llama-server.exe` no en `tasklist`.

## 6. Límites honestos

- **No verificado:** la línea `cmd.exe` literal de §4 tal cual (verificada vía `.cmd` equivalente con `call`); perfil `web`
  (solo se probó `headless`); `maxTokens` > 4096 por request; modelos `qwen3.8-27b` como id (el proxy reescribe igual).
- **Sin clave cloud:** `web_search`/`web_fetch` del harness piden `DEEPSEEK_API_KEY` (no configurado → esas herramientas
  fallan, el resto funciona). `session-log-deepseek` viene desactivado por defecto en esta versión (opt-in).
- **Gateway:** basta `/v1/chat/completions` + `/v1/models` (ambos existen). `/v1/responses` existe pero no se necesita
  (usamos protocolo `openai-completions`). Tool-call streaming: el proxy lo reenvía y el run B lo ejercitó (4 turnos con
  `write`+`exec`), sin problema.
- **Estado dejado:** `LocalMind.exe` en ejecución (lo arranqué yo para usar la API; no mato procesos por norma),
  motor `stopped`, sin `llama-server.exe`, scratch (`work/*.cmd`, `/tmp/dsh-*`) eliminado.
