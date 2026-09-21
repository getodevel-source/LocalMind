use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use tiny_http::{Header, Response, Server, StatusCode};

use crate::process::{ProcessManager, StartRequest};
use crate::profiles::get_hardware_profiles;

pub struct HttpServer {
    server: Arc<Server>,
    port: u16,
}

impl HttpServer {
    pub fn start(process_mgr: Arc<ProcessManager>, base_dir: PathBuf) -> Result<Self, String> {
        let mut port = 17860;
        let mut server = None;

        for p in port..port + 10 {
            if let Ok(s) = Server::http(format!("127.0.0.1:{}", p)) {
                server = Some(s);
                port = p;
                break;
            }
        }

        let server = server.ok_or_else(|| "No se pudo iniciar el servidor HTTP en el rango 17860-17870".to_string())?;
        let server = Arc::new(server);
        let srv_clone = Arc::clone(&server);
        let mgr = Arc::clone(&process_mgr);
        let ui_path = base_dir.join("ui.html");
        let ico_path = base_dir.join("localmind.ico");
        let png_path = base_dir.join("localmind.png");

        thread::spawn(move || {
            for mut req in srv_clone.incoming_requests() {
                let url = req.url().to_string();
                let method = req.method().to_string();

                let cors_header = Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap();
                let cors_methods = Header::from_bytes(
                    &b"Access-Control-Allow-Methods"[..],
                    &b"GET, POST, OPTIONS, PUT, DELETE"[..],
                )
                .unwrap();
                let cors_headers = Header::from_bytes(
                    &b"Access-Control-Allow-Headers"[..],
                    &b"Content-Type, Authorization"[..],
                )
                .unwrap();

                if method == "OPTIONS" {
                    let mut resp = Response::empty(200);
                    resp.add_header(cors_header);
                    resp.add_header(cors_methods);
                    resp.add_header(cors_headers);
                    let _ = req.respond(resp);
                    continue;
                }

                if method == "GET" && (url == "/" || url == "/index.html") {
                    let html = if ui_path.exists() {
                        std::fs::read_to_string(&ui_path).unwrap_or_else(|_| include_str!("../ui_fallback.html").to_string())
                    } else {
                        include_str!("../ui_fallback.html").to_string()
                    };

                    let ct = Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap();
                    let mut resp = Response::from_string(html);
                    resp.add_header(ct);
                    resp.add_header(cors_header);
                    let _ = req.respond(resp);
                    continue;
                }

                if method == "GET" && url == "/localmind.ico" {
                    let bytes = std::fs::read(&ico_path).unwrap_or_default();
                    let ct = Header::from_bytes(&b"Content-Type"[..], &b"image/x-icon"[..]).unwrap();
                    let mut resp = Response::from_data(bytes);
                    resp.add_header(ct);
                    resp.add_header(cors_header);
                    let _ = req.respond(resp);
                    continue;
                }

                if method == "GET" && url == "/localmind.png" {
                    let bytes = std::fs::read(&png_path).unwrap_or_default();
                    let ct = Header::from_bytes(&b"Content-Type"[..], &b"image/png"[..]).unwrap();
                    let mut resp = Response::from_data(bytes);
                    resp.add_header(ct);
                    resp.add_header(cors_header);
                    let _ = req.respond(resp);
                    continue;
                }

                // API Routes
                if method == "GET" && url == "/api/models" {
                    let models = mgr.list_models();
                    let json = serde_json::to_string(&models).unwrap_or_else(|_| "[]".to_string());
                    let ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
                    let mut resp = Response::from_string(json);
                    resp.add_header(ct);
                    resp.add_header(cors_header);
                    let _ = req.respond(resp);
                    continue;
                }

                if method == "GET" && url == "/api/profiles" {
                    let profiles = get_hardware_profiles();
                    let json = serde_json::to_string(&profiles).unwrap_or_else(|_| "[]".to_string());
                    let ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
                    let mut resp = Response::from_string(json);
                    resp.add_header(ct);
                    resp.add_header(cors_header);
                    let _ = req.respond(resp);
                    continue;
                }

                if method == "GET" && url == "/api/status" {
                    let st = mgr.get_status();
                    let json = serde_json::to_string(&st).unwrap_or_else(|_| "{}".to_string());
                    let ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
                    let mut resp = Response::from_string(json);
                    resp.add_header(ct);
                    resp.add_header(cors_header);
                    let _ = req.respond(resp);
                    continue;
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

                    let ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
                    match mgr.start(start_req) {
                        Ok(pid) => {
                            let json = format!(r#"{{"status":"starting","pid":{}}}"#, pid);
                            let mut resp = Response::from_string(json);
                            resp.add_header(ct);
                            resp.add_header(cors_header);
                            let _ = req.respond(resp);
                        }
                        Err(e) => {
                            let json = format!(r#"{{"error":"{}"}}"#, e.replace('"', "\\\""));
                            let mut resp = Response::from_string(json).with_status_code(StatusCode(400));
                            resp.add_header(ct);
                            resp.add_header(cors_header);
                            let _ = req.respond(resp);
                        }
                    }
                    continue;
                }

                if method == "POST" && url == "/api/stop" {
                    mgr.stop();
                    let json = r#"{"status":"stopped"}"#;
                    let ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
                    let mut resp = Response::from_string(json);
                    resp.add_header(ct);
                    resp.add_header(cors_header);
                    let _ = req.respond(resp);
                    continue;
                }

                if method == "GET" && url == "/api/logs" {
                    let logs = mgr.get_recent_logs();
                    let json = serde_json::to_string(&logs).unwrap_or_else(|_| "[]".to_string());
                    let ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
                    let mut resp = Response::from_string(json);
                    resp.add_header(ct);
                    resp.add_header(cors_header);
                    let _ = req.respond(resp);
                    continue;
                }

                if method == "POST" && url == "/api/launch_omp" {
                    let mut body = String::new();
                    let _ = req.as_reader().read_to_string(&mut body);

                    let req_model = serde_json::from_str::<serde_json::Value>(&body)
                        .ok()
                        .and_then(|v| v.get("model").and_then(|m| m.as_str().map(|s| s.to_string())));

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

                    mgr.log(&format!("[LocalMind] Terminal OMP lanzada conectada a {}.", omp_model));
                    let json = format!(r#"{{"status":"ok","model":"{}"}}"#, omp_model);
                    let ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
                    let mut resp = Response::from_string(json);
                    resp.add_header(ct);
                    resp.add_header(cors_header);
                    let _ = req.respond(resp);
                    continue;
                }

                if method == "POST" && url == "/api/open_browser" {
                    let _ = std::process::Command::new("cmd.exe")
                        .args(["/c", "start http://127.0.0.1:8080"])
                        .spawn();
                    mgr.log("[LocalMind] Abierto navegador web en http://127.0.0.1:8080");
                    let json = r#"{"status":"ok"}"#;
                    let ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
                    let mut resp = Response::from_string(json);
                    resp.add_header(ct);
                    resp.add_header(cors_header);
                    let _ = req.respond(resp);
                    continue;
                }

                // Proxy route: Forward /v1/chat/completions to llama-server with sanitization
                if method == "POST" && url == "/v1/chat/completions" {
                    let mut body_bytes = Vec::new();
                    let _ = req.as_reader().read_to_end(&mut body_bytes);

                    // Sanitize reasoning_effort if present to prevent Jinja 500 error
                    let payload_bytes = if let Ok(mut json_val) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
                        if let Some(obj) = json_val.as_object_mut() {
                            if let Some(re) = obj.get("reasoning_effort").and_then(|v| v.as_str()) {
                                if re.eq_ignore_ascii_case("minimal") {
                                    obj.insert("reasoning_effort".to_string(), serde_json::Value::String("low".to_string()));
                                } else if re.eq_ignore_ascii_case("high") {
                                    obj.insert("reasoning_effort".to_string(), serde_json::Value::String("xhigh".to_string()));
                                }
                            }
                        }
                        serde_json::to_vec(&json_val).unwrap_or(body_bytes)
                    } else {
                        body_bytes
                    };

                    match ureq::post("http://127.0.0.1:8080/v1/chat/completions")
                        .set("Content-Type", "application/json")
                        .send_bytes(&payload_bytes)
                    {
                        Ok(resp) => {
                            let mut stream_reader = resp.into_reader();
                            let mut out_bytes = Vec::new();
                            let _ = stream_reader.read_to_end(&mut out_bytes);

                            let ct = Header::from_bytes(&b"Content-Type"[..], &b"text/event-stream"[..]).unwrap();
                            let mut proxy_resp = Response::from_data(out_bytes);
                            proxy_resp.add_header(ct);
                            proxy_resp.add_header(cors_header);
                            let _ = req.respond(proxy_resp);
                        }
                        Err(e) => {
                            let err_json = format!(r#"{{"error":"Error al contactar motor: {}"}}"#, e);
                            let ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
                            let mut err_resp = Response::from_string(err_json).with_status_code(StatusCode(502));
                            err_resp.add_header(ct);
                            err_resp.add_header(cors_header);
                            let _ = req.respond(err_resp);
                        }
                    }
                    continue;
                }

                // 404 fallback
                let mut resp = Response::from_string("Not Found").with_status_code(StatusCode(404));
                resp.add_header(cors_header);
                let _ = req.respond(resp);
            }
        });

        Ok(Self { server, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}
