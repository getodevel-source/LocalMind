//! Configs privadas por lanzamiento para pi/omp (D-5, D-6; corrige D-2, D-4).
//!
//! - JAMÁS toca `%USERPROFILE%\.pi` ni `%USERPROFILE%\.omp`.
//! - Por lanzamiento crea `%APPDATA%\LocalMind\agents\<agent>\` con `models.json`
//!   (pi) y `models.yml` + `config.yml` (omp), y el lanzador exporta
//!   `PI_CODING_AGENT_DIR=<ese dir>` (pi y omp —fork de pi— leen
//!   `<APPNAME>_CODING_AGENT_DIR` vía `dist/config.js`).
//! - `baseUrl` apunta al GATEWAY (`http://127.0.0.1:<http_port>/v1`), nunca al
//!   motor: así el tráfico pasa por la puerta con clave, el aliasing y
//!   `usage.jsonl`. El `contextWindow` sigue viniendo del estado VIVO.
//! - Se conserva la forma JSON/YML que ya funcionaba con los binarios
//!   instalados; solo cambia DÓNDE va, QUÉ puerto/contexto/modelo nombra.
//! - Si el motor no corre, el endpoint de lanzamiento devuelve 4xx/409 y NO se
//!   escriben configs que apunten a un puerto muerto (D-4).

use std::path::PathBuf;

use crate::process::ServerStatus;

/// Error español para el endpoint de lanzamiento cuando no hay motor.
pub fn engine_down_error(st: &ServerStatus) -> String {
    if st.port == 0 || st.status != "running" {
        format!(
            "{{\"error\":\"motor_apagado\",\"message\":\"El motor no está en ejecución (estado: {}). Inícielo primero con /api/start.\"}}",
            st.status
        )
    } else {
        String::new()
    }
}

/// ¿Hay motor vivo al que apuntar? Puerto real + estado running.
pub fn engine_live(st: &ServerStatus) -> bool {
    st.port > 0 && st.status == "running"
}

/// El id servido por el motor: `st.model` (p. ej. `qwen3.8-27b-....gguf`
/// sin extensión o el nombre tal cual lo publica `ServerStatus`).
///
/// Los CLIs se configuran con los alias estables (`localmind`, `qwen3.8-27b`)
/// y el proxy reescribe cualquier `model` pedido al id servido (LM-PXY-4/5).
/// Aquí solo se necesita el contexto y el puerto vivos; el id se conserva en
/// los alias estables que ya conocen pi/omp.
pub fn agent_dir(agent: &str) -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("LocalMind"));
    base.join("LocalMind").join("agents").join(agent)
}

/// `models.json` de pi con la forma que ya funcionaba (merge) pero dirigida al
/// GATEWAY (`http://127.0.0.1:<http_port>/v1`), no al motor: así el tráfico
/// pasa por la puerta con clave, el aliasing y `usage.jsonl`.
/// Conserva ids `localmind` + `qwen3.8-27b`.
pub fn pi_models_json(http_port: u16, context: usize, key: &str, vision: bool) -> String {
    let max_toks = (context / 2).min(16384);
    let input: Vec<&str> = if vision { vec!["text", "image"] } else { vec!["text"] };
    // thinkingLevelMap verificado contra el esquema pi (`model-config.d.ts`):
    let thinking_map = serde_json::json!({
        "minimal": "low", "low": "low", "medium": "medium",
        "high": "xhigh", "xhigh": "xhigh", "max": null
    });
    // Coste local nulo (evita que pi estime gasto) y `system` en vez de
    // `developer` (los motores locales no conocen el rol `developer`).
    let cost = serde_json::json!({"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0});
    let compat = serde_json::json!({"supportsDeveloperRole": false});
    let model_entry = |id: &str, name: &str| {
        serde_json::json!({
            "id": id,
            "name": name,
            "contextWindow": context,
            "maxTokens": max_toks,
            "reasoning": true,
            "input": input,
            "cost": cost,
            "thinkingLevelMap": thinking_map,
            "compat": compat,
        })
    };
    let v = serde_json::json!({
        "providers": {
            "localmind": {
                "name": "LocalMind",
                "baseUrl": format!("http://127.0.0.1:{}/v1", http_port),
                "apiKey": key,
                "api": "openai-completions",
                "models": [
                    model_entry("localmind", "LocalMind Active Model"),
                    model_entry("qwen3.8-27b", "Qwen 3.8 27B (LocalMind)"),
                ]
            }
        }
    });
    serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".to_string())
}

/// `models.yml` de omp con la plantilla que ya funcionaba, dirigida al
/// GATEWAY y al contexto vivo, con la clave local (`apiKey`, no `auth: none`,
/// porque `/v1/*` exige Bearer desde D-7).
/// `vision` igual que en pi: `[text]` salvo mmproj verificado (cuando la
/// visión aterrice, pasar `vision=true` desde el estado vivo del motor).
pub fn omp_models_yml(http_port: u16, context: usize, key: &str, vision: bool) -> String {
    let input = if vision { "[text, image]" } else { "[text]" };
    format!(
        r#"providers:
  localmind:
    baseUrl: http://127.0.0.1:{}/v1
    apiKey: {}
    api: openai-completions
    models:
      - id: qwen3.8-27b
        name: Qwen 3.8 27B (LocalMind)
        reasoning: true
        input: {}
        contextWindow: {}
        maxTokens: 16384
        cost:
          input: 0
          output: 0
          cacheRead: 0
          cacheWrite: 0
        thinkingLevelMap:
          minimal: low
          low: low
          medium: medium
          high: xhigh
          xhigh: xhigh
          max: null
        compat:
          supportsReasoningEffort: false
          reasoningContentField: reasoning_content
          supportsDeveloperRole: false
      - id: localmind
        name: LocalMind Active Model
        reasoning: true
        input: {}
        contextWindow: {}
        maxTokens: 16384
        cost:
          input: 0
          output: 0
          cacheRead: 0
          cacheWrite: 0
        thinkingLevelMap:
          minimal: low
          low: low
          medium: medium
          high: xhigh
          xhigh: xhigh
          max: null
        compat:
          supportsReasoningEffort: false
          reasoningContentField: reasoning_content
          supportsDeveloperRole: false
"#,
        http_port, key, input, context, input, context
    )
}

/// `config.yml` mínimo de omp: fija `modelRoles.default` al provider local para
/// que omp resuelva el provider desde este dir privado (verificado en máquina:
/// sin `PI_CODING_AGENT_DIR` el mismo comando fallaba con
/// `Model "<id>" not found ... Or create ...\.omp\agent\models.yml`).
pub fn omp_config_yml() -> String {
    "modelRoles:\n  default: localmind/localmind\n".to_string()
}

/// ¿Visión verificada para el motor vivo? Hoy el mmproj está apagado por
/// defecto (LM-MOD-2, `MmprojConfig.auto=false`): sin prueba de `config.rs`
/// (archivo ajeno) se declara `[text]` honesto. Cuando la visión aterrice,
/// leer aquí el estado vivo (p. ej. flag mmproj del `ServerStatus`/config) y
/// devolver true solo entonces.
pub fn vision_enabled() -> bool {
    false
}

/// Escribir el dir privado del agente. Devuelve el dir o un mensaje español.
/// `key` es la clave local ya cargada por el servidor (fuente única:
/// `HttpServer::start` → `handle_launch` → aquí).
/// `http_port` es el puerto del GATEWAY (no el del motor): los CLIs hablan
/// con `http://127.0.0.1:<http_port>/v1` para pasar por clave/aliasing/usage.
pub fn write_agent_dir(agent: &str, http_port: u16, context: usize, key: &str) -> Result<PathBuf, String> {
    let vision = vision_enabled();
    let dir = agent_dir(agent);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Err(format!("No se pudo crear {}: {}", dir.display(), e));
    }
    if agent == "pi" {
        let p = dir.join("models.json");
        if let Err(e) = std::fs::write(&p, pi_models_json(http_port, context, key, vision)) {
            return Err(format!("No se pudo escribir {}: {}", p.display(), e));
        }
    } else {
        let m = dir.join("models.yml");
        if let Err(e) = std::fs::write(&m, omp_models_yml(http_port, context, key, vision)) {
            return Err(format!("No se pudo escribir {}: {}", m.display(), e));
        }
        let c = dir.join("config.yml");
        if let Err(e) = std::fs::write(&c, omp_config_yml()) {
            return Err(format!("No se pudo escribir {}: {}", c.display(), e));
        }
    }
    Ok(dir)
}

/// ¿La carpeta de proyecto es el home? omp se niega a arrancar en `%USERPROFILE%`
/// sin `--allow-home`.
pub fn is_home_dir(dir: &str) -> bool {
    let d = dir.trim().trim_matches('"');
    if d.is_empty() {
        return false;
    }
    let norm = |s: &str| {
        s.replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    let want = norm(d);
    if let Ok(home) = std::env::var("USERPROFILE") {
        if !home.is_empty() && want == norm(&home) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pi_json_apunta_al_gateway() {
        // El puerto es el del GATEWAY (17860 ligado), no el del motor (8080).
        let s = pi_models_json(17861, 65536, "clave-falsa", false);
        assert!(s.contains("http://127.0.0.1:17861/v1"));
        assert!(!s.contains("http://127.0.0.1:8080/v1"));
        assert!(s.contains("\"contextWindow\": 65536") || s.contains("\"contextWindow\":65536") || s.contains("65536"));
        assert!(s.contains("\"localmind\""));
        assert!(s.contains("qwen3.8-27b"));
        // Otro puerto ligado → otro baseUrl (nada hardcodeado).
        let s2 = pi_models_json(17860, 32768, "clave-falsa", false);
        assert!(s2.contains("http://127.0.0.1:17860/v1"));
    }

    #[test]
    fn omp_yml_apunta_al_gateway() {
        let s = omp_models_yml(17861, 65536, "clave-falsa", false);
        assert!(s.contains("http://127.0.0.1:17861/v1"));
        assert!(!s.contains("http://127.0.0.1:8080/v1"));
        assert!(s.contains("contextWindow: 65536"));
        let c = omp_config_yml();
        assert!(c.contains("modelRoles:") && c.contains("default:"));
    }

    #[test]
    fn configs_llevan_la_clave_y_texto_sin_vision() {
        // Clave falsa de prueba: jamás la real de %APPDATA%.
        let key = "clave-falsa-de-prueba-0123456789abcdef";
        let pi = pi_models_json(8099, 65536, key, false);
        assert!(pi.contains(key), "models.json sin apiKey");
        assert!(!pi.contains("auth"), "models.json no usa auth");
        let omp = omp_models_yml(8099, 65536, key, false);
        assert!(omp.contains(&format!("apiKey: {}", key)), "models.yml sin apiKey");
        assert!(!omp.contains("auth: none"), "models.yml aún declara auth: none");
        // Motor solo-texto: ninguna config anuncia imagen.
        assert!(!pi.contains("\"image\""), "pi anuncia image sin visión: {}", pi);
        assert!(!omp.contains("image"), "omp anuncia image sin visión: {}", omp);
        // La clave autoriza Bearer y sin clave se rechaza (puerta del gateway).
        let h = |f: &str, v: &str| tiny_http::Header::from_bytes(f.as_bytes(), v.as_bytes()).unwrap();
        assert!(crate::auth::is_authorized(
            &[h("Authorization", &format!("Bearer {}", key))],
            key
        ));
        assert!(!crate::auth::is_authorized(&[], key));
        assert!(!crate::auth::is_authorized(&[h("Authorization", "Bearer otra")], key));
        // Con visión verificada sí se anuncia imagen en ambas.
        let pi_v = pi_models_json(8099, 65536, key, true);
        let omp_v = omp_models_yml(8099, 65536, key, true);
        assert!(pi_v.contains("\"image\""), "pi con visión debe anunciar image");
        assert!(omp_v.contains("[text, image]"), "omp con visión debe anunciar image");
    }
    #[test]
    fn motor_apagado_da_error_y_no_vive() {
        let st = ServerStatus {
            status: "stopped".to_string(),
            is_healthy: false,
            pid: None,
            model: String::new(),
            context: 0,
            profile: String::new(),
            port: 0,
            idle_remaining_secs: None,
            last_error: None,
            verifying: false,
            starting_for_secs: 0,
            eta_secs: 0,
            decode_tps: None,
            acceptance_ok: None,
            acceptance_error: None,
        };
        assert!(!engine_live(&st));
        assert!(engine_down_error(&st).contains("motor_apagado"));
    }
}
