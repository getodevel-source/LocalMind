//! Registro de agentes lanzables.
//! - `pi`/`omp`: CLIs con dir privado `%APPDATA%\LocalMind\agents\<agent>\`
//!   (puerto/contexto/apiKey vivos), `PI_CODING_AGENT_DIR` y spawn por
//!   Windows Terminal → `cmd start`.
//! - `opencode`: harness `opencode-ai` (npm global) con `OPENCODE_CONFIG_CONTENT`
//!   inyectado por env (puerto/contexto vivos, clave por `{env:...}`) y dirs
//!   XDG aislados; sin ficheros de config del usuario tocados.
//! - `web`: abre la interfaz web externa (misma acción que el antiguo
//!   `/api/open_browser`: el navegador en el puerto del motor).
//! - `deepseek`: harness oficial `@deepseek-ai/dsh` (npm global): el binario
//!   real es el shim `%APPDATA%\npm\dsh.cmd`; env aislado (`DSH_HOME` propio,
//!   telemetría off, `workspace-write`) y clave por `set /p`.
//!
//! El spawn de terminal SIEMPRE verifica el resultado: intenta WT y, si falla,
//! cae a `cmd start` (`spawn_terminal`); si ambos fallan el endpoint responde
//! 500 en español en vez de un falso 200. Todo lo testeable vive en funciones
//! puras (sin diálogos ni red ni spawn).

use std::path::PathBuf;

/// Ids estables del selector único de la UI, en orden fijo.
pub const AGENT_ORDER: [&str; 5] = ["pi", "omp", "opencode", "web", "deepseek"];

/// Agente pedido en `POST /api/launch {"agent": …}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentId {
    Pi,
    Omp,
    OpenCode,
    Web,
    DeepSeek,
}

/// `None` = agente desconocido (el endpoint responde 400).
pub fn parse_agent_id(agent: &str) -> Option<AgentId> {
    match agent.trim().to_lowercase().as_str() {
        "pi" => Some(AgentId::Pi),
        "omp" => Some(AgentId::Omp),
        "opencode" | "opencode2" => Some(AgentId::OpenCode),
        "web" => Some(AgentId::Web),
        "deepseek" => Some(AgentId::DeepSeek),
        _ => None,
    }
}

/// Etiqueta española para la UI y los logs.
pub fn agent_label(id: AgentId) -> &'static str {
    match id {
        AgentId::Pi => "Pi",
        AgentId::Omp => "OMP",
        AgentId::OpenCode => "OpenCode",
        AgentId::Web => "Interfaz web externa",
        AgentId::DeepSeek => "DeepSeek harness",
    }
}

/// `kind` para `GET /api/agents` (`cli` o `web`).
pub fn agent_kind(id: AgentId) -> &'static str {
    match id {
        AgentId::Web => "web",
        _ => "cli",
    }
}

/// Id de modelo CLI para `pi`/`omp`/`opencode`, como lo pide el binario:
/// `<provider>/<id>`.
///
/// `served` es el id REALMENTE servido por el motor (`agents::served_model_id`,
/// p. ej. `Ternary-Bonsai-2-27B-PTQ1_0`). Antes esto era un literal por agente
/// (`localmind/qwen3.8-27b`), así que con Bonsai cargado el lanzador escribía
/// "Qwen" en el config privado del agente: la etiqueta mentía. Ahora el nombre
/// que ve el agente es el del motor que lo va a atender.
///
/// El prefijo `localmind/` se conserva porque es el nombre del provider en la
/// config privada, no un alias de modelo: el id que va detrás es el que decide
/// qué se anuncia. Los agentes ya configurados con el alias `localmind` (o con
/// el nombre del modelo anterior) siguen conectando porque el proxy reescribe
/// cualquier `model` pedido al servido (D-2).
pub fn cli_model(id: AgentId, served: &str) -> String {
    let model = if served.trim().is_empty() { "localmind" } else { served.trim() };
    match id {
        AgentId::Pi | AgentId::Omp | AgentId::OpenCode => format!("localmind/{}", model),
        _ => String::new(),
    }
}

/// Binario del agente en PATH (lo mismo que usa el lanzador).
/// `None` para `web` (no es un binario).
/// `deepseek` es el harness oficial `@deepseek-ai/dsh` (npm global): el
/// binario real es el shim `%APPDATA%\npm\dsh.cmd`, así que además de PATH se
/// comprueban `%APPDATA%\npm\dsh.cmd` y `dsh.cmd`. `opencode` igual con
/// `%APPDATA%\npm\opencode.cmd` (instalado aquí como `opencode-ai` global).
pub fn agent_bin(id: AgentId) -> Option<&'static str> {
    match id {
        AgentId::Pi => Some("pi"),
        AgentId::Omp => Some("omp"),
        AgentId::OpenCode => Some("opencode"),
        AgentId::DeepSeek => Some("dsh"),
        AgentId::Web => None,
    }
}

/// ¿Hay ejecutable localizable? `web` siempre disponible (es el navegador).
pub fn agent_available(id: AgentId) -> bool {
    match id {
        AgentId::Web => true,
        AgentId::Pi | AgentId::Omp => match agent_bin(id) {
            Some(bin) => bin_present(bin),
            None => false,
        },
        AgentId::DeepSeek => deepseek_installed(),
        AgentId::OpenCode => opencode_installed(),
    }
}

/// Shim `%APPDATA%\npm\<bin>.cmd` (npm global en Windows): el binario real de
/// `dsh`/`opencode` vive ahí aunque no esté en el PATH del servicio.
fn npm_shim_present(bin: &str) -> bool {
    if let Ok(appdata) = std::env::var("APPDATA") {
        let shim = PathBuf::from(appdata).join("npm").join(format!("{}.cmd", bin));
        if shim.is_file() {
            return true;
        }
    }
    PathBuf::from(format!("{}.cmd", bin)).is_file()
}
/// ¿Está instalado OpenCode? `opencode` en PATH o `%APPDATA%\npm\opencode.cmd`.
pub fn opencode_installed() -> bool {
    if bin_present("opencode") {
        return true;
    }
    npm_shim_present("opencode")
}
/// ¿Está instalado el harness DeepSeek? `dsh` en PATH o `%APPDATA%\npm\dsh.cmd`.
pub fn deepseek_installed() -> bool {
    if bin_present("dsh") {
        return true;
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        let shim = std::path::PathBuf::from(appdata).join("npm").join("dsh.cmd");
        if shim.is_file() {
            return true;
        }
    }
    // Último recurso: `dsh.cmd` resoluble desde el cwd (como hace cmd.exe).
    std::path::PathBuf::from("dsh.cmd").is_file()
}

/// Directorio aislado del harness (`DSH_HOME`): nunca toca `~/.dsh`.
pub fn deepseek_home() -> Option<PathBuf> {
    std::env::var("APPDATA")
        .map(|a| PathBuf::from(a).join("LocalMind").join("agents").join("deepseek"))
        .ok()
}

/// Dir privado de OpenCode (aisla XDG data/config/cache/state): nunca toca
/// `~/.config/opencode` ni `~/.local/share/opencode` del usuario.
#[allow(dead_code)]
pub fn opencode_home() -> Option<PathBuf> {
    std::env::var("APPDATA")
        .map(|a| PathBuf::from(a).join("LocalMind").join("agents").join("opencode"))
        .ok()
}

/// Env del lanzador DeepSeek (la clave NUNCA va en ficheros: se exporta como
/// `LOCALMIND_API_KEY` leída de `%APPDATA%\LocalMind\gateway.key`).
/// Solo la usan los tests: el lanzador real exporta la clave por `set /p`.
#[allow(dead_code)]
pub fn deepseek_env(gateway_key: &str) -> Vec<(String, String)> {
    let home = deepseek_home()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    vec![
        ("DSH_HOME".to_string(), home),
        ("DSH_TELEMETRY_MODE".to_string(), "DISABLED".to_string()),
        ("DSH_PERMISSION_MODE".to_string(), "workspace-write".to_string()),
        ("LOCALMIND_API_KEY".to_string(), gateway_key.to_string()),
    ]
}

/// Escapar un argumento para `cmd.exe` entrecomillado con `"` (duplica las
/// comillas; `&` queda inofensivo dentro de las comillas).
pub fn cmd_quote(arg: &str) -> String {
    format!("\"{}\"", arg.replace('"', "\"\""))
}

/// Línea interna del harness DeepSeek (`dsh --profile headless ["<tarea>"]`).
/// `task=None` o vacía = modo interactivo (se abre el terminal con el
/// harness y el dueño conversa con su modelo local); `Some(tarea)` = one-shot
/// headless con esa tarea citada de forma segura.
pub fn deepseek_inner_cmd(task: Option<&str>) -> String {
    let mut cmd = "dsh --profile headless".to_string();
    if let Some(t) = task.map(str::trim).filter(|t| !t.is_empty()) {
        cmd.push(' ');
        cmd.push_str(&cmd_quote(t));
    }
    cmd
}

/// Línea completa para `cmd.exe /k` (o `wt … cmd.exe /k`): prefijo `cd /d`,
/// env del harness y la clave leída del fichero (nunca en argv).
pub fn deepseek_cmdline(cd_prefix: &str, inner: &str) -> String {
    // `set /p` con redirección evita exponer la clave en la línea visible.
    let home = deepseek_home()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let key_file = std::env::var("APPDATA")
        .map(|a| PathBuf::from(a).join("LocalMind").join("gateway.key"))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    format!(
        "{}set \"DSH_HOME={}\" && set \"DSH_TELEMETRY_MODE=DISABLED\" && set \"DSH_PERMISSION_MODE=workspace-write\" && set /p LOCALMIND_API_KEY<\"{}\" && {}",
        cd_prefix, home, key_file, inner
    )
}

/// Patch de perfil Cordis del harness (`profiles/headless/cordis.patch.yml`,
/// ver `docs/agents/deepseek-harness.md` §3): apunta el provider `localmind`
/// al GATEWAY (`http://127.0.0.1:<http_port>/v1`, nunca al motor) con el
/// contexto VIVO. La clave viaja por `apiKeyEnv` (nombre de env, nunca cruda).
/// `model_id` es el alias estable (`localmind`); `maxTokens` fijo 4096 (doc).
pub fn deepseek_profile_patch(http_port: u16, context: usize, model_id: &str) -> String {
    format!(
        "# LocalMind: punto del harness dsh contra el modelo local (generado).\n# - Proveedor OpenAI-compatible en http://127.0.0.1:{port}/v1 (protocolo openai-completions).\n# - Clave via env LOCALMIND_API_KEY (el lanzador la lee de %APPDATA%\\LocalMind\\gateway.key).\n# - Compat: el gateway/proxy LocalMind solo habla chat/completions clasico:\n#   sin rol \"developer\" (usa \"system\") y cap de salida como \"max_tokens\".\n- id: agent-default-model\n  config:\n    provider: localmind\n    model: {model}\n- id: llm-pi-ai\n  config:\n    providers:\n      localmind:\n        displayName: LocalMind\n        apiKeyEnv: LOCALMIND_API_KEY\n        api: openai-completions\n        baseURL: http://127.0.0.1:{port}/v1\n        compat:\n          supportsDeveloperRole: false\n          maxTokensField: max_tokens\n          supportsReasoningEffort: false\n        models:\n          - id: {model}\n            name: LocalMind local model\n            contextWindow: {context}\n            maxTokens: 4096\n",
        port = http_port,
        model = model_id,
        context = context,
    )
}

/// Ruta del patch dentro del dir privado (`DSH_HOME`).
pub fn deepseek_patch_path(home: &std::path::Path) -> PathBuf {
    home.join("profiles").join("headless").join("cordis.patch.yml")
}

/// Escribir el patch solo si cambia (tmp+rename atómico; mtime estable si el
/// contenido es idéntico). Crea el dir `profiles/headless` si falta.
/// Devuelve `Ok(true)` si escribió, `Ok(false)` si ya estaba igual.
pub fn write_deepseek_patch(home: &std::path::Path, content: &str) -> Result<bool, String> {
    let dest = deepseek_patch_path(home);
    if let Some(parent) = dest.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Err(format!("No se pudo crear {}: {}", parent.display(), e));
        }
    }
    if let Ok(cur) = std::fs::read_to_string(&dest) {
        if cur == content {
            return Ok(false);
        }
    }
    let tmp = dest.with_extension("yml.tmp");
    if let Err(e) = std::fs::write(&tmp, content) {
        return Err(format!("No se pudo escribir {}: {}", tmp.display(), e));
    }
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("No se pudo activar {}: {}", dest.display(), e));
    }
    Ok(true)
}

/// ¿Existe `bin` resoluble en PATH? (misma detección que usa el lanzador).
pub fn bin_present(bin: &str) -> bool {
    let needle = bin.trim().to_lowercase();
    if needle.is_empty() {
        return false;
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    // En Windows el cwd también resuelve.
    if cfg!(windows) {
        if let Ok(cwd) = std::env::current_dir() {
            dirs.push(cwd);
        }
    }
    for dir in dirs {
        for ext in exe_suffixes() {
            let cand = if ext.is_empty() {
                dir.join(&needle)
            } else {
                dir.join(format!("{}.{}", needle, ext))
            };
            if is_executable_file(&cand) {
                return true;
            }
        }
    }
    false
}

#[cfg(windows)]
fn exe_suffixes() -> Vec<String> {
    let pathext = std::env::var_os("PATHEXT")
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| ".EXE".to_string());
    let mut out: Vec<String> = pathext
        .split(';')
        .map(|s| s.trim().trim_start_matches('.').to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    out.push(String::new());
    out
}

#[cfg(windows)]
fn is_executable_file(p: &PathBuf) -> bool {
    p.is_file()
}

#[cfg(not(windows))]
fn is_executable_file(p: &PathBuf) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.is_file()
        && std::fs::metadata(p)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

/// Ficha para `GET /api/agents` (orden fijo de `AGENT_ORDER`).
pub fn agents_list_json() -> String {
    let items: Vec<serde_json::Value> = AGENT_ORDER
        .iter()
        .filter_map(|id| parse_agent_id(id))
        .map(|id| {
            let raw = match id {
                AgentId::Pi => "pi",
                AgentId::Omp => "omp",
                AgentId::OpenCode => "opencode",
                AgentId::Web => "web",
                AgentId::DeepSeek => "deepseek",
            };
            serde_json::json!({
                "id": raw,
                "label": agent_label(id),
                "kind": agent_kind(id),
                "available": agent_available(id),
            })
        })
        .collect();
    serde_json::json!({ "agents": items }).to_string()
}

/// Acción de lanzamiento (testeable sin navegador ni spawn).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchAction {
    /// CLI con dir privado (`pi`/`omp`).
    Cli(AgentId),
    /// Abrir el navegador en el puerto del motor (`web`).
    Web,
    /// Harness pendiente: responde 501 (`deepseek` sin binario).
    PendingHarness,
}

/// `web` mapea a la acción del navegador; `deepseek` sin binario instalado
/// mapea a harness pendiente (501). El llamador decide con `bin_present`.
pub fn launch_action_for(id: AgentId, deepseek_installed: bool) -> LaunchAction {
    match id {
        AgentId::Web => LaunchAction::Web,
        AgentId::DeepSeek if !deepseek_installed => LaunchAction::PendingHarness,
        other => LaunchAction::Cli(other),
    }
}

/// Mensaje español del 501 cuando el harness no está instalado.
pub fn deepseek_not_configured_msg() -> String {
    "El harness de DeepSeek todavía no está configurado".to_string()
}

/// Sufijo `--thinking <nivel>` (mismo que el lanzador actual).
/// Sin `effort` (campo omitido) => `low`: la fase de razonamiento domina la
/// latencia al primer token (medido: ~74 s con thinking al máximo frente a
/// ~1 s con `low`), así que el default arranca en el modo más rápido. Cada
/// valor explícito (`off`/`low`/`medium`/`high`/`max`) se respeta tal cual;
/// cualquier otro string conserva el fallback previo (`max`).
pub fn effort_flag(effort: Option<&str>) -> &'static str {
    match effort {
        Some("off") => " --thinking off",
        Some("low") => " --thinking low",
        Some("medium") => " --thinking medium",
        Some("high") => " --thinking high",
        Some("max") => " --thinking max",
        None => " --thinking low",
        _ => " --thinking max",
    }
}

/// Línea de comando interna del CLI (`pi`/`omp`).
/// `agent_path` es el dir privado ya escrito por `agents::write_agent_dir`.
/// `http_port` es el puerto del GATEWAY (no el del motor): el CLI habla con
/// `http://127.0.0.1:<http_port>/v1` para pasar por clave/aliasing/usage.
/// `served` = id del modelo realmente cargado (ver `cli_model`).
pub fn cli_inner_cmd(
    id: AgentId,
    cd_prefix: &str,
    http_port: u16,
    gateway_key: &str,
    agent_path: &str,
    effort: Option<&str>,
    allow_home: &str,
    served: &str,
) -> String {
    let bin = agent_bin(id).unwrap_or("");
    let model = cli_model(id, served);
    let model_flag = match id {
        AgentId::Pi => format!("--provider localmind --model {}", model),
        _ => format!("--model {}", model),
    };
    format!(
        "{}set \"OPENAI_BASE_URL=http://127.0.0.1:{}/v1\" && set \"OPENAI_API_KEY={}\" && set \"PI_CODING_AGENT_DIR={}\" && {} {}{}{}",
        cd_prefix,
        http_port,
        gateway_key,
        agent_path,
        bin,
        model_flag,
        effort_flag(effort),
        allow_home
    )
}

/// Config `opencode.json` del provider `localmind` (verificado v1.18.10 con
/// `opencode debug config` + one-shot real): apunta al GATEWAY
/// (`http://127.0.0.1:<http_port>/v1`) con el contexto VIVO. Se escribe como
/// FICHERO en el dir privado (`XDG_CONFIG_HOME/opencode/opencode.json`),
/// NUNCA por `OPENCODE_CONFIG_CONTENT`: el `set "VAR=<json>"` de cmd.exe
/// corrompe el JSON (las `\"` sobreviven literales y opencode lo rechaza con
/// `ProviderModelNotFoundError`; probado: `{\"a\":1}` → `InvalidSymbol`).
/// `api_key_mode`: `env` = referencia `{env:LOCALMIND_API_KEY}` (la clave se
/// exporta por `set /p`); `inline` = clave cruda (solo tests, jamás argv).
/// `model_id` es el alias corto (`qwen3.8-27b`, sin prefijo `localmind/`).
#[allow(dead_code)]
pub fn opencode_config_content(http_port: u16, context: usize, model_id: &str) -> String {
    opencode_config_json(http_port, context, model_id, None)
}

/// Núcleo testeable: construye el JSON con la clave según el modo.
/// `api_key=None` → `{env:LOCALMIND_API_KEY}` (lo que escribe el lanzador).
pub fn opencode_config_json(http_port: u16, context: usize, model_id: &str, api_key: Option<&str>) -> String {
    let key = api_key.unwrap_or("{env:LOCALMIND_API_KEY}");
    serde_json::json!({
        "$schema": "https://opencode.ai/config.json",
        "model": format!("localmind/{}", model_id),
        "small_model": format!("localmind/{}", model_id),
        "provider": {
            "localmind": {
                "npm": "@ai-sdk/openai-compatible",
                "name": "LocalMind",
                "options": {
                    "baseURL": format!("http://127.0.0.1:{}/v1", http_port),
                    "apiKey": key
                },
                "models": {
                    model_id: {
                        "name": model_id,
                        "limit": { "context": context, "output": 8192 },
                        "modalities": { "input": ["text"], "output": ["text"] }
                    }
                }
            }
        }
    })
    .to_string()
}

/// Ruta del fichero de config dentro del dir privado:
/// `<home>/config/opencode/opencode.json` (`home` = dir `agents/opencode`).
/// Solo ahí; jamás `~/.config/opencode` del usuario.
pub fn opencode_config_path(home: &std::path::Path) -> PathBuf {
    home.join("config").join("opencode").join("opencode.json")
}

/// Escribir la config solo si cambia (tmp+rename atómico; mtime estable si el
/// contenido es idéntico). Crea el dir padre si falta.
/// Devuelve `Ok(true)` si escribió, `Ok(false)` si ya estaba igual.
pub fn write_opencode_config(home: &std::path::Path, content: &str) -> Result<bool, String> {
    let dest = opencode_config_path(home);
    if let Some(parent) = dest.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Err(format!("No se pudo crear {}: {}", parent.display(), e));
        }
    }
    if let Ok(cur) = std::fs::read_to_string(&dest) {
        if cur == content {
            return Ok(false);
        }
    }
    let tmp = dest.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, content) {
        return Err(format!("No se pudo escribir {}: {}", tmp.display(), e));
    }
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("No se pudo activar {}: {}", dest.display(), e));
    }
    Ok(true)
}

/// Env XDG aislado de OpenCode + `LOCALMIND_API_KEY` (solo tests: el lanzador
/// real exporta la clave por `set /p` desde `gateway.key`, nunca en argv).
#[allow(dead_code)]
pub fn opencode_env(home: &str, gateway_key: &str) -> Vec<(String, String)> {
    vec![
        ("XDG_DATA_HOME".to_string(), format!("{}\\data", home)),
        ("XDG_CONFIG_HOME".to_string(), format!("{}\\config", home)),
        ("XDG_CACHE_HOME".to_string(), format!("{}\\cache", home)),
        ("XDG_STATE_HOME".to_string(), format!("{}\\state", home)),
        ("LOCALMIND_API_KEY".to_string(), gateway_key.to_string()),
    ]
}

/// Línea interna de OpenCode: `opencode --model localmind/<id> ["run" "<tarea>"]`.
/// Sin `task` = TUI interactiva; con `task` = one-shot no interactivo
/// (`opencode run "<tarea>"`, verificado en `--help` v1.18.10).
pub fn opencode_inner_cmd(model_id: &str, task: Option<&str>) -> String {
    let mut cmd = format!("opencode --model localmind/{}", model_id);
    if let Some(t) = task.map(str::trim).filter(|t| !t.is_empty()) {
        cmd.push_str(" run ");
        cmd.push_str(&cmd_quote(t));
    }
    cmd
}

/// Línea completa para `cmd.exe /k`: prefijo `cd /d`, dirs XDG aislados y la
/// clave por `set /p` desde `gateway.key` (nunca en argv). La config ya NO
/// viaja por `OPENCODE_CONFIG_CONTENT` (el `set "VAR=<json>"` la corrompía:
/// `\"` literales → `InvalidSymbol`, provider ignorado y
/// `ProviderModelNotFoundError`); se escribe como fichero
/// `<home>/config/opencode/opencode.json` antes del spawn.
pub fn opencode_cmdline(cd_prefix: &str, home: &str, key_file: &str, inner: &str) -> String {
    format!(
        "{}set \"XDG_DATA_HOME={}\\data\" && set \"XDG_CONFIG_HOME={}\\config\" && set \"XDG_CACHE_HOME={}\\cache\" && set \"XDG_STATE_HOME={}\\state\" && set /p LOCALMIND_API_KEY<\"{}\" && {}",
        cd_prefix, home, home, home, home, key_file, inner
    )
}

/// Escapa un JSON para `set "VAR=<json>"` en cmd.exe (`"` → `\"`).
/// OBSOLETO para OpenCode (se conserva por compat de tests): el escape
/// produce JSON inválido para opencode (`InvalidSymbol`, provider ignorado).
/// La config se escribe como fichero (`write_opencode_config`).
#[allow(dead_code)]
pub fn cmd_set_json(json: &str) -> String {
    json.replace('"', "\\\"")
}

/// Rama de spawn de terminal (pura, sin spawn): `wt` → `cmd-start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnBranch {
    Wt,
    CmdStart,
}

/// Decisión de spawn testeable: `wt_usable` = el alias existe; `wt_ok` =
/// `Some(true/false)` si ya se intentó y el spawn devolvió Ok/Err, `None` si
/// aún no se intentó. Política "try + verify": solo se queda en `wt` si el
/// spawn devolvió `Ok`; ante `Err` cae a `cmd-start`.
#[allow(dead_code)]
pub fn spawn_branch(wt_usable: bool, wt_ok: Option<bool>) -> SpawnBranch {
    match (wt_usable, wt_ok) {
        (true, Some(true)) | (true, None) => SpawnBranch::Wt,
        _ => SpawnBranch::CmdStart,
    }
}

/// Spawn de terminal con verificación + fallback (`wt` → `cmd start`).
/// Devuelve `(ok, rama_usada, detalle)`: `detalle` es `pid <n>` o el error
/// textual; el llamador lo mete en el log y en el 500 si todo falla.
pub fn spawn_terminal(
    wt: Option<std::process::Command>,
    make_cmd: &dyn Fn() -> std::process::Command,
    req_dir: Option<&str>,
) -> (bool, SpawnBranch, String) {
    if let Some(mut c) = wt {
        if let Some(dir) = req_dir {
            if !dir.trim().is_empty() {
                c.current_dir(dir);
            }
        }
        match c.spawn() {
            Ok(child) => return (true, SpawnBranch::Wt, format!("pid {}", child.id())),
            Err(e) => {
                let mut c2 = make_cmd();
                if let Some(dir) = req_dir {
                    if !dir.trim().is_empty() {
                        c2.current_dir(dir);
                    }
                }
                match c2.spawn() {
                    Ok(child) => {
                        return (true, SpawnBranch::CmdStart, format!("pid {} (fallback tras fallo de wt: {})", child.id(), e))
                    }
                    Err(e2) => return (false, SpawnBranch::CmdStart, format!("wt: {}; cmd: {}", e, e2)),
                }
            }
        }
    }
    let mut c = make_cmd();
    if let Some(dir) = req_dir {
        if !dir.trim().is_empty() {
            c.current_dir(dir);
        }
    }
    match c.spawn() {
        Ok(child) => (true, SpawnBranch::CmdStart, format!("pid {}", child.id())),
        Err(e) => (false, SpawnBranch::CmdStart, format!("cmd: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agente_desconocido_rechazado() {
        assert_eq!(parse_agent_id("nope"), None);
        assert_eq!(parse_agent_id(""), None);
        assert_eq!(parse_agent_id("  "), None);
        assert_eq!(parse_agent_id("terminal"), None);
        assert_eq!(parse_agent_id("PI"), Some(AgentId::Pi));
        assert_eq!(parse_agent_id(" Omp "), Some(AgentId::Omp));
    }

    #[test]
    fn deepseek_sin_binario_es_501_pendiente() {
        assert_eq!(
            launch_action_for(AgentId::DeepSeek, false),
            LaunchAction::PendingHarness
        );
        assert_eq!(
            launch_action_for(AgentId::DeepSeek, true),
            LaunchAction::Cli(AgentId::DeepSeek)
        );
        assert!(deepseek_not_configured_msg().contains("todavía no está configurado"));
    }

    #[test]
    fn lista_de_agentes_cinco_ids_en_orden() {
        let v: serde_json::Value = serde_json::from_str(&agents_list_json()).unwrap();
        let ids: Vec<&str> = v["agents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["pi", "omp", "opencode", "web", "deepseek"]);
        let kinds: Vec<&str> = v["agents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, vec!["cli", "cli", "cli", "web", "cli"]);
        assert_eq!(v["agents"][3]["available"], serde_json::json!(true));
        assert_eq!(v["agents"][3]["label"], serde_json::json!("Interfaz web externa"));
    }

    #[test]
    fn web_mapea_a_accion_navegador() {
        assert_eq!(launch_action_for(AgentId::Web, false), LaunchAction::Web);
        assert_eq!(
            launch_action_for(AgentId::Pi, true),
            LaunchAction::Cli(AgentId::Pi)
        );
        // El CLI habla con el GATEWAY y lleva la clave (puerta con Bearer).
        assert_eq!(
            cli_inner_cmd(AgentId::Pi, "", 17861, "CLAVE-GW", "C:\\dir", Some("max"), "", "M-VIVO"),
            "set \"OPENAI_BASE_URL=http://127.0.0.1:17861/v1\" && set \"OPENAI_API_KEY=CLAVE-GW\" && set \"PI_CODING_AGENT_DIR=C:\\dir\" && pi --provider localmind --model localmind/M-VIVO --thinking max"
        );
        // omp igual: gateway + clave, sin hardcodear 8080.
        let omp = cli_inner_cmd(AgentId::Omp, "", 17860, "K2", "C:\\d", Some("low"), "", "M-VIVO");
        assert!(omp.contains("http://127.0.0.1:17860/v1"));
        assert!(omp.contains("OPENAI_API_KEY=K2"));
        assert!(!omp.contains("http://127.0.0.1:8080/v1"));
    }

    #[test]
    fn effort_default_low_y_explicitos_intactos() {
        // Default: omitido => `low` (el razonamiento domina el primer token).
        assert_eq!(effort_flag(None), " --thinking low");
        // Explícitos: cada valor aceptado se respeta tal cual.
        assert_eq!(effort_flag(Some("off")), " --thinking off");
        assert_eq!(effort_flag(Some("low")), " --thinking low");
        assert_eq!(effort_flag(Some("medium")), " --thinking medium");
        assert_eq!(effort_flag(Some("high")), " --thinking high");
        assert_eq!(effort_flag(Some("max")), " --thinking max");
        // La línea construida solo lleva `--thinking low` cuando no se pidió effort.
        let sin_effort = cli_inner_cmd(AgentId::Pi, "", 17861, "K", "C:\\d", None, "", "M-VIVO");
        assert!(sin_effort.contains("--thinking low"), "{}", sin_effort);
        let con_max = cli_inner_cmd(AgentId::Pi, "", 17861, "K", "C:\\d", Some("max"), "", "M-VIVO");
        assert!(con_max.contains("--thinking max"), "{}", con_max);
        assert!(!con_max.contains("--thinking low"), "{}", con_max);
    }

    #[test]
    fn deepseek_env_y_argv() {
        // Env exacto del doc: DSH_HOME aislado, telemetría off, workspace-write
        // y la clave exportada (nunca en ficheros).
        let env = deepseek_env("CLAVE-FALSA");
        let get = |k: &str| {
            env.iter()
                .find(|(kk, _)| kk == k)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        assert!(get("DSH_HOME").ends_with("agents\\deepseek") || get("DSH_HOME").ends_with("agents/deepseek"));
        assert_eq!(get("DSH_TELEMETRY_MODE"), "DISABLED");
        assert_eq!(get("DSH_PERMISSION_MODE"), "workspace-write");
        assert_eq!(get("LOCALMIND_API_KEY"), "CLAVE-FALSA");
        // Orden argv: `dsh --profile headless ["<tarea>"]`.
        assert_eq!(deepseek_inner_cmd(None), "dsh --profile headless");
        assert_eq!(deepseek_inner_cmd(Some("  ")), "dsh --profile headless");
        assert_eq!(
            deepseek_inner_cmd(Some("corre los tests")),
            "dsh --profile headless \"corre los tests\""
        );
        // La tarea citada no puede romper la línea: `&` queda dentro de las
        // comillas y `"` se duplica.
        let evil = deepseek_inner_cmd(Some("a\" & del C:\\"));
        assert!(evil.starts_with("dsh --profile headless \""));
        assert!(evil.contains("\"\""));
        assert!(!evil.contains("&& del"));
        // La cmdline completa nunca lleva la clave en argv: va por `set /p`.
        let full = deepseek_cmdline("cd /d \"C:\\proj\" && ", "dsh --profile headless");
        assert!(full.contains("set /p LOCALMIND_API_KEY<"));
        assert!(!full.contains("CLAVE-FALSA"));
        assert!(full.contains("dsh --profile headless"));
    }

    #[test]
    fn opencode_parse_y_label() {
        assert_eq!(parse_agent_id("opencode"), Some(AgentId::OpenCode));
        assert_eq!(parse_agent_id("opencode2"), Some(AgentId::OpenCode));
        assert_eq!(parse_agent_id(" OpenCode "), Some(AgentId::OpenCode));
        assert_eq!(agent_label(AgentId::OpenCode), "OpenCode");
        assert_eq!(agent_kind(AgentId::OpenCode), "cli");
        assert_eq!(agent_bin(AgentId::OpenCode), Some("opencode"));
        assert_eq!(cli_model(AgentId::OpenCode, "Ternary-Bonsai-2-27B-PTQ1_0"), "localmind/Ternary-Bonsai-2-27B-PTQ1_0");
        // Desconocidos siguen a 400.
        assert_eq!(parse_agent_id("codex"), None);
        assert_eq!(parse_agent_id("opencode3"), None);
    }

    #[test]
    fn opencode_payload_apunta_al_gateway() {
        // Payload con el puerto del GATEWAY (no el del motor) y contexto vivo;
        // la clave viaja por `{env:...}`, jamás cruda.
        let raw = opencode_config_content(17861, 32768, "qwen3.8-27b");
        assert!(!raw.contains("CLAVE-FALSA"));
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["model"], serde_json::json!("localmind/qwen3.8-27b"));
        assert_eq!(v["small_model"], serde_json::json!("localmind/qwen3.8-27b"));
        let prov = &v["provider"]["localmind"];
        assert_eq!(prov["npm"], serde_json::json!("@ai-sdk/openai-compatible"));
        assert_eq!(
            prov["options"]["baseURL"],
            serde_json::json!("http://127.0.0.1:17861/v1")
        );
        // La clave cruda jamás va en el payload: referencia `{env:...}`.
        assert_eq!(prov["options"]["apiKey"], serde_json::json!("{env:LOCALMIND_API_KEY}"));
        let m = &prov["models"]["qwen3.8-27b"];
        assert_eq!(m["limit"]["context"], serde_json::json!(32768));
        assert_eq!(m["limit"]["output"], serde_json::json!(8192));
        // Cambiar el puerto ligado cambia la URL (nada hardcodeado a 8080).
        let raw2 = opencode_config_content(17860, 65536, "qwen3.8-27b");
        assert!(raw2.contains("http://127.0.0.1:17860/v1"));
        assert!(!raw2.contains("http://127.0.0.1:8080/v1"));
        assert!(raw2.contains("\"context\":65536"));
    }

    #[test]
    fn opencode_argv_y_cmdline() {
        // Orden argv: `opencode --model localmind/<id> [run "<tarea>"]`.
        assert_eq!(
            opencode_inner_cmd("qwen3.8-27b", None),
            "opencode --model localmind/qwen3.8-27b"
        );
        assert_eq!(
            opencode_inner_cmd("qwen3.8-27b", Some("  ")),
            "opencode --model localmind/qwen3.8-27b"
        );
        assert_eq!(
            opencode_inner_cmd("qwen3.8-27b", Some("arregla el bug")),
            "opencode --model localmind/qwen3.8-27b run \"arregla el bug\""
        );
        // Inyección `&`/`"` queda dentro de las comillas.
        let evil = opencode_inner_cmd("qwen3.8-27b", Some("a\" & del C:\\"));
        assert!(evil.contains(" run \""));
        assert!(evil.contains("\"\""));
        assert!(!evil.contains("&& del"));
        // La cmdline aísla XDG y lee la clave por `set /p` (nunca en argv).
        // La config YA NO viaja por env (corrompía el JSON): se escribe como
        // fichero `config/opencode/opencode.json` en el dir privado.
        let home = "C:\\home-oc";
        let inner = opencode_inner_cmd("qwen3.8-27b", Some("t"));
        let full = opencode_cmdline("cd /d \"C:\\proj\" && ", home, "C:\\k", &inner);
        assert!(full.contains("set \"XDG_DATA_HOME=C:\\home-oc\\data\""));
        assert!(full.contains("set \"XDG_CONFIG_HOME=C:\\home-oc\\config\""));
        assert!(full.contains("set \"XDG_CACHE_HOME=C:\\home-oc\\cache\""));
        assert!(full.contains("set \"XDG_STATE_HOME=C:\\home-oc\\state\""));
        assert!(!full.contains("OPENCODE_CONFIG_CONTENT"));
        assert!(full.contains("set /p LOCALMIND_API_KEY<"));
        assert!(!full.contains("CLAVE-FALSA"));
        // El home aislado es el dir privado, no el del usuario.
        let h = opencode_home().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        assert!(h.ends_with("agents\\opencode") || h.ends_with("agents/opencode"));
        assert!(!h.is_empty());
    }

    #[test]
    fn opencode_config_fichero_gateway_y_estable() {
        // El fichero apunta al GATEWAY ligado con contexto vivo; con clave
        // `{env:}` por defecto y cruda solo si se pide explícito (tests).
        let a = opencode_config_json(17861, 32768, "qwen3.8-27b", None);
        assert!(a.contains("http://127.0.0.1:17861/v1"));
        assert!(!a.contains("http://127.0.0.1:8080/v1"));
        assert!(a.contains("{env:LOCALMIND_API_KEY}"));
        assert!(!a.contains("CLAVE-FALSA"));
        assert!(a.contains("\"context\":32768"));
        let inline = opencode_config_json(17861, 32768, "qwen3.8-27b", Some("K-CRUDA"));
        assert!(inline.contains("K-CRUDA"));
        // Otro puerto/contexto → difieren; mismo input ⇒ idéntico.
        let b = opencode_config_json(17862, 65536, "qwen3.8-27b", None);
        assert!(b.contains("http://127.0.0.1:17862/v1"));
        assert_ne!(a, b);
        assert_eq!(a, opencode_config_json(17861, 32768, "qwen3.8-27b", None));
        // Ruta dentro del dir privado y escritura solo-si-cambia.
        let dir = std::env::temp_dir().join(format!("lm-oc-cfg-{}", std::process::id()));
        let home = dir.join("opencode");
        let dest = opencode_config_path(&home);
        assert!(dest.to_string_lossy().contains("opencode.json"));
        assert!(dest.starts_with(&home));
        assert_eq!(write_opencode_config(&home, &a).unwrap(), true);
        assert!(dest.is_file());
        assert_eq!(write_opencode_config(&home, &a).unwrap(), false);
        assert_eq!(write_opencode_config(&home, &b).unwrap(), true);
        // `cmd_set_json` documenta el escape roto (se conserva, no se usa).
        assert_eq!(cmd_set_json("{\"a\":1}"), "{\\\"a\\\":1}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn spawn_branch_try_verify_y_fallback() {
        // Sin wt → cmd directo. Con wt sin intentar → wt. Con wt+Ok → wt.
        assert_eq!(spawn_branch(false, None), SpawnBranch::CmdStart);
        assert_eq!(spawn_branch(false, Some(true)), SpawnBranch::CmdStart);
        assert_eq!(spawn_branch(true, None), SpawnBranch::Wt);
        assert_eq!(spawn_branch(true, Some(true)), SpawnBranch::Wt);
        // Con wt pero spawn Err → fallback a cmd-start (el caso del alias
        // reparse de 0 bytes que devuelve Err al hacer spawn).
    }

    #[test]
    fn deepseek_patch_gateway_y_estable() {
        // El patch apunta al GATEWAY ligado (no al motor) con contexto vivo;
        // la clave viaja por `apiKeyEnv` (nombre), jamás cruda.
        let a = deepseek_profile_patch(17861, 32768, "localmind");
        assert!(a.contains("baseURL: http://127.0.0.1:17861/v1"));
        assert!(a.contains("contextWindow: 32768"));
        assert!(a.contains("model: localmind"));
        assert!(a.contains("apiKeyEnv: LOCALMIND_API_KEY"));
        assert!(a.contains("api: openai-completions"));
        assert!(a.contains("supportsDeveloperRole: false"));
        assert!(a.contains("maxTokensField: max_tokens"));
        assert!(a.contains("supportsReasoningEffort: false"));
        assert!(!a.contains("CLAVE-FALSA"));
        assert!(!a.contains("17860") || a.contains("17861"));
        // Otro puerto/contexto → difieren exactamente en esos campos.
        let b = deepseek_profile_patch(17862, 65536, "localmind");
        assert!(b.contains("baseURL: http://127.0.0.1:17862/v1"));
        assert!(b.contains("contextWindow: 65536"));
        assert_ne!(a, b);
        // Estable: mismo input ⇒ idéntico output (mtime estable).
        let a2 = deepseek_profile_patch(17861, 32768, "localmind");
        assert_eq!(a, a2);
        // Escritura solo-si-cambia: segunda vez no toca el fichero.
        let dir = std::env::temp_dir().join(format!("lm-ds-patch-{}", std::process::id()));
        let home = dir.join("deepseek");
        assert_eq!(write_deepseek_patch(&home, &a).unwrap(), true);
        let dest = deepseek_patch_path(&home);
        assert!(dest.is_file());
        assert_eq!(write_deepseek_patch(&home, &a2).unwrap(), false);
        assert_eq!(write_deepseek_patch(&home, &b).unwrap(), true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- LM-MOD-3: lo que el lanzador anuncia sale del motor, no de un literal ----

    /// El id que va al `--model` del binario y al config privado es el del
    /// modelo realmente servido, para los TRES agentes con config.
    ///
    /// Falla sobre el código original: `cli_model` devolvía
    /// `localmind/qwen3.8-27b` para omp y opencode y `localmind/localmind`
    /// para pi, con independencia del modelo cargado (el defecto reportado:
    /// con Bonsai cargado, OMP respondía `"model":"localmind/qwen3.8-27b"`).
    #[test]
    fn el_lanzador_anuncia_el_modelo_servido_no_un_literal() {
        let bonsai = "Ternary-Bonsai-2-27B-PTQ1_0";
        let qwen = "Qwen3.8-27B-IQ4_XS_4BPW";

        for id in [AgentId::Pi, AgentId::Omp, AgentId::OpenCode] {
            let con_bonsai = cli_model(id, bonsai);
            let con_qwen = cli_model(id, qwen);
            assert_ne!(con_bonsai, con_qwen, "{:?}: el id no depende del modelo servido", id);
            assert!(con_bonsai.contains("Bonsai"), "{:?} no anuncia Bonsai: {}", id, con_bonsai);
            assert!(con_qwen.contains("Qwen"), "{:?} no anuncia Qwen: {}", id, con_qwen);
            // El prefijo de provider se conserva: no es un alias de modelo.
            assert!(con_bonsai.starts_with("localmind/"), "{:?} perdió el provider: {}", id, con_bonsai);
        }

        // El `--model` que se ejecuta lleva el mismo id que la respuesta JSON.
        let linea = cli_inner_cmd(AgentId::Omp, "", 17860, "K", "C:\\d", Some("low"), "", bonsai);
        assert!(linea.contains("--model localmind/Ternary-Bonsai-2-27B-PTQ1_0"), "{}", linea);
        assert!(!linea.contains("qwen3.8-27b"), "la línea aún nombra al modelo viejo: {}", linea);
    }

    /// Sin id servido (`""` = motor sin modelo) el lanzador cae al alias
    /// estable `localmind`, que el proxy reescribe al servido (D-2). No inventa
    /// un nombre de modelo ni deja el de la sesión anterior pegado.
    #[test]
    fn sin_modelo_servido_el_lanzador_usa_el_alias_estable() {
        for id in [AgentId::Pi, AgentId::Omp, AgentId::OpenCode] {
            assert_eq!(cli_model(id, ""), "localmind/localmind", "{:?}", id);
            assert_eq!(cli_model(id, "   "), "localmind/localmind", "{:?}", id);
        }
        // `web` y `deepseek` no pasan id por `--model` (van por otro camino).
        assert_eq!(cli_model(AgentId::Web, "M"), "");
        assert_eq!(cli_model(AgentId::DeepSeek, "M"), "");
    }

    /// El patch de Cordis nombra el modelo servido y su contexto vivo.
    /// Falla sobre el código original, que fijaba `"localmind"` en el patch.
    #[test]
    fn el_patch_de_deepseek_nombra_el_modelo_y_el_contexto_vivos() {
        let a = deepseek_profile_patch(17861, 32768, "Ternary-Bonsai-2-27B-PTQ1_0");
        assert!(a.contains("model: Ternary-Bonsai-2-27B-PTQ1_0"), "{}", a);
        assert!(a.contains("id: Ternary-Bonsai-2-27B-PTQ1_0"), "{}", a);
        assert!(a.contains("contextWindow: 32768"), "{}", a);
        // Cambiar de modelo cambia el patch (se reescribe en cada lanzamiento).
        let b = deepseek_profile_patch(17861, 32768, "Qwen3.8-27B-IQ4_XS_4BPW");
        assert_ne!(a, b);
        // Cambiar de contexto también (nada queda colgado de una sesión vieja).
        let c = deepseek_profile_patch(17861, 262144, "Ternary-Bonsai-2-27B-PTQ1_0");
        assert!(c.contains("contextWindow: 262144"), "{}", c);
        assert_ne!(a, c);
    }
}
