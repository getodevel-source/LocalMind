//! Servidor HTTP LocalMind: UI + `/api/*` + proxy OpenAI/Anthropic/Responses.
//!
//! Cambios de esta fase (ver reporte):
//! - D-7: clave local en `%APPDATA%\LocalMind\gateway.key`; `/v1/*` y `/api/*`
//!   exigen Bearer/x-api-key/cookie; `/`, `/index.html`, iconos siguen públicos
//!   y fijan cookie `lm_key` para la WebView.
//! - D-5/D-6: lanzadores pi/omp escriben configs privadas en
//!   `%APPDATA%\LocalMind\agents\<agent>\` desde el estado VIVO del motor;
//!   jamás tocan `%USERPROFILE%\.pi` ni `%USERPROFILE%\.omp`.
//! - D-2: alias `localmind`/`qwen3.8-27b` + `GET /v1/models`; cualquier `model`
//!   pedido se reescribe al id servido.
//! - Claude Code: `POST /v1/messages` (Anthropic). Codex: `POST /v1/responses`.
//! - Telemetría base: `usage.jsonl` por request proxyeado completado.

use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tiny_http::{Header, Response, Server, StatusCode};

use crate::config::ConfigStore;
use crate::process::{LogEvent, ProcessManager, ServerStatus, StartRequest};

pub struct HttpServer {
    port: u16,
}

/// Cabeceras CORS fijas (sin `Origin`): métodos y headers permitidos.
/// `Access-Control-Allow-Origin` ya NO se envía con `*` (SRS P29): solo se
/// refleja el `Origin` de la petición cuando es loopback
/// (`http://127.0.0.1|localhost|[::1][:puerto]`); cualquier otro origen no
/// recibe cabeceras CORS. Las lecturas de la WebView (mismo origen) no
/// necesitan CORS en absoluto.
fn cors_fixed_headers() -> [Header; 2] {
    [
        Header::from_bytes(
            &b"Access-Control-Allow-Methods"[..],
            &b"GET, POST, OPTIONS, PUT, DELETE"[..],
        )
        .unwrap(),
        Header::from_bytes(
            &b"Access-Control-Allow-Headers"[..],
            &b"Content-Type, Authorization, x-api-key"[..],
        )
        .unwrap(),
    ]
}

/// `Access-Control-Allow-Origin: <origen>` solo si el origen es loopback.
fn cors_origin_header(origin: Option<&str>) -> Option<Header> {
    let o = origin?;
    if !crate::meta::is_loopback_origin(o) {
        return None;
    }
    Header::from_bytes(&b"Access-Control-Allow-Origin"[..], o.as_bytes()).ok()
}

/// `Set-Cookie` de sesión. El valor sale de la clave local (64 hex de
/// `generate_key()` o del `gateway.key` de disco), así que es DINÁMICO: una
/// clave alterada con un byte de control haría fallar el parseo. Se devuelve
/// `Option` —igual que `cors_origin_header`— para que el llamante responda 500
/// en vez de entrar en `unwrap` con `panic = "abort"`.
fn session_cookie_header(gateway_key: &str) -> Option<Header> {
    Header::from_bytes(
        &b"Set-Cookie"[..],
        crate::auth::set_cookie_value(gateway_key).as_bytes(),
    )
    .ok()
}

/// Respuesta JSON con CORS solo para origen loopback (o sin CORS si `None`).
fn json_response_for_origin(
    status_code: u16,
    body: String,
    origin: Option<&str>,
) -> Response<Cursor<Vec<u8>>> {
    let mut resp = Response::from_string(body).with_status_code(StatusCode(status_code));
    let ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    resp.add_header(ct);
    for h in cors_fixed_headers() {
        resp.add_header(h);
    }
    if let Some(o) = cors_origin_header(origin) {
        resp.add_header(o);
    }
    resp
}

fn unauthorized_json_for_origin(origin: Option<&str>) -> Response<Cursor<Vec<u8>>> {
    json_response_for_origin(401, r#"{"error":"unauthorized"}"#.to_string(), origin)
}

// ---------------------------------------------------------------------------
// Parseo y validación de cuerpos POST (puros, testeables sin socket)
// ---------------------------------------------------------------------------
// conventions: `Result<_, String>` con mensajes en español, sin I/O y sin
// `unwrap`, igual que `translate.rs`: todo fallo del cliente es un 4xx que el
// handler convierte en respuesta.

/// `POST /api/start`: un cuerpo MAL FORMADO no puede degradarse a un arranque
/// con defaults. Antes, `serde_json::from_str(...).unwrap_or(default)` hacía
/// que `{"context":"abc"}` pasara todos los `validate_req_*` en vacío y
/// arrancara el motor con 200. Ahora es 400. Un cuerpo VACÍO sigue siendo
/// `StartRequest::default()` (llamadores que hacen POST sin cuerpo).
fn parse_start_body(body: &str) -> Result<StartRequest, String> {
    if body.trim().is_empty() {
        return Ok(StartRequest {
            model: None,
            profile: None,
            context: None,
            threads: None,
            priority: None,
        });
    }
    serde_json::from_str::<StartRequest>(body).map_err(|e| format!("JSON inválido: {}", e))
}

/// `POST /api/profiles/import`: el import NO puede saltarse los validadores que
/// `/api/profiles/save` sí aplica. Reusa los mismos predicados y nombra el
/// campo culpable, en el mismo formato `campo inválido: <campo>`.
fn parse_profiles_import(body: &str) -> Result<Vec<crate::config::HardwareProfile>, String> {
    let imported: Vec<crate::config::HardwareProfile> = serde_json::from_str(body)
        .map_err(|e| format!("JSON inválido: {}", e))?;
    if imported.is_empty() {
        return Err("La lista de perfiles está vacía".to_string());
    }
    for p in &imported {
        let bad = |campo: &str| format!("campo inválido: {}", campo);
        if !crate::config::profile_id_ok(&p.id) {
            return Err(bad("id"));
        }
        if !crate::config::profile_context_ok(p.context) {
            return Err(bad("context"));
        }
        if !crate::config::profile_cache_ram_ok(p.cache_ram) {
            return Err(bad("cache_ram"));
        }
        if !crate::config::profile_flags_ok(&p.extra_flags) {
            return Err(bad("extra_flags"));
        }
    }
    Ok(imported)
}

/// Añadir CORS a una respuesta ya construida: fijas + `Origin` si es loopback.
fn add_cors_for<R: Read>(resp: &mut Response<R>, origin: Option<&str>) {
    if let Some(o) = cors_origin_header(origin) {
        resp.add_header(o);
    }
    for h in cors_fixed_headers() {
        resp.add_header(h);
    }
}

fn status_json(st: ServerStatus) -> String {
    serde_json::to_string(&st).unwrap_or_else(|_| "{}".to_string())
}

fn app_config_json(c: &crate::config::AppConfig) -> String {
    // Forma que la UI espera (`GET /api/config`, `POST` devuelve lo mismo):
    // solo las secciones editables; `ubatch`/`device`/`llama_port`/`http_port`
    // son de solo lectura y no se aceptan en el POST.
    serde_json::json!({
        "engine": {
            "idle_timeout_secs": c.engine.idle_timeout_secs,
            "threads": c.engine.threads,
            "priority": c.engine.priority,
            "speculation_enabled": c.engine.speculation.as_ref().is_some_and(|s| s.enabled),
            "batch": c.engine.batch,
            "ubatch": 512,
            "device": c.engine.device,
            "llama_port": c.engine.llama_port,
            "http_port": c.engine.http_port,
        },
        "generation": {
            "temperature": c.generation.temperature,
            "top_p": c.generation.top_p,
            "max_tokens": c.generation.max_tokens,
            "seed": c.generation.seed,
        },
        "notifications": {
            "enabled": c.notifications.enabled,
            "on_ready": c.notifications.on_ready,
            "on_failure": c.notifications.on_failure,
            "on_autostop": c.notifications.on_autostop,
        },
    })
    .to_string()
}

fn settings_json(cfg: &crate::config::AppConfig, config_path: &std::path::Path, http_port: u16) -> String {
    let body = serde_json::json!({
        "last": cfg.last,
        "config_path": config_path.to_string_lossy(),
        "http_port": http_port,
        "llama_port": cfg.engine.llama_port,
    });
    body.to_string()
}
struct Metrics {
    gen_tps: Option<f64>,
    prompt_tps: Option<f64>,
    tokens_pred: Option<f64>,
    prompt_tot: Option<f64>,
    cached_tot: Option<f64>,
    requests_processing: Option<f64>,
    draft_tot: Option<f64>,
    accepted_tot: Option<f64>,
}

fn parse_metrics(text: &str) -> Metrics {
    let mut m = Metrics {
        gen_tps: None,
        prompt_tps: None,
        tokens_pred: None,
        prompt_tot: None,
        cached_tot: None,
        requests_processing: None,
        draft_tot: None,
        accepted_tot: None,
    };
    for line in text.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let (key, val) = match line.rsplit_once(' ') {
            Some(kv) => kv,
            None => continue,
        };
        let key = key.split('{').next().unwrap_or(key).trim();
        let val: f64 = match val.trim().parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        macro_rules! set {
            ($field:ident, $name:literal) => {
                if key.starts_with($name) && m.$field.is_none() {
                    m.$field = Some(val);
                }
            };
        }
        set!(gen_tps, "llamacpp:predicted_tokens_seconds");
        set!(prompt_tps, "llamacpp:prompt_tokens_seconds");
        set!(tokens_pred, "llamacpp:tokens_predicted_total");
        set!(prompt_tot, "llamacpp:prompt_tokens_total");
        set!(cached_tot, "llamacpp:prompt_tokens_cached_total");
        set!(requests_processing, "llamacpp:requests_processing");
        set!(draft_tot, "llamacpp:spec_decode_num_draft_tokens_total");
        set!(accepted_tot, "llamacpp:spec_decode_num_accepted_tokens_total");
    }
    m
}

/// Cargar un recurso estático (`localmind.ico`, `localmind.png`).
/// `None` → el handler responde 404 `{"error":"not_found"}`.
fn load_asset(base_dir: &std::path::Path, name: &str) -> Option<Vec<u8>> {
    std::fs::read(base_dir.join(name)).ok()
}

/// Normaliza `reasoning_effort` antes de reenviar al motor (última línea de
/// defensa del gateway: los tres proxys pasan por aquí).
/// La plantilla Qwen solo acepta `low`/`medium`/`xhigh` y responde 500 ante
/// cualquier otro valor (medido: `"max"` → error de chat-template).
/// - `minimal` → `low`; `high`/`max` → `xhigh` (insensible a mayúsculas).
/// - `low`/`medium`/`xhigh` se conservan (en minúsculas).
/// - Cualquier otro string (incluido `off`: la plantilla también lo rechaza)
///   o un valor no-string se ELIMINA: el motor usa su default en vez de dar 500.
/// Los lanzadores aceptan `--thinking off|low|medium|high|max` (omitido =>
/// `low`, porque el razonamiento domina el primer token) y sus
/// `thinkingLevelMap` marcan `max` como `null` (nivel no soportado lado
/// cliente), pero si un `max` crudo llega al gateway, aquí se convierte a
/// `xhigh` en vez de tumbar el motor.
fn sanitize_payload(body_bytes: Vec<u8>) -> Vec<u8> {
    if let Ok(mut json_val) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
        if let Some(obj) = json_val.as_object_mut() {
            if obj.contains_key("reasoning_effort") {
                match obj
                    .get("reasoning_effort")
                    .and_then(|v| v.as_str())
                    .and_then(normalize_reasoning_effort)
                {
                    Some(mapped) => {
                        obj.insert("reasoning_effort".to_string(), serde_json::Value::String(mapped));
                    }
                    None => {
                        obj.remove("reasoning_effort");
                    }
                }
            }
        }
        serde_json::to_vec(&json_val).unwrap_or(body_bytes)
    } else {
        body_bytes
    }
}

/// Mapea un `reasoning_effort` al vocabulario de la plantilla Qwen
/// (`low`/`medium`/`xhigh`); `None` = desconocido → el llamador elimina el campo.
fn normalize_reasoning_effort(v: &str) -> Option<String> {
    if v.eq_ignore_ascii_case("minimal") || v.eq_ignore_ascii_case("low") {
        Some("low".to_string())
    } else if v.eq_ignore_ascii_case("high")
        || v.eq_ignore_ascii_case("max")
        || v.eq_ignore_ascii_case("xhigh")
    {
        Some("xhigh".to_string())
    } else if v.eq_ignore_ascii_case("medium") {
        Some("medium".to_string())
    } else {
        None
    }
}

/// Reescribir el `model` pedido al id servido por el motor (LM-PXY-4/5, D-2).
/// Nunca se rechaza: los agentes pueden pedir cualquier alias.
fn rewrite_model_to_served(body_bytes: &[u8], served: &str) -> Vec<u8> {
    if served.is_empty() {
        return body_bytes.to_vec();
    }
    if let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(body_bytes) {
        if let Some(obj) = v.as_object_mut() {
            if obj.get("model").and_then(|m| m.as_str()).is_some() {
                obj.insert("model".to_string(), serde_json::Value::String(served.to_string()));
                return serde_json::to_vec(&v).unwrap_or_else(|_| body_bytes.to_vec());
            }
        }
    }
    body_bytes.to_vec()
}

/// Id servido por el motor para las superficies HTTP (`/v1/models` y el
/// rewrite del proxy). Delega en `agents::served_model_id`, la MISMA función
/// que escriben los lanzadores en la config de cada agente: si divergieran,
/// el agente anunciaría un modelo y el proxy respondería por otro.
///
/// El fallback (motor sin modelo) mantiene el alias estable para que
/// `/v1/models` SIEMPRE conteste y el proxy siga teniendo un id al que
/// reescribir aunque el motor esté apagado (D-2).
fn served_model_id(st: &ServerStatus, cfg: &ConfigStore) -> String {
    crate::agents::served_model_id(st).unwrap_or_else(|| {
        cfg.get()
            .engine
            .aliases
            .first()
            .cloned()
            .unwrap_or_else(|| "localmind".to_string())
    })
}

/// Cuerpo `GET /v1/models` (forma OpenAI): id servido + alias estables.
fn models_list_json(st: &ServerStatus, cfg: &ConfigStore) -> String {
    let served = served_model_id(st, cfg);
    let mut ids = vec![served.clone()];
    for a in &cfg.get().engine.aliases {
        if !ids.iter().any(|x| x == a) {
            ids.push(a.clone());
        }
    }
    let data: Vec<serde_json::Value> = ids
        .iter()
        .map(|id| {
            serde_json::json!({
                "id": id,
                "object": "model",
                "owned_by": "localmind",
            })
        })
        .collect();
    serde_json::json!({"object": "list", "data": data}).to_string()
}

impl HttpServer {
    pub fn start(
        process_mgr: Arc<ProcessManager>,
        config: Arc<ConfigStore>,
        base_dir: PathBuf,
    ) -> Result<Self, String> {
        // Clave local: crearla en el primer arranque para que las rutas
        // protegidas y la cookie de la WebView funcionen desde el inicio.
        let gateway_key = crate::auth::load_or_create_key();
        let preferred = config.get().engine.http_port;
        let mut server = None;
        let mut port = preferred;

        for p in preferred..preferred + 10 {
            if let Ok(s) = Server::http(("127.0.0.1", p)) {
                server = Some(s);
                port = p;
                break;
            }
        }

        let server = server.ok_or_else(|| {
            format!(
                "No se pudo iniciar el servidor HTTP en el rango {}-{}",
                preferred,
                preferred + 10
            )
        })?;
        let server = Arc::new(server);
        let srv_clone = Arc::clone(&server);
        // Puerto ligado de verdad: los CLIs hablan con el GATEWAY (no con el
        // motor) para pasar por clave/aliasing/usage. Se propaga a cada
        // request junto con la clave (ambos se clonan por request).
        let bound_port = port;
        thread::spawn(move || {
            for req in srv_clone.incoming_requests() {
                let url = req.url().to_string();
                let method = req.method().to_string();
                let mgr = Arc::clone(&process_mgr);
                let cfg = Arc::clone(&config);
                let base = base_dir.clone();
                let key = gateway_key.clone();

                thread::spawn(move || {
                    handle_request(req, method, url, mgr, cfg, base, key, bound_port);
                });
            }
        });

        Ok(Self { port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

/// Rutas públicas (sin clave): UI + iconos. Todo lo demás exige clave.
fn is_public_path(method: &str, url: &str) -> bool {
    if method == "OPTIONS" {
        return true;
    }
    if method != "GET" {
        return false;
    }
    let path = url.split('?').next().unwrap_or(url);
    matches!(path, "/" | "/index.html" | "/localmind.ico" | "/localmind.png")
}

fn handle_request(
    mut req: tiny_http::Request,
    method: String,
    url: String,
    mgr: Arc<ProcessManager>,
    cfg: Arc<ConfigStore>,
    base_dir: PathBuf,
    gateway_key: String,
    http_port: u16,
) {
    // Origen de esta petición para CORS estricto (P29): solo los orígenes
    // loopback reciben `Access-Control-Allow-Origin` reflejado.
    let origin = crate::meta::request_origin(req.headers());
    let origin_ref = origin.as_deref();
    let fixed = cors_fixed_headers();

    if method == "OPTIONS" {
        let mut resp = Response::empty(200);
        if let Some(o) = cors_origin_header(origin_ref) {
            resp.add_header(o);
        }
        resp.add_header(fixed[0].clone());
        resp.add_header(fixed[1].clone());
        let _ = req.respond(resp);
        return;
    }

    if method == "GET" && (url == "/" || url == "/index.html") {
        let ui = base_dir.join("ui.html");
        let html = if ui.exists() {
            std::fs::read_to_string(&ui)
                .unwrap_or_else(|_| include_str!("../ui_fallback.html").to_string())
        } else {
            include_str!("../ui_fallback.html").to_string()
        };
        let ct = Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap();
        let Some(cookie) = session_cookie_header(&gateway_key) else {
            let _ = req.respond(json_response_for_origin(500, r#"{"error":"clave de sesión inválida"}"#.into(), origin_ref));
            return;
        };
        let mut resp = Response::from_string(html);
        resp.add_header(ct);
        resp.add_header(cookie);
        add_cors_for(&mut resp, origin_ref);
        let _ = req.respond(resp);
        return;
    }

    if method == "GET" && url == "/localmind.ico" {
        let bytes = match load_asset(&base_dir, "localmind.ico") {
            Some(b) => b,
            None => {
                let _ = req.respond(json_response_for_origin(404, "{\"error\":\"not_found\"}".into(), origin_ref));
                return;
            }
        };
        let ct = Header::from_bytes(&b"Content-Type"[..], &b"image/x-icon"[..]).unwrap();
        let Some(cookie) = session_cookie_header(&gateway_key) else {
            let _ = req.respond(json_response_for_origin(500, r#"{"error":"clave de sesión inválida"}"#.into(), origin_ref));
            return;
        };
        let mut resp = Response::from_data(bytes);
        resp.add_header(ct);
        resp.add_header(cookie);
        add_cors_for(&mut resp, origin_ref);
        let _ = req.respond(resp);
        return;
    }

    if method == "GET" && url == "/localmind.png" {
        let bytes = match load_asset(&base_dir, "localmind.png") {
            Some(b) => b,
            None => {
                let _ = req.respond(json_response_for_origin(404, "{\"error\":\"not_found\"}".into(), origin_ref));
                return;
            }
        };
        let ct = Header::from_bytes(&b"Content-Type"[..], &b"image/png"[..]).unwrap();
        let Some(cookie) = session_cookie_header(&gateway_key) else {
            let _ = req.respond(json_response_for_origin(500, r#"{"error":"clave de sesión inválida"}"#.into(), origin_ref));
            return;
        };
        let mut resp = Response::from_data(bytes);
        resp.add_header(ct);
        resp.add_header(cookie);
        add_cors_for(&mut resp, origin_ref);
        let _ = req.respond(resp);
        return;
    }
    // Clave local (D-7): `/v1/*` y `/api/*` exigen Bearer, x-api-key o cookie.
    if !is_public_path(&method, &url) && !crate::auth::is_authorized(req.headers(), &gateway_key) {
        let _ = req.respond(unauthorized_json_for_origin(origin_ref));
        return;
    }

    if method == "GET" && url == "/api/models" {
        // Listado utilizable: raíz + un nivel (`models/<sub>/x.gguf`), sin
        // `.cache`/`.verified`. `filename` sigue siendo el fichero (lo que la
        // UI usa como `option.value` y `/api/start` como `model`); `path`
        // absoluta se conserva; se añade `rel` (relativa a `models/`).
        let models_dir = mgr.models_dir().to_path_buf();
        let list: Vec<serde_json::Value> = crate::models::list_gguf_files(&models_dir)
            .iter()
            .map(|(name, path, bytes)| {
                let size_gb = (*bytes as f64) / (1024.0 * 1024.0 * 1024.0);
                let rel = path
                    .strip_prefix(&models_dir)
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| name.clone());
                let v = serde_json::json!({
                    "filename": name,
                    "name": name.trim_end_matches(".gguf"),
                    "size_gb": (size_gb * 100.0).round() / 100.0,
                    "path": path.to_string_lossy(),
                    "rel": rel,
                });
                crate::models::enrich_model_entry(&models_dir, v, *bytes)
            })
            .collect();
        let json = serde_json::to_string(&list).unwrap_or_else(|_| "[]".to_string());
        let _ = req.respond(json_response_for_origin(200, json, origin_ref));
        return;
    }

    // Metadatos del gateway: versión de la app + build/commit del motor.
    // La ruta nunca falla por el motor: ante cualquier problema los campos
    // del motor salen `null` y la respuesta sigue siendo 200.
    if method == "GET" && url == "/api/version" {
        let info = crate::meta::engine_info(&base_dir);
        let body = serde_json::json!({
            "app": env!("CARGO_PKG_VERSION"),
            "engine_build": info.build,
            "engine_commit": info.commit,
            "engine_path": info.path,
        });
        let _ = req.respond(json_response_for_origin(200, body.to_string(), origin_ref));
        return;
    }

    // Export del anillo de logs en memoria (mismo contenido que /api/logs).
    if method == "GET" && url == "/api/logs/export" {
        let lines = mgr.get_recent_logs();
        let mut resp = Response::from_string(lines.join("\n")).with_status_code(StatusCode(200));
        let ct = Header::from_bytes(&b"Content-Type"[..], &b"text/plain; charset=utf-8"[..]).unwrap();
        resp.add_header(ct);
        let stamp = crate::meta::utc_stamp_now();
        let cd = Header::from_bytes(
            &b"Content-Disposition"[..],
            format!("attachment; filename=\"localmind-logs-{}.txt\"", stamp).as_bytes(),
        )
        .unwrap();
        resp.add_header(cd);
        add_cors_for(&mut resp, origin_ref);
        let _ = req.respond(resp);
        return;
    }

    if method == "GET" && url == "/api/hardware" {
        let hw = mgr.detect_hardware();
        let json = serde_json::to_string(&hw).unwrap_or_else(|_| "{}".to_string());
        let _ = req.respond(json_response_for_origin(200, json, origin_ref));
        return;
    }

    if method == "GET" && url == "/api/profiles" {
        let profiles = crate::profiles::get_hardware_profiles(&cfg.get());
        let json = serde_json::to_string(&profiles).unwrap_or_else(|_| "[]".to_string());
        let _ = req.respond(json_response_for_origin(200, json, origin_ref));
        return;
    }

    if method == "GET" && url == "/api/profiles/export" {
        let profiles = crate::profiles::get_hardware_profiles(&cfg.get());
        let json = serde_json::to_string_pretty(&profiles).unwrap_or_else(|_| "[]".to_string());
        let mut resp = json_response_for_origin(200, json, origin_ref);
        let cd = Header::from_bytes(&b"Content-Disposition"[..], &b"attachment; filename=\"localmind_profiles.json\""[..]).unwrap();
        resp.add_header(cd);
        let _ = req.respond(resp);
        return;
    }

    if method == "POST" && url == "/api/profiles/import" {
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        match parse_profiles_import(&body) {
            Ok(imported) => {
                cfg.update(|c| {
                    c.profiles = imported;
                });
                let _ = cfg.save();
                let _ = req.respond(json_response_for_origin(200, r#"{"status":"ok","message":"Perfiles actualizados"}"#.into(), origin_ref));
            }
            Err(e) => {
                let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": e }).to_string(), origin_ref));
            }
        }
        return;
    }

    // Alta/edición de un perfil (`upsert`): valida id/contexto/cache/flags y
    // devuelve la lista completa en la forma de `GET /api/profiles`. Nada se
    // persiste si algún campo es inválido.
    if method == "POST" && url == "/api/profiles/save" {
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let v: serde_json::Value = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(e) => {
                let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": format!("JSON inválido: {}", e) }).to_string(), origin_ref));
                return;
            }
        };
        let bad = |campo: &str| {
            serde_json::json!({ "error": format!("campo inválido: {}", campo) }).to_string()
        };
        let m = match v.as_object() {
            Some(m) => m,
            None => {
                let _ = req.respond(json_response_for_origin(400, bad("perfil"), origin_ref));
                return;
            }
        };
        let id = match m.get("id").and_then(|j| j.as_str()) {
            Some(id) if crate::config::profile_id_ok(id) => id.to_string(),
            _ => {
                let _ = req.respond(json_response_for_origin(400, bad("id"), origin_ref));
                return;
            }
        };
        let name = match m.get("name").and_then(|j| j.as_str()) {
            Some(s) if !s.trim().is_empty() && s.len() <= 120 => s.to_string(),
            _ => {
                let _ = req.respond(json_response_for_origin(400, bad("name"), origin_ref));
                return;
            }
        };
        let description = match m.get("description") {
            None => String::new(),
            Some(j) => match j.as_str() {
                Some(s) if s.len() <= 2000 => s.to_string(),
                _ => {
                    let _ = req.respond(json_response_for_origin(400, bad("description"), origin_ref));
                    return;
                }
            },
        };
        let context = match m.get("context").and_then(|j| j.as_u64()).and_then(|n| usize::try_from(n).ok()) {
            Some(n) if crate::config::profile_context_ok(n) => n,
            _ => {
                let _ = req.respond(json_response_for_origin(400, bad("context"), origin_ref));
                return;
            }
        };
        let cache_ram = match m.get("cache_ram") {
            None => 0,
            Some(j) => match j.as_u64().and_then(|n| usize::try_from(n).ok()) {
                Some(n) if crate::config::profile_cache_ram_ok(n) => n,
                _ => {
                    let _ = req.respond(json_response_for_origin(400, bad("cache_ram"), origin_ref));
                    return;
                }
            },
        };
        let extra_flags: Vec<String> = match m.get("extra_flags") {
            None => Vec::new(),
            Some(j) => match j.as_array() {
                Some(arr) => {
                    let mut out = Vec::with_capacity(arr.len());
                    for f in arr {
                        match f.as_str() {
                            Some(s) => out.push(s.to_string()),
                            None => {
                                let _ = req.respond(json_response_for_origin(400, bad("extra_flags"), origin_ref));
                                return;
                            }
                        }
                    }
                    out
                }
                None => {
                    let _ = req.respond(json_response_for_origin(400, bad("extra_flags"), origin_ref));
                    return;
                }
            },
        };
        if !crate::config::profile_flags_ok(&extra_flags) {
            let _ = req.respond(json_response_for_origin(400, bad("extra_flags"), origin_ref));
            return;
        }
        let mut next = cfg.get();
        let prof = crate::config::HardwareProfile {
            id: id.clone(),
            name,
            description,
            context,
            cache_ram,
            extra_flags,
        };
        match next.profiles.iter().position(|p| p.id == id) {
            Some(i) => next.profiles[i] = prof,
            None => next.profiles.push(prof),
        }
        let prev = cfg.get();
        cfg.update(|c| *c = next.clone());
        if let Err(e) = cfg.save() {
            cfg.update(|c| *c = prev);
            let _ = req.respond(json_response_for_origin(500, serde_json::json!({ "error": format!("no se pudo guardar el perfil: {}", e) }).to_string(), origin_ref));
            return;
        }
        mgr.log(&format!("[LocalMind] Perfil '{}' guardado.", id));
        let list = crate::profiles::get_hardware_profiles(&next);
        let json = serde_json::to_string(&list).unwrap_or_else(|_| "[]".to_string());
        let _ = req.respond(json_response_for_origin(200, json, origin_ref));
        return;
    }

    // Borrado de un perfil: 400 si no existe, si es el último o si está en
    // uso (perfil del motor en curso o `last.profile` de arranque).
    if method == "POST" && url == "/api/profiles/delete" {
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let id = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("id").and_then(|j| j.as_str().map(str::to_string)))
            .unwrap_or_default();
        if !crate::config::profile_id_ok(&id) {
            let _ = req.respond(json_response_for_origin(400, r#"{"error":"campo inválido: id"}"#.into(), origin_ref));
            return;
        }
        let cur = cfg.get();
        if !cur.profiles.iter().any(|p| p.id == id) {
            let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": format!("perfil desconocido: '{}'", id) }).to_string(), origin_ref));
            return;
        }
        if cur.profiles.len() <= 1 {
            let _ = req.respond(json_response_for_origin(400, r#"{"error":"no se puede eliminar el último perfil"}"#.into(), origin_ref));
            return;
        }
        // En uso = motor en curso con ese perfil, o arranque futuro (`last`).
        let live = mgr.get_status().profile;
        let boot = cur.last.profile.clone().unwrap_or_default();
        if live == id || boot == id {
            let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": format!("el perfil '{}' está en uso", id) }).to_string(), origin_ref));
            return;
        }
        let mut next = cur.clone();
        next.profiles.retain(|p| p.id != id);
        cfg.update(|c| *c = next.clone());
        if let Err(e) = cfg.save() {
            cfg.update(|c| *c = cur);
            let _ = req.respond(json_response_for_origin(500, serde_json::json!({ "error": format!("no se pudo eliminar el perfil: {}", e) }).to_string(), origin_ref));
            return;
        }
        mgr.log(&format!("[LocalMind] Perfil '{}' eliminado.", id));
        let list = crate::profiles::get_hardware_profiles(&next);
        let json = serde_json::to_string(&list).unwrap_or_else(|_| "[]".to_string());
        let _ = req.respond(json_response_for_origin(200, json, origin_ref));
        return;
    }

    if method == "POST" && url == "/api/import_model" {
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let source_path_str = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("path").and_then(|p| p.as_str().map(str::to_string)));

        if let Some(src) = source_path_str {
            let p = std::path::Path::new(&src);
            match mgr.import_model_from_path(p) {
                Ok(filename) => {
                    let _ = req.respond(json_response_for_origin(200, serde_json::json!({ "status": "ok", "filename": filename }).to_string(), origin.as_deref()));
                }
                Err(e) => {
                    let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": e }).to_string(), origin.as_deref()));
                }
            }
        } else {
            let _ = req.respond(json_response_for_origin(400, r#"{"error":"Falta el parámetro 'path'"}"#.into(), origin.as_deref()));
        }
        return;
    }

    if method == "POST" && url == "/api/open_models_dir" {
        let p = mgr.models_dir();
        let _ = std::process::Command::new("explorer.exe")
            .arg(p)
            .spawn();
        let _ = req.respond(json_response_for_origin(200, r#"{"status":"ok"}"#.into(), origin.as_deref()));
        return;
    }
    if method == "GET" && url == "/api/status" {
        let _ = req.respond(json_response_for_origin(200, status_json(mgr.get_status()), origin.as_deref()));
        return;
    }

    if method == "GET" && url == "/api/settings" {
        let c = cfg.get();
        let json = settings_json(&c, cfg.path(), c.engine.http_port);
        let _ = req.respond(json_response_for_origin(200, json, origin.as_deref()));
        return;
    }

    // Ajustes de la UI (Settings): `GET` devuelve solo las secciones
    // editables; el POST acepta un cuerpo parcial, rechaza claves
    // desconocidas, valida rangos y persiste de forma atómica. Los cambios
    // que afectan al motor (engine) se aplican en el PRÓXIMO arranque: el
    // motor en curso nunca se reconfigura ni se reinicia desde aquí.
    if method == "GET" && url == "/api/config" {
        let c = cfg.get();
        let _ = req.respond(json_response_for_origin(200, app_config_json(&c), origin.as_deref()));
        return;
    }

    if method == "POST" && url == "/api/config" {
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let v: serde_json::Value = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(e) => {
                let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": format!("JSON inválido: {}", e) }).to_string(), origin.as_deref()));
                return;
            }
        };
        if !v.is_object() {
            let _ = req.respond(json_response_for_origin(400, r#"{"error":"cuerpo inválido: se esperaba un objeto"}"#.into(), origin.as_deref()));
            return;
        }
        let obj = v.as_object().map(|o| o.clone()).unwrap_or_default();
        // Claves de primer nivel: solo las tres secciones editables.
        for k in obj.keys() {
            if k != "engine" && k != "generation" && k != "notifications" {
                let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": format!("clave desconocida: {}", k) }).to_string(), origin.as_deref()));
                return;
            }
        }
        let bad = |campo: &str| {
            serde_json::json!({ "error": format!("campo inválido: {}", campo) }).to_string()
        };
        let mut next = cfg.get();
        // --- engine (parcial; `ubatch`/`device`/`llama_port`/`http_port` son
        // --- de solo lectura y se rechazan si vienen en el cuerpo).
        if let Some(eng) = obj.get("engine") {
            let em = match eng.as_object() {
                Some(m) => m,
                None => {
                    let _ = req.respond(json_response_for_origin(400, bad("engine"), origin.as_deref()));
                    return;
                }
            };
            for k in em.keys() {
                if k != "idle_timeout_secs" && k != "threads" && k != "priority" && k != "speculation" {
                    let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": format!("campo inválido: engine.{} (solo lectura o desconocido)", k) }).to_string(), origin.as_deref()));
                    return;
                }
            }
            if let Some(j) = em.get("idle_timeout_secs") {
                let n = j.as_u64();
                match n {
                    Some(n) if crate::config::engine_idle_timeout_ok(n) => next.engine.idle_timeout_secs = n,
                    _ => {
                        let _ = req.respond(json_response_for_origin(400, bad("engine.idle_timeout_secs"), origin.as_deref()));
                        return;
                    }
                }
            }
            if let Some(j) = em.get("threads") {
                if j.is_null() {
                    next.engine.threads = None;
                } else if let Some(n) = j.as_u64().and_then(|n| usize::try_from(n).ok()) {
                    if !crate::config::engine_threads_ok(n) {
                        let _ = req.respond(json_response_for_origin(400, bad("engine.threads"), origin.as_deref()));
                        return;
                    }
                    next.engine.threads = Some(n);
                } else {
                    let _ = req.respond(json_response_for_origin(400, bad("engine.threads"), origin.as_deref()));
                    return;
                }
            }
            if let Some(j) = em.get("priority") {
                match j.as_str() {
                    Some(s) if crate::config::engine_priority_ok(s) => next.engine.priority = s.to_string(),
                    _ => {
                        let _ = req.respond(json_response_for_origin(400, bad("engine.priority"), origin.as_deref()));
                        return;
                    }
                }
            }
            if let Some(j) = em.get("speculation") {
                match j.as_object() {
                    Some(sm) => {
                        for k in sm.keys() {
                            if k != "enabled" {
                                let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": format!("campo inválido: engine.speculation.{} (solo se acepta enabled)", k) }).to_string(), origin.as_deref()));
                                return;
                            }
                        }
                        match sm.get("enabled") {
                            Some(b) if b.is_boolean() => {
                                let en = b.as_bool().unwrap_or(true);
                                if next.engine.speculation.is_none() {
                                    next.engine.speculation = Some(crate::config::SpeculationConfig::default());
                                }
                                if let Some(s) = next.engine.speculation.as_mut() {
                                    s.enabled = en;
                                }
                            }
                            _ => {
                                let _ = req.respond(json_response_for_origin(400, bad("engine.speculation.enabled"), origin.as_deref()));
                                return;
                            }
                        }
                    }
                    _ => {
                        let _ = req.respond(json_response_for_origin(400, bad("engine.speculation"), origin.as_deref()));
                        return;
                    }
                }
            }
        }
        // --- generation (parcial; rangos de `config.rs`).
        if let Some(gen) = obj.get("generation") {
            let gm = match gen.as_object() {
                Some(m) => m,
                None => {
                    let _ = req.respond(json_response_for_origin(400, bad("generation"), origin.as_deref()));
                    return;
                }
            };
            for k in gm.keys() {
                if k != "temperature" && k != "top_p" && k != "max_tokens" && k != "seed" {
                    let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": format!("clave desconocida: generation.{}", k) }).to_string(), origin.as_deref()));
                    return;
                }
            }
            if let Some(j) = gm.get("temperature") {
                match j.as_f64() {
                    Some(t) if crate::config::gen_temperature_ok(t) => next.generation.temperature = t,
                    _ => {
                        let _ = req.respond(json_response_for_origin(400, bad("generation.temperature"), origin.as_deref()));
                        return;
                    }
                }
            }
            if let Some(j) = gm.get("top_p") {
                match j.as_f64() {
                    Some(p) if crate::config::gen_top_p_ok(p) => next.generation.top_p = p,
                    _ => {
                        let _ = req.respond(json_response_for_origin(400, bad("generation.top_p"), origin.as_deref()));
                        return;
                    }
                }
            }
            if let Some(j) = gm.get("max_tokens") {
                match j.as_u64().and_then(|n| usize::try_from(n).ok()) {
                    Some(m) if crate::config::gen_max_tokens_ok(m) => next.generation.max_tokens = m,
                    _ => {
                        let _ = req.respond(json_response_for_origin(400, bad("generation.max_tokens"), origin.as_deref()));
                        return;
                    }
                }
            }
            if let Some(j) = gm.get("seed") {
                match j.as_i64() {
                    Some(s) if crate::config::gen_seed_ok(s) => next.generation.seed = s,
                    _ => {
                        let _ = req.respond(json_response_for_origin(400, bad("generation.seed"), origin.as_deref()));
                        return;
                    }
                }
            }
        }
        // --- notifications (parcial; todo booleanos).
        if let Some(not) = obj.get("notifications") {
            let nm = match not.as_object() {
                Some(m) => m,
                None => {
                    let _ = req.respond(json_response_for_origin(400, bad("notifications"), origin.as_deref()));
                    return;
                }
            };
            for k in nm.keys() {
                if k != "enabled" && k != "on_ready" && k != "on_failure" && k != "on_autostop" {
                    let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": format!("clave desconocida: notifications.{}", k) }).to_string(), origin.as_deref()));
                    return;
                }
            }
            let flag = |key: &str, slot: &mut bool| -> bool {
                match nm.get(key) {
                    None => true,
                    Some(j) if j.is_boolean() => {
                        *slot = j.as_bool().unwrap_or(*slot);
                        true
                    }
                    _ => false,
                }
            };
            if !flag("enabled", &mut next.notifications.enabled) {
                let _ = req.respond(json_response_for_origin(400, bad("notifications.enabled"), origin.as_deref()));
                return;
            }
            if !flag("on_ready", &mut next.notifications.on_ready) {
                let _ = req.respond(json_response_for_origin(400, bad("notifications.on_ready"), origin.as_deref()));
                return;
            }
            if !flag("on_failure", &mut next.notifications.on_failure) {
                let _ = req.respond(json_response_for_origin(400, bad("notifications.on_failure"), origin.as_deref()));
                return;
            }
            if !flag("on_autostop", &mut next.notifications.on_autostop) {
                let _ = req.respond(json_response_for_origin(400, bad("notifications.on_autostop"), origin.as_deref()));
                return;
            }
        }
        // Todo válido: persistencia atómica (memoria + TOML). Si el disco
        // falla, se revierte la memoria para no mentir a la UI.
        let prev = cfg.get();
        cfg.update(|c| *c = next.clone());
        if let Err(e) = cfg.save() {
            cfg.update(|c| *c = prev);
            let _ = req.respond(json_response_for_origin(500, serde_json::json!({ "error": format!("no se pudo guardar la configuración: {}", e) }).to_string(), origin.as_deref()));
            return;
        }
        mgr.log("[LocalMind] Configuración actualizada (aplica al próximo arranque del motor).");
        let _ = req.respond(json_response_for_origin(200, app_config_json(&next), origin.as_deref()));
        return;
    }

    if method == "GET" && url == "/api/metrics" {
        let st = mgr.get_status();
        if (st.status != "running" && !st.is_healthy) || st.port == 0 {
            let _ = req.respond(json_response_for_origin(502, r#"{"error":"engine_down"}"#.into(), origin.as_deref()));
            return;
        }
        match ureq::get(&format!("http://127.0.0.1:{}/metrics", st.port))
            // El motor exige `--api-key` (D-45): sin la cabecera responde 401.
            .set("Authorization", &crate::auth::bearer(&gateway_key))
            .timeout(Duration::from_secs(2))
            .call()
        {
            Ok(resp) => {
                let mut body = String::new();
                if resp.into_reader().read_to_string(&mut body).is_ok() {
                    let m = parse_metrics(&body);
                    let json = serde_json::json!({
                        "gen_tps": m.gen_tps,
                        "prompt_tps": m.prompt_tps,
                        "tokens_predicted": m.tokens_pred,
                        "prompt_tokens_total": m.prompt_tot,
                        "prompt_tokens_cached": m.cached_tot,
                        "requests_processing": m.requests_processing,
                        "spec_draft_tokens": m.draft_tot,
                        "spec_accepted_tokens": m.accepted_tot,
                    });
                    let _ = req.respond(json_response_for_origin(200, json.to_string(), origin.as_deref()));
                } else {
                    let _ = req.respond(json_response_for_origin(502, r#"{"error":"engine_read_failed"}"#.into(), origin.as_deref()));
                }
            }
            Err(_) => {
                let _ = req.respond(json_response_for_origin(502, r#"{"error":"engine_down"}"#.into(), origin.as_deref()));
            }
        }
        return;
    }

    if method == "POST" && url == "/api/start" {
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let start_req: StartRequest = match parse_start_body(&body) {
            Ok(r) => r,
            Err(e) => {
                let _ = req.respond(
                    json_response_for_origin(400, serde_json::json!({ "error": e }).to_string(), origin.as_deref()),
                );
                return;
            }
        };
        match mgr.start(start_req) {
            Ok(pid) => {
                let _ = req.respond(json_response_for_origin(200, format!(r#"{{"status":"starting","pid":{}}}"#, pid), origin.as_deref()));
            }
            Err(e) => {
                let _ = req.respond(
                    json_response_for_origin(400, serde_json::json!({ "error": e }).to_string(), origin.as_deref()),
                );
            }
        }
        return;
    }

    if method == "POST" && url == "/api/stop" {
        mgr.stop();
        let _ = req.respond(json_response_for_origin(200, r#"{"status":"stopped"}"#.into(), origin.as_deref()));
        return;
    }

    if method == "GET" && url == "/api/logs" {
        let json = serde_json::to_string(&mgr.get_recent_logs()).unwrap_or_else(|_| "[]".to_string());
        let _ = req.respond(json_response_for_origin(200, json, origin.as_deref()));
        return;
    }

    // SSE: history bufferizada con seq + live. Reader bloqueante sobre mpsc.
    if method == "GET" && url == "/api/events" {
        let rx = mgr.subscribe_logs();
        let initial: Vec<LogEvent> = mgr.get_log_events();
        let reader: Box<dyn Read + Send + Sync> = Box::new(sse::EventReader::new(rx, initial));
        let mut resp =
            Response::new(StatusCode(200), Vec::new(), reader, None, None);
        for h in sse_headers() {
            resp.add_header(h);
        }
        if let Some(o) = cors_origin_header(origin_ref) {
            resp.add_header(o);
        }
        let _ = req.respond(resp);
        return;
    }

    if method == "POST" && url == "/api/select_folder" {
        let folder = rfd::FileDialog::new()
            .set_title("Selecciona la carpeta de tu proyecto")
            .pick_folder();

        if let Some(path) = folder {
            let p_str = path.to_string_lossy().to_string();
            let _ = req.respond(json_response_for_origin(200, serde_json::json!({ "status": "ok", "path": p_str }).to_string(), origin.as_deref()));
        } else {
            let _ = req.respond(json_response_for_origin(200, serde_json::json!({ "status": "cancelled" }).to_string(), origin.as_deref()));
        }
        return;
    }
    // Lanzador genérico: un selector en la UI (`pi`/`omp`/`web`/`deepseek`).
    // `/api/launch_omp` y `/api/launch_pi` siguen como reenvíos finos hasta
    // que la nueva UI esté en producción; entonces podrán eliminarse.
    if method == "POST" && url == "/api/launch" {
        handle_launch_generic(req, &mgr, &gateway_key, http_port, origin.clone());
        return;
    }
    if method == "POST" && url == "/api/launch_omp" {
        handle_launch_compat(req, &mgr, "omp", &gateway_key, http_port, origin.clone());
        return;
    }

    if method == "POST" && url == "/api/launch_pi" {
        handle_launch_compat(req, &mgr, "pi", &gateway_key, http_port, origin.clone());
        return;
    }

    if method == "GET" && url == "/api/agents" {
        let _ = req.respond(json_response_for_origin(200, crate::launcher::agents_list_json(), origin.as_deref()));
        return;
    }

    if method == "POST" && url == "/api/open_browser" {
        let st = mgr.get_status();
        let port = st.port;
        let _ = std::process::Command::new("cmd.exe")
            .args(["/c", &format!("start http://127.0.0.1:{}", port)])
            .spawn();
        mgr.log(&format!(
            "[LocalMind] Abierto navegador web en http://127.0.0.1:{}",
            port
        ));
        let _ = req.respond(json_response_for_origin(200, r#"{"status":"ok"}"#.into(), origin.as_deref()));
        return;
    }

    if method == "POST" && url == "/api/models/download" {
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let v: serde_json::Value = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(e) => {
                let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": format!("JSON inválido: {}", e) }).to_string(), origin.as_deref()));
                return;
            }
        };
        let repo = v.get("repo").and_then(|r| r.as_str()).unwrap_or("").trim().to_string();
        if let Err(e) = crate::models::check_repo(&repo) {
            let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": e }).to_string(), origin.as_deref()));
            return;
        }
        let revision = v.get("revision").and_then(|r| r.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        if let Some(r) = revision.as_deref() {
            if let Err(e) = crate::models::check_revision(r) {
                let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": e }).to_string(), origin.as_deref()));
                return;
            }
        }
        let only_file = v.get("files").and_then(|f| f.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        if let Some(f) = only_file.as_deref() {
            if let Err(e) = crate::models::check_rel_path(f) {
                let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": e }).to_string(), origin.as_deref()));
                return;
            }
        }
        let models_dir = mgr.models_dir().to_path_buf();
        let logger: std::sync::Arc<dyn Fn(String) + Send + Sync> = {
            let m = Arc::clone(&mgr);
            std::sync::Arc::new(move |line: String| m.log(&line))
        };
        match crate::models::start_job(models_dir, logger, repo, revision, only_file) {
            Ok(()) => {
                let _ = req.respond(json_response_for_origin(200, crate::models::job_snapshot().json(), origin.as_deref()));
            }
            Err(e) => {
                let _ = req.respond(json_response_for_origin(409, serde_json::json!({ "error": e }).to_string(), origin.as_deref()));
            }
        }
        return;
    }

    if method == "POST" && url == "/api/models/download/cancel" {
        let _ = req.as_reader().read_to_string(&mut String::new());
        let _ = req.respond(json_response_for_origin(200, crate::models::job_cancel().json(), origin.as_deref()));
        return;
    }

    if method == "POST" && url == "/api/models/import_pick" {
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let overwrite = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("overwrite").and_then(|o| o.as_bool()))
            .unwrap_or(false);
        // El diálogo nativo bloquea: hilo propio, nunca el hilo HTTP; espera
        // acotada (120 s) para no dejar la conexión colgada. Al vencer se
        // responde `cancelled/timeout` y el hilo del diálogo queda inofensivo
        // (su resultado ya nadie lo lee).
        let models_dir = mgr.models_dir().to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let picked = rfd::FileDialog::new()
                .set_title("Elige uno o varios .gguf para importar a models/")
                .add_filter("GGUF", &["gguf"])
                .pick_files();
            let _ = tx.send(picked);
        });
        let picked = rx.recv_timeout(Duration::from_secs(crate::models::IMPORT_PICK_TIMEOUT_SECS));
        let picked = match picked {
            Ok(p) => p,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let _ = req.respond(json_response_for_origin(200, r#"{"status":"cancelled","reason":"timeout"}"#.into(), origin.as_deref()));
                return;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => None,
        };
        let body_out = match picked {
            None => r#"{"status":"cancelled"}"#.to_string(),
            Some(files) if files.is_empty() => r#"{"status":"cancelled"}"#.to_string(),
            Some(files) => match crate::models::copy_picked(&models_dir, &files, overwrite) {
                Ok(names) => {
                    mgr.log(&format!("[LocalMind] Modelos importados: {}", names.join(", ")));
                    serde_json::json!({ "status": "ok", "files": names }).to_string()
                }
                Err(e) => serde_json::json!({ "error": e }).to_string(),
            },
        };
        let code = if body_out.contains("\"error\"") { 400 } else { 200 };
        let _ = req.respond(json_response_for_origin(code, body_out, origin.as_deref()));
        return;
    }

    // Telemetría visible: resumen incremental + últimas líneas (LM-TEL-3).
    if method == "GET" && url == "/api/usage" {
        let _ = req.respond(json_response_for_origin(200, crate::usage::usage_summary(), origin.as_deref()));
        return;
    }

    if method == "GET" && (url == "/api/usage/raw" || url.starts_with("/api/usage/raw?")) {
        let limit = url
            .split('?')
            .nth(1)
            .unwrap_or("")
            .split('&')
            .filter_map(|kv| {
                let mut it = kv.splitn(2, '=');
                match (it.next(), it.next()) {
                    (Some(k), Some(val)) if k == "limit" => val.parse::<usize>().ok(),
                    _ => None,
                }
            })
            .next()
            .unwrap_or(100)
            .min(500);
        let _ = req.respond(json_response_for_origin(200, crate::usage::usage_raw(limit), origin.as_deref()));
        return;
    }

    // Superficie OpenAI: lista de modelos (id servido + alias estables).
    if method == "GET" && (url == "/v1/models" || url == "/v1/models/") {
        let st = mgr.get_status();
        let json = models_list_json(&st, &cfg);
        let _ = req.respond(json_response_for_origin(200, json, origin.as_deref()));
        return;
    }

    // Proxy SSE LIVE: reader de ureq directo en el body de tiny_http.
    if method == "POST" && url == "/v1/chat/completions" {
        handle_chat_completions(req, &mgr, &cfg, origin.clone());
        return;
    }

    // Claude Code: Anthropic Messages.
    if method == "POST" && url == "/v1/messages" {
        handle_anthropic_messages(req, &mgr, &cfg, origin.clone());
        return;
    }

    // Codex: OpenAI Responses.
    if method == "POST" && url == "/v1/responses" {
        handle_responses(req, &mgr, &cfg, origin.clone());
        return;
    }

    let _ = req.respond(json_response_for_origin(404, "{\"error\":\"not_found\"}".into(), origin.as_deref()));
}
/// Acción del navegador (misma que `/api/open_browser`): abre la interfaz
/// web externa en el puerto del motor. El llamador responde `{"status":"ok"}`.
fn open_browser_action(mgr: &Arc<ProcessManager>) {
    let st = mgr.get_status();
    let port = st.port;
    let _ = std::process::Command::new("cmd.exe")
        .args(["/c", &format!("start http://127.0.0.1:{}", port)])
        .spawn();
    mgr.log(&format!(
        "[LocalMind] Abierto navegador web en http://127.0.0.1:{}",
        port
    ));
}

/// Lanzador genérico (`POST /api/launch {"agent":"pi"|"omp"|"opencode"|"web"|"deepseek"}`).
///
/// - `pi`/`omp`: escribe `%APPDATA%\LocalMind\agents\<agent>\` con el puerto,
///   contexto y clave VIVOS, lanza con `PI_CODING_AGENT_DIR` por Windows
///   Terminal → `cmd start`. Sin motor vivo: 409 con el mensaje existente.
///   `effort` (`off`|`low`|`medium`|`high`|`max`): nivel `--thinking` del CLI;
///   omitido => `low` (la fase de razonamiento domina la latencia al primer
///   token, así que el default arranca en el modo más rápido).
/// - `opencode`: `OPENCODE_CONFIG_CONTENT` por env + XDG aislados (409 sin motor).
/// - `web`: abre la interfaz web externa (misma acción que
///   `/api/open_browser`: el navegador en el puerto del motor).
/// - `deepseek`: harness `dsh --profile headless` (501 sin binario).
fn handle_launch_generic(
    mut req: tiny_http::Request,
    mgr: &Arc<ProcessManager>,
    gateway_key: &str,
    http_port: u16,
    origin: Option<String>,
) {
    let mut body = String::new();
    let _ = req.as_reader().read_to_string(&mut body);
    let body_val = serde_json::from_str::<serde_json::Value>(&body).unwrap_or(serde_json::Value::Null);
    let agent_raw = body_val.get("agent").and_then(|a| a.as_str()).unwrap_or("").to_string();
    let req_dir = body_val.get("cwd").and_then(|d| d.as_str().map(str::to_string));
    let req_effort = body_val.get("effort").and_then(|e| e.as_str().map(str::to_string));
    // `task` lo usan `deepseek`/`opencode` (one-shot); para pi/omp/web se
    // ignora. Sin `task` se abre el harness en modo interactivo.
    let req_task = body_val.get("task").and_then(|t| t.as_str().map(str::to_string));
    let id = match crate::launcher::parse_agent_id(&agent_raw) {
        Some(id) => id,
        None => {
            let _ = req.respond(json_response_for_origin(
                400,
                serde_json::json!({ "error": format!("Agente desconocido: '{}'. Use pi, omp, opencode, web o deepseek.", agent_raw) }).to_string(),
                origin.as_deref(),
            ));
            return;
        }
    };
    // `web` = misma acción que `/api/open_browser` (navegador al motor).
    if id == crate::launcher::AgentId::Web {
        open_browser_action(&mgr);
        let _ = req.respond(json_response_for_origin(200, r#"{"status":"ok"}"#.into(), origin.as_deref()));
        return;
    }

    // `deepseek` = harness oficial `@deepseek-ai/dsh` (verificado en
    // `docs/agents/deepseek-harness.md`). Sin binario instalado: 501 para
    // mostrarlo deshabilitado en la UI. Con `task` = one-shot headless, sin
    // ella = terminal interactiva con el modelo local.
    // `web` = navegador al motor; `deepseek` sin binario = 501 (el mapping
    // vive en `launcher`, testeado). `task` solo la usa `deepseek`.
    match crate::launcher::launch_action_for(id, crate::launcher::deepseek_installed()) {
        crate::launcher::LaunchAction::Web => {
            open_browser_action(&mgr);
            let _ = req.respond(json_response_for_origin(200, r#"{"status":"ok"}"#.into(), origin.as_deref()));
            return;
        }
        crate::launcher::LaunchAction::PendingHarness => {
            let _ = req.respond(json_response_for_origin(
                501,
                serde_json::json!({ "error": crate::launcher::deepseek_not_configured_msg() }).to_string(),
                origin.as_deref(),
            ));
            return;
        }
        crate::launcher::LaunchAction::Cli(_) => {}
    }
    // Guardia de motor con el estado del MANAGER QUE NOS LLAMÓ (mismo `mgr`):
    // `launch_cli`/`launch_opencode` revalidan con su propio `mgr`, así que el
    // 409 siempre refleja el estado real del motor de esta instancia.
    // `deepseek`/`opencode` = harnesses reales; `pi`/`omp` = núcleo CLI.
    if id == crate::launcher::AgentId::DeepSeek {
        launch_deepseek(req, &mgr, req_dir.as_deref(), req_task.as_deref(), http_port, origin.as_deref());
        return;
    }
    if id == crate::launcher::AgentId::OpenCode {
        launch_opencode(&mgr, req, req_dir.as_deref(), req_task.as_deref(), http_port, origin.as_deref());
        return;
    }
    // `pi`/`omp` llegan aquí (el `match` ya resolvió `web` y el 501).
    launch_cli(req, &mgr, http_port, id, req_dir.as_deref(), req_effort.as_deref(), gateway_key, origin.as_deref());
}

/// Reenvíos finos de `/api/launch_omp` y `/api/launch_pi` (UI actual).
/// Se eliminarán cuando la nueva UI con selector único esté en producción.
fn handle_launch_compat(
    mut req: tiny_http::Request,
    mgr: &Arc<ProcessManager>,
    agent: &str,
    gateway_key: &str,
    http_port: u16,
    origin: Option<String>,
) {
    let mut body = String::new();
    let _ = req.as_reader().read_to_string(&mut body);
    let body_val = serde_json::from_str::<serde_json::Value>(&body).unwrap_or(serde_json::Value::Null);
    let req_dir = body_val.get("cwd").and_then(|d| d.as_str().map(str::to_string));
    let req_effort = body_val.get("effort").and_then(|e| e.as_str().map(str::to_string));
    let id = match crate::launcher::parse_agent_id(agent) {
        Some(id) => id,
        None => {
            let _ = req.respond(json_response_for_origin(
                400,
                serde_json::json!({ "error": format!("Agente desconocido: '{}'.", agent) }).to_string(),
                origin.as_deref(),
            ));
            return;
        }
    };
    launch_cli(req, &mgr, http_port, id, req_dir.as_deref(), req_effort.as_deref(), gateway_key, origin.as_deref());
}

/// Helpers de spawn de terminal con verificación + fallback (`wt` → `cmd`).
/// `wt.exe` en `%LOCALAPPDATA%\Microsoft\WindowsApps` es un alias de ejecución
/// de 0 bytes: `p.exists()` lo selecciona aunque el spawn falle desde nuestro
/// contexto. Por eso la política es "try + verify" (`spawn_terminal`): si el
/// spawn de WT devuelve `Err`, se reintenta con `cmd start` y se registra qué
/// rama se usó; si ambas fallan el endpoint responde 500 en español.
fn wt_command(cmd_str: &str) -> Option<std::process::Command> {
    let wt_path = std::env::var("LOCALAPPDATA")
        .map(|l| PathBuf::from(l).join("Microsoft/WindowsApps/wt.exe"))
        .ok()?;
    if !wt_path.exists() {
        return None;
    }
    let mut c = std::process::Command::new(wt_path);
    c.args(["-w", "0", "new-tab", "cmd.exe", "/k", cmd_str]);
    Some(c)
}

fn cmd_start_command(cmd_str: &str) -> std::process::Command {
    let mut c = std::process::Command::new("cmd.exe");
    c.args(["/c", &format!("start cmd.exe /k \"{}\"", cmd_str)]);
    c
}

/// Los CLIs hablan con el GATEWAY ligado (`http://127.0.0.1:<http_port>/v1`,
/// propagado desde `HttpServer::start` por cada request) para pasar por
/// clave/aliasing/usage. El puerto del motor (`st.port`) es solo interno.

/// Núcleo CLI compartido (`pi`/`omp`; `opencode`/`deepseek` tienen lanzador
/// propio): 409 sin motor, dir privado con estado vivo, spawn verificado.
fn launch_cli(
    req: tiny_http::Request,
    mgr: &Arc<ProcessManager>,
    http_port: u16,
    id: crate::launcher::AgentId,
    req_dir: Option<&str>,
    req_effort: Option<&str>,
    gateway_key: &str,
    origin: Option<&str>,
) {
    let st = mgr.get_status();
    if !crate::agents::engine_live(&st) {
        let err = crate::agents::engine_down_error(&st);
        let _ = req.respond(json_response_for_origin(409, err, origin));
        return;
    }
    let context = st.context;
    // `http_port` es el puerto del GATEWAY ligado (propagado desde `start`);
    // el del motor (`st.port`) es solo interno (health/slots/metrics).
    let agent = match id {
        crate::launcher::AgentId::Pi => "pi",
        crate::launcher::AgentId::Omp => "omp",
        crate::launcher::AgentId::DeepSeek => "deepseek",
        crate::launcher::AgentId::OpenCode => "opencode",
        crate::launcher::AgentId::Web => {
            let _ = req.respond(json_response_for_origin(
                400,
                serde_json::json!({ "error": "El agente web no usa este lanzamiento." }).to_string(),
                origin,
            ));
            return;
        }
    };
    let cli_model = crate::launcher::cli_model(id);
    let label = crate::launcher::agent_label(id);
    let agent_dir = match crate::agents::write_agent_dir(agent, http_port, context, gateway_key) {
        Ok(d) => d,
        Err(e) => {
            let _ = req.respond(json_response_for_origin(500, serde_json::json!({ "error": e }).to_string(), origin));
            return;
        }
    };

    // omp se niega a arrancar en el home sin --allow-home.
    let mut allow_home = String::new();
    if let Some(dir) = req_dir {
        if crate::agents::is_home_dir(dir) {
            allow_home = " --allow-home".to_string();
        }
    }

    let cd_prefix = match req_dir {
        Some(dir) if !dir.trim().is_empty() => format!("cd /d \"{}\" && ", dir),
        _ => String::new(),
    };
    let agent_path = agent_dir.to_string_lossy().to_string();
    let inner_cmd = crate::launcher::cli_inner_cmd(
        id,
        &cd_prefix,
        http_port,
        gateway_key,
        &agent_path,
        req_effort,
        &allow_home,
    );

    // Terminal del sistema con verificación + fallback (`wt` → `cmd start`):
    // el spawn de WT puede devolver `Err` (alias reparse de 0 bytes) y antes
    // se tragaba con `let _`. Ahora se registra el pid o el error, y si ambas
    // ramas fallan el endpoint responde 500 en vez de un falso 200.
    let cmd_str = inner_cmd.clone();
    let wt = wt_command(&cmd_str);
    let (ok, branch, detail) = crate::launcher::spawn_terminal(wt, &|| cmd_start_command(&cmd_str), req_dir);
    let via = match branch {
        crate::launcher::SpawnBranch::Wt => "wt",
        crate::launcher::SpawnBranch::CmdStart => "cmd-start",
    };
    if !ok {
        mgr.log(&format!(
            "[LocalMind] ERROR al lanzar Terminal {} en '{}': {} (vía {}).",
            label,
            req_dir.unwrap_or("directorio default"),
            detail, via
        ));
        let _ = req.respond(json_response_for_origin(500, serde_json::json!({ "error": format!("No se pudo abrir la terminal ({}): {}", via, detail) }).to_string(), origin));
        return;
    }
    mgr.log(&format!(
        "[LocalMind] Terminal {} lanzada ({} {}) en '{}' (gateway :{}, ctx: {}) conectada a {}.",
        label, via, detail,
        req_dir.unwrap_or("directorio default"),
        http_port, context, cli_model
    ));
    let _ = req.respond(json_response_for_origin(200, format!(r#"{{"status":"ok","model":"{}","port":{},"context":{}}}"#, cli_model, http_port, context), origin));
}

/// Lanzador DeepSeek (`dsh --profile headless ["<tarea>"]`, verificado en
/// `docs/agents/deepseek-harness.md`): env aislado (`DSH_HOME` propio,
/// telemetría off, `workspace-write`) y clave por `set /p` (nunca en argv).
/// Sin `task` = terminal interactiva con el modelo local; con `task` =
/// one-shot headless. La clave se lee aquí mismo de `gateway.key`.
fn launch_deepseek(
    req: tiny_http::Request,
    mgr: &Arc<ProcessManager>,
    req_dir: Option<&str>,
    req_task: Option<&str>,
    http_port: u16,
    origin: Option<&str>,
) {
    let inner = crate::launcher::deepseek_inner_cmd(req_task);
    let cd_prefix = match req_dir {
        Some(dir) if !dir.trim().is_empty() => format!("cd /d \"{}\" && ", dir),
        _ => String::new(),
    };
    // Patch Cordis generado con valores VIVOS (gateway + contexto): el fichero
    // estático con 17860 hardcodeado queda obsoleto en cuanto el HTTP liga
    // otro puerto. Solo se reescribe si cambia (mtime estable).
    let st = mgr.get_status();
    let home = crate::launcher::deepseek_home().unwrap_or_else(|| crate::agents::agent_dir("deepseek"));
    let patch = crate::launcher::deepseek_profile_patch(http_port, st.context, "localmind");
    match crate::launcher::write_deepseek_patch(&home, &patch) {
        Ok(_) => {}
        Err(e) => {
            let _ = req.respond(json_response_for_origin(500, serde_json::json!({ "error": e }).to_string(), origin));
            return;
        }
    }
    let cmd_str = crate::launcher::deepseek_cmdline(&cd_prefix, &inner);
    let wt = wt_command(&cmd_str);
    let (ok, branch, detail) = crate::launcher::spawn_terminal(wt, &|| cmd_start_command(&cmd_str), req_dir);
    let via = match branch {
        crate::launcher::SpawnBranch::Wt => "wt",
        crate::launcher::SpawnBranch::CmdStart => "cmd-start",
    };
    let what = match req_task.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => format!("tarea '{}'", t.chars().take(60).collect::<String>()),
        None => "sesión interactiva".to_string(),
    };
    if !ok {
        mgr.log(&format!(
            "[LocalMind] ERROR al lanzar Terminal DeepSeek harness en '{}' ({}): {} (vía {}).",
            req_dir.unwrap_or("directorio default"),
            what, detail, via
        ));
        let _ = req.respond(json_response_for_origin(500, serde_json::json!({ "error": format!("No se pudo abrir la terminal ({}): {}", via, detail) }).to_string(), origin));
        return;
    }
    mgr.log(&format!(
        "[LocalMind] Terminal DeepSeek harness lanzada ({} {}) en '{}' ({}).",
        via, detail,
        req_dir.unwrap_or("directorio default"),
        what
    ));
    let _ = req.respond(json_response_for_origin(200, r#"{"status":"ok","agent":"deepseek"}"#.into(), origin));
}

/// Lanzador OpenCode (`opencode --model localmind/<id> [run "<tarea>"]`,
/// verificado v1.18.10): `OPENCODE_CONFIG_CONTENT` inyectado por env con el
/// puerto/contexto VIVOS (clave por `{env:LOCALMIND_API_KEY}`, nunca cruda) y
/// dirs XDG aislados en el dir privado (nunca `~/.config/opencode`). Sin
/// motor vivo: 409; sin binario: 501 en español. Spawn verificado + fallback.
fn launch_opencode(
    mgr: &Arc<ProcessManager>,
    req: tiny_http::Request,
    req_dir: Option<&str>,
    req_task: Option<&str>,
    http_port: u16,
    origin: Option<&str>,
) {
    if !crate::launcher::opencode_installed() {
        let _ = req.respond(json_response_for_origin(
            501,
            serde_json::json!({ "error": "El harness de OpenCode todavía no está configurado" }).to_string(),
            origin,
        ));
        return;
    }
    let st = mgr.get_status();
    if !crate::agents::engine_live(&st) {
        let err = crate::agents::engine_down_error(&st);
        let _ = req.respond(json_response_for_origin(409, err, origin));
        return;
    }
    // Alias corto para el payload (`qwen3.8-27b`); el flag lleva el prefijo.
    // baseURL = GATEWAY (contabilidad/aliasing), no el motor.
    // La config se escribe como FICHERO en el dir privado (no por env: el
    // `set "VAR=<json>"` de cmd.exe corrompía el JSON con `\"` literales y
    // opencode ignoraba el provider → `ProviderModelNotFoundError`).
    let full = crate::launcher::cli_model(crate::launcher::AgentId::OpenCode);
    let model_id = full.strip_prefix("localmind/").unwrap_or(full);
    let home = crate::agents::agent_dir("opencode");
    if let Err(e) = std::fs::create_dir_all(&home) {
        let _ = req.respond(json_response_for_origin(500, serde_json::json!({ "error": format!("No se pudo crear {}: {}", home.display(), e) }).to_string(), origin));
        return;
    }
    let gateway_key_live = crate::auth::load_or_create_key();
    let content = crate::launcher::opencode_config_json(http_port, st.context, model_id, Some(&gateway_key_live));
    match crate::launcher::write_opencode_config(&home, &content) {
        Ok(_) => {}
        Err(e) => {
            let _ = req.respond(json_response_for_origin(500, serde_json::json!({ "error": e }).to_string(), origin));
            return;
        }
    }
    let home_s = home.to_string_lossy().to_string();
    let key_file = std::env::var("APPDATA")
        .map(|a| PathBuf::from(a).join("LocalMind").join("gateway.key"))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let inner = crate::launcher::opencode_inner_cmd(model_id, req_task);
    let cd_prefix = match req_dir {
        Some(dir) if !dir.trim().is_empty() => format!("cd /d \"{}\" && ", dir),
        _ => String::new(),
    };
    let cmd_str = crate::launcher::opencode_cmdline(&cd_prefix, &home_s, &key_file, &inner);
    let wt = wt_command(&cmd_str);
    let (ok, branch, detail) = crate::launcher::spawn_terminal(wt, &|| cmd_start_command(&cmd_str), req_dir);
    let via = match branch {
        crate::launcher::SpawnBranch::Wt => "wt",
        crate::launcher::SpawnBranch::CmdStart => "cmd-start",
    };
    let what = match req_task.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => format!("tarea '{}'", t.chars().take(60).collect::<String>()),
        None => "sesión interactiva".to_string(),
    };
    if !ok {
        mgr.log(&format!(
            "[LocalMind] ERROR al lanzar Terminal OpenCode en '{}' ({}): {} (vía {}).",
            req_dir.unwrap_or("directorio default"),
            what, detail, via
        ));
        let _ = req.respond(json_response_for_origin(500, serde_json::json!({ "error": format!("No se pudo abrir la terminal ({}): {}", via, detail) }).to_string(), origin));
        return;
    }
    mgr.log(&format!(
        "[LocalMind] Terminal OpenCode lanzada ({} {}) en '{}' ({}) con modelo {} (gateway :{}, ctx: {}).",
        via, detail,
        req_dir.unwrap_or("directorio default"),
        what, full, http_port, st.context
    ));
    let _ = req.respond(json_response_for_origin(200, format!(r#"{{"status":"ok","agent":"opencode","model":"{}","port":{},"context":{}}}"#, full, http_port, st.context), origin));
}

/// `POST /v1/chat/completions`: alias rewriting + usage + registro.
fn handle_chat_completions(
    mut req: tiny_http::Request,
    mgr: &Arc<ProcessManager>,
    cfg: &Arc<ConfigStore>,
    origin: Option<String>,
) {
    let mut body_bytes = Vec::new();
    let _ = req.as_reader().read_to_end(&mut body_bytes);
    mgr.touch_activity();

    let st = mgr.get_status();
    if st.port == 0 {
        let _ = req.respond(json_response_for_origin(502, r#"{"error":"engine_down"}"#.into(), origin.as_deref()));
        return;
    }
    let served = served_model_id(&st, cfg);
    let req_model = serde_json::from_slice::<serde_json::Value>(&body_bytes)
        .ok()
        .and_then(|v| v.get("model").and_then(|m| m.as_str().map(str::to_string)))
        .unwrap_or_else(|| served.clone());
    let stream_req = serde_json::from_slice::<serde_json::Value>(&body_bytes)
        .ok()
        .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
        .unwrap_or(false);

    let mut payload_bytes = sanitize_payload(body_bytes);
    payload_bytes = rewrite_model_to_served(&payload_bytes, &served);
    payload_bytes = crate::usage::ensure_stream_usage(&payload_bytes);

    let t0 = Instant::now();
    match ureq::post(&format!("http://127.0.0.1:{}/v1/chat/completions", st.port))
        .set("Content-Type", "application/json")
        // El motor exige `--api-key` (D-45): sin la cabecera responde 401 y el
        // proxy devolvería el error del motor al cliente en vez de traducirlo.
        .set("Authorization", &crate::auth::bearer(crate::auth::gateway_key()))
        .send_bytes(&payload_bytes)
    {
        Ok(resp) => {
            if stream_req {
                // Streaming incremental (LM-PXY-1/LM-UI-5): el tee cede cada
                // chunk al cliente en cuanto llega y registra el uso al EOF
                // desde la cola de 64 KiB.
                let reader = proxy::TeeLogReader::new(
                    resp.into_reader(),
                    "chat.completions".to_string(),
                    req_model,
                    t0,
                );
                let ct = Header::from_bytes(&b"Content-Type"[..], &b"text/event-stream"[..]).unwrap();
                let mut hdrs = vec![ct];
                if let Some(o) = cors_origin_header(origin.as_deref()) {
                    hdrs.push(o);
                }
                hdrs.extend(cors_fixed_headers());
                let proxy_resp = Response::new(StatusCode(200), hdrs, reader, None, None);
                let _ = req.respond(proxy_resp);
            } else {
                let mut raw = String::new();
                let mut reader = resp.into_reader();
                if reader.read_to_string(&mut raw).is_err() {
                    let _ = req.respond(json_response_for_origin(502, r#"{"error":"Error al contactar motor"}"#.into(), origin.as_deref()));
                    return;
                }
                let (p, c) = crate::usage::extract_usage_from_sse(&raw);
                crate::usage::log_usage(
                    "chat.completions",
                    &req_model,
                    p,
                    c,
                    t0.elapsed().as_millis() as u64,
                    false,
                );
                // Reenviar el JSON tal cual con Content-Type json.
                let _ = req.respond(json_response_for_origin(200, raw, origin.as_deref()));
            }
        }
        Err(ureq::Error::Status(code, resp)) => {
            let mut body = String::new();
            let _ = resp.into_reader().read_to_string(&mut body);
            let _ = req.respond(json_response_for_origin(code, body, origin.as_deref()));
        }
        Err(_) => {
            let _ = req.respond(json_response_for_origin(502, r#"{"error":"Error al contactar motor"}"#.into(), origin.as_deref()));
        }
    }
}

/// `POST /v1/messages` (Anthropic, lo que necesita Claude Code).
fn handle_anthropic_messages(
    mut req: tiny_http::Request,
    mgr: &Arc<ProcessManager>,
    cfg: &Arc<ConfigStore>,
    origin: Option<String>,
) {
    // La clave local ya se validó arriba (Bearer, x-api-key o cookie); la
    // versión Anthropic (`anthropic-version`) se acepta en cualquier valor.

    let mut body_bytes = Vec::new();
    let _ = req.as_reader().read_to_end(&mut body_bytes);
    mgr.touch_activity();
    let body: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(e) => {
            let (code, text) = crate::translate::anthropic_error(
                400,
                "invalid_request_error",
                &format!("JSON inválido: {}", e),
            );
            let _ = req.respond(json_response_for_origin(code, text, origin.as_deref()));
            return;
        }
    };
    let req_model = body.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string();
    let stream = body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

    let st = mgr.get_status();
    if st.port == 0 {
        let (code, text) = crate::translate::anthropic_error(
            502,
            "api_error",
            "Motor apagado (engine_down)",
        );
        let _ = req.respond(json_response_for_origin(code, text, origin.as_deref()));
        return;
    }
    let served = served_model_id(&st, cfg);
    let open = match crate::translate::anthropic_to_openai(&body, &served) {
        Ok(o) => o,
        Err(e) => {
            let (code, text) =
                crate::translate::anthropic_error(400, "invalid_request_error", &e);
            let _ = req.respond(json_response_for_origin(code, text, origin.as_deref()));
            return;
        }
    };
    let mut payload = sanitize_payload(serde_json::to_vec(&open).unwrap_or_default());
    payload = crate::usage::ensure_stream_usage(&payload);

    let t0 = Instant::now();
    let model_for_log = if req_model.is_empty() { served.clone() } else { req_model.clone() };
    match ureq::post(&format!("http://127.0.0.1:{}/v1/chat/completions", st.port))
        .set("Content-Type", "application/json")
        // El motor exige `--api-key` (D-45): sin la cabecera responde 401 y el
        // proxy devolvería el error del motor al cliente en vez de traducirlo.
        .set("Authorization", &crate::auth::bearer(crate::auth::gateway_key()))
        .send_bytes(&payload)
    {
        Ok(resp) => {
            if stream {
                // Traducción incremental evento por evento: el primer delta sale
                // al cliente sin esperar al `[DONE]`; el cierre con usage sale al EOF.
                let msg_id = format!("msg_{}", unique_suffix());
                let mut state = proxy::TranslateState::Anthropic(
                    crate::translate::AnthropicSse::new(&model_for_log, msg_id),
                );
                let preamble = {
                    match &mut state {
                        proxy::TranslateState::Anthropic(s) => s.preamble(),
                        proxy::TranslateState::Responses(_) => String::new(),
                    }
                };
                let reader = proxy::TranslateLogReader::new(
                    resp.into_reader(),
                    state,
                    |st, ev| match st {
                        proxy::TranslateState::Anthropic(s) => s.feed_event(ev),
                        proxy::TranslateState::Responses(_) => String::new(),
                    },
                    |st| match st {
                        proxy::TranslateState::Anthropic(s) => s.finish(),
                        proxy::TranslateState::Responses(_) => String::new(),
                    },
                    "messages".to_string(),
                    model_for_log,
                    t0,
                    preamble,
                );
                let ct = Header::from_bytes(&b"Content-Type"[..], &b"text/event-stream"[..]).unwrap();
                let mut hdrs = vec![ct];
                if let Some(o) = cors_origin_header(origin.as_deref()) {
                    hdrs.push(o);
                }
                hdrs.extend(cors_fixed_headers());
                let r = Response::new(StatusCode(200), hdrs, reader, None, None);
                let _ = req.respond(r);
            } else {
                let mut raw = String::new();
                let mut reader = resp.into_reader();
                let _ = reader.read_to_string(&mut raw);
                let (pt, ct_) = crate::usage::extract_usage_from_sse(&raw);
                crate::usage::log_usage("messages", &model_for_log, pt, ct_, t0.elapsed().as_millis() as u64, false);
                // El motor en no-streaming devuelve JSON chat/completions.
                let chat: serde_json::Value = raw
                    .lines()
                    .filter_map(|l| {
                        let t = l.trim();
                        let p = t.strip_prefix("data:").map(|s| s.trim()).unwrap_or(t);
                        if p.is_empty() || p == "[DONE]" || !p.starts_with('{') {
                            return None;
                        }
                        serde_json::from_str(p).ok()
                    })
                    .last()
                    .or_else(|| serde_json::from_str(&raw).ok())
                    .unwrap_or(serde_json::Value::Null);
                let env = crate::translate::openai_to_anthropic(&chat, &model_for_log);
                let _ = req.respond(json_response_for_origin(200, env.to_string(), origin.as_deref()));
            }
        }
        Err(ureq::Error::Status(code, resp)) => {
            let mut body = String::new();
            let _ = resp.into_reader().read_to_string(&mut body);
            let (c, text) = crate::translate::responses_error(code, &snippet(&body));
            let _ = req.respond(json_response_for_origin(c, text, origin.as_deref()));
        }
        Err(_) => {
            let _ = req.respond(json_response_for_origin(502, r#"{"error":"Error al contactar motor"}"#.into(), origin.as_deref()));
        }
    }
}

/// `POST /v1/responses` (Responses, lo que necesita Codex).
fn handle_responses(
    mut req: tiny_http::Request,
    mgr: &Arc<ProcessManager>,
    cfg: &Arc<ConfigStore>,
    origin: Option<String>,
) {
    let mut body_bytes = Vec::new();
    let _ = req.as_reader().read_to_end(&mut body_bytes);
    mgr.touch_activity();
    let body: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(e) => {
            let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": format!("JSON inválido: {}", e) }).to_string(), origin.as_deref()));
            return;
        }
    };
    let req_model = body.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string();

    let st = mgr.get_status();
    if st.port == 0 {
        let _ = req.respond(json_response_for_origin(502, r#"{"error":"engine_down"}"#.into(), origin.as_deref()));
        return;
    }
    let served = served_model_id(&st, cfg);
    let (open, stream) = match crate::translate::responses_to_openai(&body, &served) {
        Ok(v) => v,
        Err(e) => {
            let _ = req.respond(json_response_for_origin(400, serde_json::json!({ "error": e }).to_string(), origin.as_deref()));
            return;
        }
    };
    let mut payload = sanitize_payload(serde_json::to_vec(&open).unwrap_or_default());
    payload = crate::usage::ensure_stream_usage(&payload);

    let t0 = Instant::now();
    let model_for_log = if req_model.is_empty() { served.clone() } else { req_model.clone() };
    match ureq::post(&format!("http://127.0.0.1:{}/v1/chat/completions", st.port))
        .set("Content-Type", "application/json")
        // El motor exige `--api-key` (D-45): sin la cabecera responde 401 y el
        // proxy devolvería el error del motor al cliente en vez de traducirlo.
        .set("Authorization", &crate::auth::bearer(crate::auth::gateway_key()))
        .send_bytes(&payload)
    {
        Ok(resp) => {
            if stream {
                let resp_id = format!("resp_{}", unique_suffix());
                let mut state = proxy::TranslateState::Responses(
                    crate::translate::ResponsesSse::new(&model_for_log, resp_id),
                );
                let preamble = match &mut state {
                    proxy::TranslateState::Responses(s) => s.preamble(),
                    proxy::TranslateState::Anthropic(_) => String::new(),
                };
                let reader = proxy::TranslateLogReader::new(
                    resp.into_reader(),
                    state,
                    |st, ev| match st {
                        proxy::TranslateState::Responses(s) => s.feed_event(ev),
                        proxy::TranslateState::Anthropic(_) => String::new(),
                    },
                    |st| match st {
                        proxy::TranslateState::Responses(s) => s.finish(),
                        proxy::TranslateState::Anthropic(_) => String::new(),
                    },
                    "responses".to_string(),
                    model_for_log,
                    t0,
                    preamble,
                );
                let ct = Header::from_bytes(&b"Content-Type"[..], &b"text/event-stream"[..]).unwrap();
                let mut hdrs = vec![ct];
                if let Some(o) = cors_origin_header(origin.as_deref()) {
                    hdrs.push(o);
                }
                hdrs.extend(cors_fixed_headers());
                let r = Response::new(StatusCode(200), hdrs, reader, None, None);
                let _ = req.respond(r);
            } else {
                let mut raw = String::new();
                let mut reader = resp.into_reader();
                let _ = reader.read_to_string(&mut raw);
                let (p, c) = crate::usage::extract_usage_from_sse(&raw);
                crate::usage::log_usage("responses", &model_for_log, p, c, t0.elapsed().as_millis() as u64, false);
                let chat: serde_json::Value = raw
                    .lines()
                    .filter_map(|l| {
                        let t = l.trim();
                        let p = t.strip_prefix("data:").map(|s| s.trim()).unwrap_or(t);
                        if p.is_empty() || p == "[DONE]" || !p.starts_with('{') {
                            return None;
                        }
                        serde_json::from_str(p).ok()
                    })
                    .last()
                    .or_else(|| serde_json::from_str(&raw).ok())
                    .unwrap_or(serde_json::Value::Null);
                let env = crate::translate::openai_to_responses(&chat, &model_for_log);
                let _ = req.respond(json_response_for_origin(200, env.to_string(), origin.as_deref()));
            }
        }
        Err(ureq::Error::Status(code, resp)) => {
            let mut body = String::new();
            let _ = resp.into_reader().read_to_string(&mut body);
            let (c, text) = crate::translate::responses_error(code, &snippet(&body));
            let _ = req.respond(json_response_for_origin(c, text, origin.as_deref()));
        }
        Err(_) => {
            let _ = req.respond(json_response_for_origin(502, r#"{"error":"Error al contactar motor"}"#.into(), origin.as_deref()));
        }
    }
}

fn unique_suffix() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let rs = RandomState::new();
    let mut h = rs.build_hasher();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    h.write_u64(nanos ^ (std::process::id() as u64));
    format!("{:016x}", h.finish())
}

fn snippet(s: &str) -> String {
    const MAX: usize = 500;
    if s.len() <= MAX {
        return s.to_string();
    }
    format!("{}…", &s[..MAX])
}

fn sse_headers() -> Vec<Header> {
    let ct = Header::from_bytes(&b"Content-Type"[..], &b"text/event-stream"[..]).unwrap();
    let cc = Header::from_bytes(&b"Cache-Control"[..], &b"no-cache"[..]).unwrap();
    let cn = Header::from_bytes(&b"Connection"[..], &b"keep-alive"[..]).unwrap();
    let mut hs = vec![ct, cc, cn];
    hs.extend(cors_fixed_headers());
    hs
}

/// Lectores de proxy incremental (LM-PXY-1): envuelven el reader del motor y
/// ceden cada chunk al cliente en cuanto llega, sin acumular el body entero.
/// Al llegar a EOF registran la línea de uso desde una cola acotada (64 KiB).
mod proxy {
    use super::*;

    /// Tee pasante para `POST /v1/chat/completions` en streaming: reenvía los
    /// bytes tal cual y guarda solo la cola para el usage final.
    pub struct TeeLogReader<R: Read + Send> {
        inner: R,
        buf: [u8; 8192],
        out: Vec<u8>,
        tail: Vec<u8>,
        done: bool,
        logged: bool,
        endpoint: String,
        model: String,
        t0: Instant,
    }
    impl<R: Read + Send> TeeLogReader<R> {

        pub fn new(inner: R, endpoint: String, model: String, t0: Instant) -> Self {
            Self {
                inner,
                buf: [0u8; 8192],
                out: Vec::new(),
                tail: Vec::new(),
                done: false,
                logged: false,
                endpoint,
                model,
                t0,
            }
        }

        fn log_once(&mut self) {
            if self.logged {
                return;
            }
            self.logged = true;
            let text = String::from_utf8_lossy(&self.tail).to_string();
            let (p, c) = crate::usage::extract_usage_from_sse(&text);
            crate::usage::log_usage(
                &self.endpoint,
                &self.model,
                p,
                c,
                self.t0.elapsed().as_millis() as u64,
                true,
            );
        }
    }

    impl<R: Read + Send> Read for TeeLogReader<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            loop {
                if !self.out.is_empty() {
                    let n = self.out.len().min(buf.len());
                    buf[..n].copy_from_slice(&self.out[..n]);
                    self.out.drain(..n);
                    return Ok(n);
                }
                if self.done {
                    self.log_once();
                    return Ok(0);
                }
                match self.inner.read(&mut self.buf) {
                    Ok(0) => {
                        self.done = true;
                    }
                    Ok(n) => {
                        crate::usage::push_tail(&mut self.tail, &self.buf[..n]);
                        self.out.extend_from_slice(&self.buf[..n]);
                    }
                    Err(e) => return Err(e),
                }
            }
        }
    }

    /// Traductor incremental SSE: parte el upstream por eventos (`\n\n`),
    /// traduce cada evento a cero o más eventos de salida y los cede ya;
    /// al EOF emite el cierre con el usage de la cola y registra el uso.
    /// `translate`: evento upstream → eventos listos. `finish`: cierre final.
    pub struct TranslateLogReader<R: Read + Send, F: Fn(&mut TranslateState, &str) -> String + Send, G: Fn(&mut TranslateState) -> String + Send> {
        inner: R,
        buf: [u8; 8192],
        pending: String,
        out: Vec<u8>,
        tail: Vec<u8>,
        done: bool,
        finished: bool,
        logged: bool,
        state: TranslateState,
        translate: F,
        finish: G,
        endpoint: String,
        model: String,
        t0: Instant,
    }

    /// Estado compartido del traductor: variante Anthropic o Responses.
    pub enum TranslateState {
        Anthropic(crate::translate::AnthropicSse),
        Responses(crate::translate::ResponsesSse),
    }

    impl<R: Read + Send, F: Fn(&mut TranslateState, &str) -> String + Send, G: Fn(&mut TranslateState) -> String + Send>
        TranslateLogReader<R, F, G>
    {
        pub fn new(
            inner: R,
            state: TranslateState,
            translate: F,
            finish: G,
            endpoint: String,
            model: String,
            t0: Instant,
            preamble: String,
        ) -> Self {
            Self {
                inner,
                buf: [0u8; 8192],
                pending: String::new(),
                out: preamble.into_bytes(),
                tail: Vec::new(),
                done: false,
                finished: false,
                logged: false,
                state,
                translate,
                finish,
                endpoint,
                model,
                t0,
            }
        }

        fn log_once(&mut self) {
            if self.logged {
                return;
            }
            self.logged = true;
            let text = String::from_utf8_lossy(&self.tail).to_string();
            let (p, c) = crate::usage::extract_usage_from_sse(&text);
            crate::usage::log_usage(
                &self.endpoint,
                &self.model,
                p,
                c,
                self.t0.elapsed().as_millis() as u64,
                true,
            );
        }

        /// Extraer eventos completos (`\n\n`) del pendiente y traducirlos ya.
        fn pump_events(&mut self) {
            while let Some(pos) = self.pending.find("\n\n") {
                let ev: String = self.pending[..pos].to_string();
                self.pending = self.pending[pos + 2..].to_string();
                let translated = (self.translate)(&mut self.state, &ev);
                self.out.extend_from_slice(translated.as_bytes());
            }
        }
    }

    impl<R: Read + Send, F: Fn(&mut TranslateState, &str) -> String + Send, G: Fn(&mut TranslateState) -> String + Send> Read
        for TranslateLogReader<R, F, G>
    {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            loop {
                if !self.out.is_empty() {
                    let n = self.out.len().min(buf.len());
                    buf[..n].copy_from_slice(&self.out[..n]);
                    self.out.drain(..n);
                    return Ok(n);
                }
                if self.done {
                    if !self.finished {
                        self.finished = true;
                        // Traducir el resto parcial (si trae un evento sin `\n\n`
                        // final) antes del cierre.
                        let rest = std::mem::take(&mut self.pending);
                        if !rest.trim().is_empty() {
                            let translated = (self.translate)(&mut self.state, &rest);
                            self.out.extend_from_slice(translated.as_bytes());
                        }
                        let closing = (self.finish)(&mut self.state);
                        self.out.extend_from_slice(closing.as_bytes());
                        continue;
                    }
                    self.log_once();
                    return Ok(0);
                }
                match self.inner.read(&mut self.buf) {
                    Ok(0) => {
                        self.done = true;
                    }
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&self.buf[..n]).to_string();
                        crate::usage::push_tail(&mut self.tail, &self.buf[..n]);
                        self.pending.push_str(&chunk);
                        self.pump_events();
                    }
                    Err(e) => return Err(e),
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Read;
        use std::sync::{Arc, Mutex};

        /// Upstream que cede la respuesta en 3 trozos y anota cuándo se le
        /// pide cada uno (para probar que el cliente recibe el trozo 1 antes
        /// de que se pida el trozo 2: incrementalidad real, sin timing).
        struct ChunkedUpstream {
            chunks: Vec<Vec<u8>>,
            next: usize,
            log: Arc<Mutex<Vec<usize>>>,
        }

        impl Read for ChunkedUpstream {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.next >= self.chunks.len() {
                    return Ok(0);
                }
                let i = self.next;
                self.log.lock().map(|mut g| g.push(100 + i)).unwrap_or(());
                let n = self.chunks[i].len().min(buf.len());
                buf[..n].copy_from_slice(&self.chunks[i][..n]);
                if n == self.chunks[i].len() {
                    self.next += 1;
                } else {
                    let rest = self.chunks[i][n..].to_vec();
                    self.chunks[i] = rest;
                }
                Ok(n)
            }
        }

        #[test]
        fn tee_incremental_y_cola_acotada() {
            // Higiene (auditoría): el tee registra en `usage.jsonl`; fijar un
            // scratch para no contaminar el registro real del dueño.
            let scratch = std::env::temp_dir().join(format!("lm-test-usage-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&scratch);
            unsafe { std::env::set_var("LOCALMIND_USAGE_PATH", scratch.join("usage.jsonl")) };
            let c1 = b"data: {\"choices\":[{\"delta\":{\"content\":\"Hola\"}}]}\n\n".to_vec();
            let c2 = b"data: {\"choices\":[{\"delta\":{\"content\":\" mundo\"}}]}\n\n".to_vec();
            let c3 = b"data: {\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":7}}\n\ndata: [DONE]\n\n".to_vec();
            let log = Arc::new(Mutex::new(Vec::new()));
            let upstream = ChunkedUpstream {
                chunks: vec![c1.clone(), c2.clone(), c3.clone()],
                next: 0,
                log: Arc::clone(&log),
            };
            let mut tee = TeeLogReader::new(
                upstream,
                "test.tee".to_string(),
                "test-model".to_string(),
                Instant::now(),
            );
            // Leer solo el primer trozo: el cliente lo tiene sin esperar al resto.
            let mut first = vec![0u8; c1.len()];
            let mut got = 0;
            while got < c1.len() {
                let n = tee.read(&mut first[got..]).unwrap();
                assert!(n > 0, "el tee debe ceder el trozo 1 de inmediato");
                got += n;
            }
            assert_eq!(&first, &c1);
            // Y el upstream solo fue pedido hasta el trozo 1 (índice 0).
            let seen: Vec<usize> = log.lock().map(|g| g.clone()).unwrap_or_default();
            assert!(
                seen.iter().all(|&x| x <= 100),
                "trozo 2 pedido antes de entregar el 1: {:?}",
                seen
            );
            // Drenar el resto: tail acotada y usage visible al final.
            let mut rest = Vec::new();
            let mut buf = [0u8; 512];
            loop {
                let n = tee.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                rest.extend_from_slice(&buf[..n]);
            }
            assert_eq!(&rest, &(c2.into_iter().chain(c3.clone().into_iter()).collect::<Vec<u8>>()));
            assert!(tee.tail.len() <= crate::usage::TAIL_MAX_BYTES);
            let text = String::from_utf8_lossy(&tee.tail).to_string();
            assert_eq!(crate::usage::extract_usage_from_sse(&text), (Some(5), Some(7)));
            // Limpieza: quitar el override y el scratch (no filtrar al resto).
            unsafe { std::env::remove_var("LOCALMIND_USAGE_PATH") };
        }
        #[test]
        fn favicon_faltante_es_404() {
            // Decisión del handler vía load_asset: nombre inexistente → None → 404.
            let dir = std::path::PathBuf::from("dir-que-no-existe-12345");
            assert!(super::super::load_asset(&dir, "localmind.ico").is_none());
            assert!(super::super::load_asset(&dir, "localmind.png").is_none());
            // Y un archivo real sí carga (el propio server.rs como testigo).
            let here = std::path::PathBuf::from("src");
            assert!(super::super::load_asset(&here, "server.rs").is_some());
        }
        #[test]
        fn reasoning_effort_max_a_xhigh_y_desconocido_se_elimina() {
            // Misma función que usan los tres proxys: `/v1/chat/completions`,
            // `/v1/messages` y `/v1/responses` (todos llaman `sanitize_payload`
            // antes de reenviar al motor).
            let get = |body: &str| {
                let out = super::super::sanitize_payload(body.as_bytes().to_vec());
                let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
                v.get("reasoning_effort").cloned()
            };
            // `max` → `xhigh` (era el 500 de la plantilla Qwen).
            assert_eq!(get(r#"{"model":"m","reasoning_effort":"max"}"#), Some(serde_json::json!("xhigh")));
            assert_eq!(get(r#"{"model":"m","reasoning_effort":"MAX"}"#), Some(serde_json::json!("xhigh")));
            // Comportamiento previo intacto.
            assert_eq!(get(r#"{"model":"m","reasoning_effort":"minimal"}"#), Some(serde_json::json!("low")));
            assert_eq!(get(r#"{"model":"m","reasoning_effort":"high"}"#), Some(serde_json::json!("xhigh")));
            // Válidos intactos (normalizados a minúsculas).
            assert_eq!(get(r#"{"model":"m","reasoning_effort":"low"}"#), Some(serde_json::json!("low")));
            assert_eq!(get(r#"{"model":"m","reasoning_effort":"medium"}"#), Some(serde_json::json!("medium")));
            assert_eq!(get(r#"{"model":"m","reasoning_effort":"xhigh"}"#), Some(serde_json::json!("xhigh")));
            assert_eq!(get(r#"{"model":"m","reasoning_effort":"Medium"}"#), Some(serde_json::json!("medium")));
            // Desconocidos (incluido `off`, que la plantilla también rechaza)
            // y no-strings: campo eliminado, el motor usa su default.
            let dropped = |body: &str| {
                let out = super::super::sanitize_payload(body.as_bytes().to_vec());
                let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
                assert!(v.get("reasoning_effort").is_none(), "debió eliminarse: {:?}", String::from_utf8_lossy(&out));
            };
            dropped(r#"{"model":"m","reasoning_effort":"ultra"}"#);
            dropped(r#"{"model":"m","reasoning_effort":"off"}"#);
            dropped(r#"{"model":"m","reasoning_effort":""}"#);
            dropped(r#"{"model":"m","reasoning_effort":42}"#);
            dropped(r#"{"model":"m","reasoning_effort":null}"#);
            // Sin campo → sin campo; resto del body intacto.
            let out = super::super::sanitize_payload(br#"{"model":"m","temperature":0.7}"#.to_vec());
            let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
            assert!(v.get("reasoning_effort").is_none());
            assert_eq!(v["temperature"], serde_json::json!(0.7));
            // No-JSON pasa intacto.
            assert_eq!(super::super::sanitize_payload(b"no-json".to_vec()), b"no-json".to_vec());
        }

        #[test]
        fn config_speculation_enabled_persiste_y_subclave_desconocida_se_rechaza() {
            // `POST /api/config {"engine":{"speculation":{"enabled":false}}}`:
            // misma lógica de validación del handler, probada sin socket:
            // persiste `false` y expone `speculation_enabled` en el GET.
            let dir = std::env::temp_dir().join(format!("lm-test-speccfg-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("localmind.toml");
            let store = crate::config::ConfigStore::load_from_path(&path);
            // Estado inicial: `speculation` ausente => GET expone `false`.
            let v0: serde_json::Value = serde_json::from_str(&super::super::app_config_json(&store.get())).unwrap();
            assert_eq!(v0["engine"]["speculation_enabled"], serde_json::json!(false));
            // Aplicar `{"enabled":false}` con la misma regla del handler:
            // solo se acepta la sub-clave `enabled` y debe ser booleano.
            let apply = |store: &crate::config::ConfigStore, body: &str| -> Result<bool, String> {
                let val: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
                let sm = val.get("engine").and_then(|e| e.get("speculation")).and_then(|s| s.as_object()).ok_or("engine.speculation")?;
                for k in sm.keys() {
                    if k != "enabled" {
                        return Err(format!("campo inválido: engine.speculation.{} (solo se acepta enabled)", k));
                    }
                }
                match sm.get("enabled") {
                    Some(b) if b.is_boolean() => {
                        let en = b.as_bool().unwrap_or(true);
                        store.update(|c| {
                            if c.engine.speculation.is_none() {
                                c.engine.speculation = Some(crate::config::SpeculationConfig::default());
                            }
                            if let Some(s) = c.engine.speculation.as_mut() {
                                s.enabled = en;
                            }
                        });
                        store.save()?;
                        Ok(en)
                    }
                    _ => Err("campo inválido: engine.speculation.enabled".to_string()),
                }
            };
            assert_eq!(apply(&store, r#"{"engine":{"speculation":{"enabled":false}}}"#).unwrap(), false);
            assert_eq!(store.get().engine.speculation.as_ref().map(|s| s.enabled), Some(false));
            let v1: serde_json::Value = serde_json::from_str(&super::super::app_config_json(&store.get())).unwrap();
            assert_eq!(v1["engine"]["speculation_enabled"], serde_json::json!(false));
            // Recarga desde disco: persiste.
            let store2 = crate::config::ConfigStore::load_from_path(&path);
            assert_eq!(store2.get().engine.speculation.as_ref().map(|s| s.enabled), Some(false));
            // Sub-clave desconocida => 400 con el mensaje del handler.
            let err = apply(&store, r#"{"engine":{"speculation":{"enabled":true,"n":8}}}"#).unwrap_err();
            assert!(err.contains("engine.speculation.n") && err.contains("solo se acepta enabled"), "{}", err);
            // `enabled` no booleano => 400.
            let err2 = apply(&store, r#"{"engine":{"speculation":{"enabled":"si"}}}"#).unwrap_err();
            assert!(err2.contains("engine.speculation.enabled"), "{}", err2);
            let _ = std::fs::remove_dir_all(&dir);
        }

                // ---- LM-MOD-3: el id anunciado sigue al motor, D-2 intacto ----

        fn store_de_prueba(dir: &std::path::Path) -> crate::config::ConfigStore {
            let _ = std::fs::create_dir_all(dir);
            crate::config::ConfigStore::load_from_path(&dir.join("localmind.toml"))
        }

        /// El id que anuncia `/v1/models` y al que el proxy reescribe ES el
        /// modelo realmente servido, y cambia con él.
        ///
        /// Falla sobre el código original solo en la parte de "cambia con él":
        /// allí el literal `qwen3.8-27b` convivía con el filename, así que la
        /// lista podía anunciar los dos. Aquí se exige una sola verdad.
        #[test]
        fn v1_models_anuncia_el_servido_y_no_un_nombre_fijo() {
            let dir = std::env::temp_dir().join(format!("lm-test-v1m-{}", std::process::id()));
            let cfg = store_de_prueba(&dir);
            let st = |model: &str, port: u16| ServerStatus {
                status: "running".to_string(),
                is_healthy: true,
                model: model.to_string(),
                port,
                ..Default::default()
            };

            let bonsai: serde_json::Value =
                serde_json::from_str(&models_list_json(&st("Ternary-Bonsai-2-27B-PTQ1_0.gguf", 8080), &cfg)).unwrap();
            let ids: Vec<String> = bonsai["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m["id"].as_str().unwrap().to_string())
                .collect();
            assert_eq!(ids[0], "Ternary-Bonsai-2-27B-PTQ1_0", "el id servido no encabeza la lista: {:?}", ids);
            // El alias de compat sigue publicado (agentes en la calle lo usan).
            assert!(ids.iter().any(|i| i == "localmind"), "se perdió el alias localmind: {:?}", ids);

            // Con Qwen cargado, la lista cambia: no es un literal con aliases.
            let qwen: serde_json::Value =
                serde_json::from_str(&models_list_json(&st("Qwen3.8-27B-IQ4_XS_4BPW.gguf", 8080), &cfg)).unwrap();
            let qwen_ids: Vec<String> = qwen["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m["id"].as_str().unwrap().to_string())
                .collect();
            assert_eq!(qwen_ids[0], "Qwen3.8-27B-IQ4_XS_4BPW", "{:?}", qwen_ids);
            assert_ne!(ids[0], qwen_ids[0]);
            let _ = std::fs::remove_dir_all(&dir);
        }

        /// D-2 intacto: un agente configurado con `localmind`, y otro con el
        /// nombre del modelo ANTERIOR, siguen llegando al motor con el id
        /// servido. Esto es la red de seguridad que hace seguro cambiar la
        /// etiqueta: se cambia lo que se ANUNCIA, no lo que se ACEPTA.
        #[test]
        fn un_alias_o_un_modelo_viejo_siempre_llega_al_servido() {
            let dir = std::env::temp_dir().join(format!("lm-test-rw-{}", std::process::id()));
            let cfg = store_de_prueba(&dir);
            let st = ServerStatus {
                status: "running".to_string(),
                is_healthy: true,
                model: "Ternary-Bonsai-2-27B-PTQ1_0.gguf".to_string(),
                port: 8080,
                ..Default::default()
            };
            let served = served_model_id(&st, &cfg);
            assert_eq!(served, "Ternary-Bonsai-2-27B-PTQ1_0");

            let get = |b: &[u8]| {
                serde_json::from_slice::<serde_json::Value>(&rewrite_model_to_served(b, &served)).unwrap()
            };
            // Alias de compat.
            let v = get(br#"{"model":"localmind","messages":[]}"#);
            assert_eq!(v["model"], serde_json::json!("Ternary-Bonsai-2-27B-PTQ1_0"));
            // Con prefijo de provider (como lo escribe el lanzador).
            let v = get(br#"{"model":"localmind/localmind","messages":[]}"#);
            assert_eq!(v["model"], serde_json::json!("Ternary-Bonsai-2-27B-PTQ1_0"));
            // El modelo ANTERIOR: el caso de un config escrito antes del cambio.
            let v = get(br#"{"model":"qwen3.8-27b","messages":[]}"#);
            assert_eq!(v["model"], serde_json::json!("Ternary-Bonsai-2-27B-PTQ1_0"));
            // Un nombre inventado tampoco se rechaza (nunca se rechaza).
            let v = get(br#"{"model":"modelo-que-no-existe","messages":[]}"#);
            assert_eq!(v["model"], serde_json::json!("Ternary-Bonsai-2-27B-PTQ1_0"));
            // El resto del payload no se toca.
            let v = get(br#"{"model":"localmind","temperature":0,"stream":true}"#);
            assert_eq!(v["temperature"], serde_json::json!(0));
            assert_eq!(v["stream"], serde_json::json!(true));
            // Un body sin `model` se devuelve intacto (no se inventa un campo).
            let out = rewrite_model_to_served(br#"{"messages":[]}"#, &served);
            assert_eq!(out, br#"{"messages":[]}"#.to_vec());
            let _ = std::fs::remove_dir_all(&dir);
        }

        /// Motor apagado: `/v1/models` SIGUE contestando (compat) y usa el
        /// alias estable, no el nombre del modelo de la sesión anterior.
        /// La etiqueta honesta de "no hay modelo" la pone la UI y el 409 del
        /// lanzador; aquí la red de seguridad nunca se queda sin responder.
        #[test]
        fn motor_apagado_responde_con_el_alias_y_no_con_el_viejo() {
            let dir = std::env::temp_dir().join(format!("lm-test-off-{}", std::process::id()));
            let cfg = store_de_prueba(&dir);
            // `ServerStatus` conserva el `model` de la sesión anterior al parar.
            let parado = ServerStatus { status: "stopped".to_string(), port: 0, ..Default::default() };
            let cuerpo = models_list_json(&parado, &cfg);
            let v: serde_json::Value = serde_json::from_str(&cuerpo).unwrap();
            let ids: Vec<String> = v["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m["id"].as_str().unwrap().to_string())
                .collect();
            assert!(!ids.is_empty(), "/v1/models dejó de contestar con el motor apagado");
            assert!(ids.iter().any(|i| i == "localmind"), "sin alias de compat: {:?}", ids);
            // El lanzador, en cambio, NO escribe nada: eso ya lo hace el 409.
            assert!(!crate::agents::engine_live(&parado));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]

        #[test]
        fn header_parse_de_literal_no_pania() {
            // Los `Header::from_bytes(...).unwrap()` que quedan en el archivo
            // parsean LITERALES de byte: no pueden fallar, y por eso el `unwrap`
            // es aceptable ahí. Este test fija ese supuesto para cada literal
            // realmente usado: si una edición futura vuelve dinámico uno de
            // ellos, esto falla y obliga a la forma `Option` de
            // `cors_origin_header` / `session_cookie_header`.
            let literales: &[(&[u8], &[u8])] = &[
                (b"Content-Type", b"application/json"),
                (b"Content-Type", b"text/html; charset=utf-8"),
                (b"Content-Type", b"text/plain; charset=utf-8"),
                (b"Content-Type", b"text/event-stream"),
                (b"Content-Type", b"image/x-icon"),
                (b"Content-Type", b"image/png"),
                (b"Content-Disposition", b"attachment; filename=\"localmind_profiles.json\""),
                (b"Cache-Control", b"no-cache"),
                (b"Connection", b"keep-alive"),
                (b"Access-Control-Allow-Methods", b"GET, POST, OPTIONS, PUT, DELETE"),
                (b"Access-Control-Allow-Headers", b"Content-Type, Authorization, x-api-key"),
            ];
            for &(name, value) in literales {
                assert!(
                    Header::from_bytes(name, value).is_ok(),
                    "literal no parseable: {}: {}",
                    String::from_utf8_lossy(name),
                    String::from_utf8_lossy(value)
                );
            }
            // Y el valor DINÁMICO de la cookie ya no entra en panic: una clave
            // manipulada da `None` y el llamante responde 500.
            // Y el valor DINÁMICO de la cookie ya no entra en panic: `Option`
            // en vez de `unwrap`, así que una clave manipulada da `None` y el
            // llamante responde 500. Comprobado empíricamente: el parser de
            // `http` rechaza un byte NO-ASCII en el valor (un `gateway.key`
            // alterado o guardado en otra codificación), no un byte de control.
            assert!(super::super::session_cookie_header(&"a".repeat(64)).is_some());
            assert!(super::super::session_cookie_header("clave\u{80}").is_none());
        }

        #[test]
        fn api_start_rechaza_cuerpo_malformado() {
            // Un tipo equivocado NO puede degradarse a un arranque con
            // defaults (que devolvía 200 y encendía el motor).
            let err = super::super::parse_start_body(r#"{"context":"abc"}"#).unwrap_err();
            assert!(err.starts_with("JSON inválido"), "{}", err);
            // Tampoco un cuerpo que no es un objeto.
            let err2 = super::super::parse_start_body("[1,2,3]").unwrap_err();
            assert!(err2.starts_with("JSON inválido"), "{}", err2);
            // Cuerpo VACÍO: sigue siendo default (POST sin cuerpo, no rompe).
            let d = super::super::parse_start_body("").unwrap();
            assert!(d.model.is_none() && d.context.is_none() && d.priority.is_none());
            let ws = super::super::parse_start_body("   ").unwrap();
            assert!(ws.profile.is_none());
            // Bien formado: pasa intacto.
            let ok = super::super::parse_start_body(r#"{"model":"a.gguf","context":32768}"#).unwrap();
            assert_eq!(ok.model.as_deref(), Some("a.gguf"));
            assert_eq!(ok.context, Some(32768));
        }

        #[test]
        fn profiles_import_rechaza_flags_invalidas() {
            // El import no puede saltarse los validadores de /profiles/save.
            let perfil = |id: &str, ctx: u64, flags: &str| {
                format!(
                    r#"[{{"id":"{}","name":"N","description":"","context":{},"cache_ram":0,"extra_flags":{}}}]"#,
                    id, ctx, flags
                )
            };
            // Flag con inyección de shell (mismo charset que `profile_flag_ok`).
            let err = super::super::parse_profiles_import(&perfil("velocidad", 32768, r#"["-flag & calc.exe"]"#))
                .unwrap_err();
            assert!(err.contains("extra_flags"), "{}", err);
            // Contexto fuera de rango / no múltiplo de 1024.
            let err2 = super::super::parse_profiles_import(&perfil("velocidad", 1023, "[]")).unwrap_err();
            assert!(err2.contains("context"), "{}", err2);
            // Id inválido.
            let err3 = super::super::parse_profiles_import(&perfil("Perfil Mal", 32768, "[]")).unwrap_err();
            assert!(err3.contains("id"), "{}", err3);
            // Lista vacía y JSON roto.
            assert!(super::super::parse_profiles_import("[]").unwrap_err().contains("vacía"));
            assert!(super::super::parse_profiles_import("{").unwrap_err().starts_with("JSON inválido"));
            // Perfil válido: pasa intacto.
            let ok = super::super::parse_profiles_import(&perfil("libros", 131072, r#"["--no-mmap"]"#)).unwrap();
            assert_eq!(ok.len(), 1);
            assert_eq!(ok[0].context, 131072);
        }
    }
}

mod sse {
    use super::*;
    use std::sync::mpsc;

    /// SSE EventReader: primero history, luego live por mpsc.
    pub struct EventReader {
        rx: std::sync::Mutex<mpsc::Receiver<String>>,
        initial: Vec<LogEvent>,
        next_live_seq: u64,
        done: bool,
        chunk: Vec<u8>,
    }

    impl EventReader {
        pub fn new(rx: mpsc::Receiver<String>, initial: Vec<LogEvent>) -> Self {
            let next_live_seq = initial.last().map(|l| l.seq + 1).unwrap_or(0);
            Self {
                rx: std::sync::Mutex::new(rx),
                initial,
                next_live_seq,
                done: false,
                chunk: Vec::new(),
            }
        }

        fn encode(&self, seq: u64, line: &str) -> Vec<u8> {
            let data = serde_json::json!({ "type":"log","seq": seq, "line": line });
            // Pad to force tiny_http BufWriter (1KB) flush — SSE events must hit the client live.
            let mut s = format!("data: {}\n: pad\n\n", data);
            let min = 1100;
            if s.len() < min {
                let pad = min - s.len() - 3;
                s.push_str(": ");
                s.push_str(&" ".repeat(pad));
                s.push_str("\n");
            }
            s.into_bytes()
        }
    }

    impl Read for EventReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.chunk.is_empty() {
                if self.done {
                    return Ok(0);
                }
                if let Some(ev) = if self.initial.is_empty() {
                    None
                } else {
                    Some(self.initial.remove(0))
                } {
                    self.chunk = self.encode(ev.seq, &ev.line);
                } else {
                    let g = match self.rx.lock() { Ok(g) => g, Err(po) => po.into_inner() };
                    match g.recv() {
                        Ok(line) => {
                            let seq = self.next_live_seq;
                            self.next_live_seq += 1;
                            self.chunk = self.encode(seq, &line);
                        }
                        Err(_) => {
                            self.done = true;
                            return Ok(0);
                        }
                    }
                }
            }
            let n = self.chunk.len().min(buf.len());
            buf[..n].copy_from_slice(&self.chunk[..n]);
            self.chunk.drain(..n);
            Ok(n)
        }
    }
}
