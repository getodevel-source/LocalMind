use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tiny_http::{Header, Response, Server, StatusCode};

use crate::config::ConfigStore;
use crate::process::{LogEvent, ProcessManager, ServerStatus, StartRequest};

pub struct HttpServer {
    port: u16,
}

fn cors_headers() -> [Header; 3] {
    [
        Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap(),
        Header::from_bytes(
            &b"Access-Control-Allow-Methods"[..],
            &b"GET, POST, OPTIONS, PUT, DELETE"[..],
        )
        .unwrap(),
        Header::from_bytes(
            &b"Access-Control-Allow-Headers"[..],
            &b"Content-Type, Authorization"[..],
        )
        .unwrap(),
    ]
}

fn json_response(status_code: u16, body: String) -> Response<Cursor<Vec<u8>>> {
    let mut resp = Response::from_string(body).with_status_code(StatusCode(status_code));
    let ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    resp.add_header(ct);
    for h in cors_headers() {
        resp.add_header(h);
    }
    resp
}

fn status_json(st: ServerStatus) -> String {
    serde_json::to_string(&st).unwrap_or_else(|_| "{}".to_string())
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

fn sanitize_payload(body_bytes: Vec<u8>) -> Vec<u8> {
    if let Ok(mut json_val) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
        if let Some(obj) = json_val.as_object_mut() {
            if let Some(re) = obj.get("reasoning_effort").and_then(|v| v.as_str()).map(str::to_string) {
                let mapped = if re.eq_ignore_ascii_case("minimal") {
                    "low".to_string()
                } else if re.eq_ignore_ascii_case("high") {
                    "xhigh".to_string()
                } else {
                    re.clone()
                };
                obj.insert("reasoning_effort".to_string(), serde_json::Value::String(mapped));
            }
        }
        serde_json::to_vec(&json_val).unwrap_or(body_bytes)
    } else {
        body_bytes
    }
}

impl HttpServer {
    pub fn start(
        process_mgr: Arc<ProcessManager>,
        config: Arc<ConfigStore>,
        base_dir: PathBuf,
    ) -> Result<Self, String> {
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

        thread::spawn(move || {
            for req in srv_clone.incoming_requests() {
                let url = req.url().to_string();
                let method = req.method().to_string();
                let mgr = Arc::clone(&process_mgr);
                let cfg = Arc::clone(&config);
                let base = base_dir.clone();

                thread::spawn(move || {
                    handle_request(req, method, url, mgr, cfg, base);
                });
            }
        });

        Ok(Self { port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

fn handle_request(
    mut req: tiny_http::Request,
    method: String,
    url: String,
    mgr: Arc<ProcessManager>,
    cfg: Arc<ConfigStore>,
    base_dir: PathBuf,
) {
    let [cors, cors_methods, cors_all] = cors_headers();

    if method == "OPTIONS" {
        let mut resp = Response::empty(200);
        resp.add_header(cors);
        resp.add_header(cors_methods);
        resp.add_header(cors_all);
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
        let mut resp = Response::from_string(html);
        resp.add_header(ct);
        resp.add_header(cors);
        let _ = req.respond(resp);
        return;
    }

    if method == "GET" && url == "/localmind.ico" {
        let bytes = std::fs::read(base_dir.join("localmind.ico")).unwrap_or_default();
        let ct = Header::from_bytes(&b"Content-Type"[..], &b"image/x-icon"[..]).unwrap();
        let mut resp = Response::from_data(bytes);
        resp.add_header(ct);
        resp.add_header(cors);
        let _ = req.respond(resp);
        return;
    }

    if method == "GET" && url == "/localmind.png" {
        let bytes = std::fs::read(base_dir.join("localmind.png")).unwrap_or_default();
        let ct = Header::from_bytes(&b"Content-Type"[..], &b"image/png"[..]).unwrap();
        let mut resp = Response::from_data(bytes);
        resp.add_header(ct);
        resp.add_header(cors);
        let _ = req.respond(resp);
        return;
    }

    if method == "GET" && url == "/api/models" {
        let json = serde_json::to_string(&mgr.list_models()).unwrap_or_else(|_| "[]".to_string());
        let _ = req.respond(json_response(200, json));
        return;
    }

    if method == "GET" && url == "/api/profiles" {
        let profiles = crate::profiles::get_hardware_profiles(&cfg.get());
        let json = serde_json::to_string(&profiles).unwrap_or_else(|_| "[]".to_string());
        let _ = req.respond(json_response(200, json));
        return;
    }

    if method == "GET" && url == "/api/status" {
        let _ = req.respond(json_response(200, status_json(mgr.get_status())));
        return;
    }

    if method == "GET" && url == "/api/settings" {
        let c = cfg.get();
        let json = settings_json(&c, cfg.path(), c.engine.http_port);
        let _ = req.respond(json_response(200, json));
        return;
    }

    if method == "GET" && url == "/api/metrics" {
        let st = mgr.get_status();
        if (st.status != "running" && !st.is_healthy) || st.port == 0 {
            let _ = req.respond(json_response(502, r#"{"error":"engine_down"}"#.into()));
            return;
        }
        match ureq::get(&format!("http://127.0.0.1:{}/metrics", st.port))
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
                    let _ = req.respond(json_response(200, json.to_string()));
                } else {
                    let _ = req.respond(json_response(502, r#"{"error":"engine_read_failed"}"#.into()));
                }
            }
            Err(_) => {
                let _ = req.respond(json_response(502, r#"{"error":"engine_down"}"#.into()));
            }
        }
        return;
    }

    if method == "POST" && url == "/api/start" {
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let start_req: StartRequest = serde_json::from_str(&body).unwrap_or(StartRequest {
            model: None,
            profile: None,
            context: None,
            threads: None,
            priority: None,
        });
        match mgr.start(start_req) {
            Ok(pid) => {
                let _ = req.respond(json_response(
                    200,
                    format!(r#"{{"status":"starting","pid":{}}}"#, pid),
                ));
            }
            Err(e) => {
                let _ = req.respond(
                    json_response(400, serde_json::json!({ "error": e }).to_string()),
                );
            }
        }
        return;
    }

    if method == "POST" && url == "/api/stop" {
        mgr.stop();
        let _ = req.respond(json_response(200, r#"{"status":"stopped"}"#.into()));
        return;
    }

    if method == "GET" && url == "/api/logs" {
        let json = serde_json::to_string(&mgr.get_recent_logs()).unwrap_or_else(|_| "[]".to_string());
        let _ = req.respond(json_response(200, json));
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
        let _ = req.respond(resp);
        return;
    }

    if method == "POST" && url == "/api/launch_omp" {
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let req_model = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("model").and_then(|m| m.as_str().map(str::to_string)));

        let active_model = mgr.get_status().model;
        let chosen = req_model.filter(|m| !m.is_empty()).unwrap_or(active_model);

        let omp_model = if chosen.to_lowercase().contains("bonsai") {
            "bonsai/bonsai-2-27b"
        } else {
            "localmind/qwen3.8-27b"
        };

        let cmd_str = format!("omp --model {}", omp_model);

        let wt_path = std::env::var("LOCALAPPDATA")
            .map(|l| PathBuf::from(l).join("Microsoft/WindowsApps/wt.exe"))
            .ok();

        if let Some(wt) = wt_path.filter(|p| p.exists()) {
            let _ = std::process::Command::new(wt)
                .args(["-w", "0", "new-tab", "cmd.exe", "/k", &cmd_str])
                .spawn();
        } else {
            let _ = std::process::Command::new("cmd.exe")
                .args(["/c", &format!("start {}", cmd_str)])
                .spawn();
        }

        mgr.log(&format!(
            "[LocalMind] Terminal OMP lanzada conectada a {}.",
            omp_model
        ));
        let _ = req.respond(json_response(
            200,
            format!(r#"{{"status":"ok","model":"{}"}}"#, omp_model),
        ));
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
        let _ = req.respond(json_response(200, r#"{"status":"ok"}"#.into()));
        return;
    }

    // Proxy SSE LIVE: reader de ureq directo en el body de tiny_http.
    if method == "POST" && url == "/v1/chat/completions" {
        let mut body_bytes = Vec::new();
        let _ = req.as_reader().read_to_end(&mut body_bytes);
        let payload_bytes = sanitize_payload(body_bytes);

        let st = mgr.get_status();
        if st.port == 0 {
            let _ = req.respond(json_response(502, r#"{"error":"engine_down"}"#.into()));
            return;
        }
        match ureq::post(&format!("http://127.0.0.1:{}/v1/chat/completions", st.port))
            .set("Content-Type", "application/json")
            .send_bytes(&payload_bytes)
        {
            Ok(resp) => {
                let reader: Box<dyn Read + Send + Sync + 'static> = Box::new(resp.into_reader());
                let ct = Header::from_bytes(&b"Content-Type"[..], &b"text/event-stream"[..]).unwrap();
                let proxy_resp = Response::new(
                    StatusCode(200),
                    vec![ct, cors, cors_methods, cors_all],
                    reader,
                    None,
                    None,
                );
                let _ = req.respond(proxy_resp);
            }
            Err(ureq::Error::Status(code, resp)) => {
                let mut body = String::new();
                let _ = resp.into_reader().read_to_string(&mut body);
                let _ = req.respond(json_response(code, body));
            }
            Err(_) => {
                let _ = req.respond(json_response(
                    502,
                    r#"{"error":"Error al contactar motor"}"#.into(),
                ));
            }
        }
        return;
    }

    let _ = req.respond(json_response(404, "{\"error\":\"not_found\"}".into()));
}

fn sse_headers() -> Vec<Header> {
    let ct = Header::from_bytes(&b"Content-Type"[..], &b"text/event-stream"[..]).unwrap();
    let cc = Header::from_bytes(&b"Cache-Control"[..], &b"no-cache"[..]).unwrap();
    let cn = Header::from_bytes(&b"Connection"[..], &b"keep-alive"[..]).unwrap();
    let mut hs = vec![ct, cc, cn];
    hs.extend(cors_headers());
    hs
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
