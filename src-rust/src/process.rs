use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::ConfigStore;
use crate::profiles::HardwareProfile;

const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub filename: String,
    pub name: String,
    pub size_gb: f64,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatus {
    pub status: String,
    pub is_healthy: bool,
    pub pid: Option<u32>,
    pub model: String,
    pub context: usize,
    pub profile: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_remaining_secs: Option<u64>,
    pub last_error: Option<String>,
    // Puerta de aceptación (D1) + progreso/ETA (D4). Los Option se serializan
    // siempre (null si ausentes), como `last_error`; `default` solo para compat
    // de Deserialize.
    #[serde(default)]
    pub verifying: bool,
    #[serde(default)]
    pub starting_for_secs: u64,
    #[serde(default)]
    pub eta_secs: u64,
    #[serde(default)]
    pub decode_tps: Option<f64>,
    #[serde(default)]
    pub acceptance_ok: Option<bool>,
    #[serde(default)]
    pub acceptance_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuDevice {
    pub id: String,
    pub name: String,
    pub vram: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareInfo {
    pub cpu_cores: usize,
    pub cpu_name: String,
    pub gpus: Vec<GpuDevice>,
}

#[derive(Debug, Deserialize)]
pub struct StartRequest {
    pub model: Option<String>,
    pub profile: Option<String>,
    pub context: Option<usize>,
    pub threads: Option<String>,
    pub priority: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LogEvent {
    pub seq: u64,
    pub line: String,
}

pub struct ProcessManager {
    child: Arc<Mutex<Option<Child>>>,
    status: Arc<RwLock<ServerStatus>>,
    recent_logs: Arc<RwLock<VecDeque<LogEvent>>>,
    log_senders: Arc<Mutex<Vec<mpsc::Sender<String>>>>,
    running_poll: Arc<AtomicBool>,
    log_seq: Arc<AtomicU64>,
    /// UNIX-epoch secs de la última generación (chat request). Para auto-stop por inactividad.
    last_activity: Arc<AtomicU64>,
    /// Instante en que empezó el arranque vigente (para `starting_for_secs` y ETA).
    start_instant: Arc<Mutex<Option<Instant>>>,
    /// Generación de arranque: se incrementa en cada `start()`/`stop()` para que el
    /// poller detecte un arranque nuevo aunque haya estado bloqueado en la puerta.
    start_epoch: Arc<AtomicU64>,
    /// Duraciones de cargas exitosas (clave = modelo + contexto), para ETA.
    load_times: Arc<Mutex<HashMap<String, u64>>>,
    /// Archivo de log con rotación (P30): se crea perezoso al primer `log()`.
    log_file: PathBuf,
    config: Arc<ConfigStore>,
    base_dir: PathBuf,
    bin_dir: PathBuf,
    models_dir: PathBuf,
}

impl ProcessManager {
    pub fn new(base_dir: PathBuf, config: Arc<ConfigStore>) -> Self {
        let bin_dir = base_dir.join("bin");
        let models_dir = base_dir.join("models");

        let cfg = config.get();
        let status = Arc::new(RwLock::new(ServerStatus {
            status: "stopped".to_string(),
            is_healthy: false,
            pid: None,
            model: String::new(),
            context: 32768,
            profile: crate::config::DEFAULT_PROFILE_ID.to_string(),
            port: cfg.engine.llama_port,
            idle_remaining_secs: None,
            last_error: None,
            verifying: false,
            starting_for_secs: 0,
            eta_secs: 0,
            decode_tps: None,
            acceptance_ok: None,
            acceptance_error: None,
        }));

        let recent_logs: Arc<RwLock<VecDeque<LogEvent>>> = Arc::new(RwLock::new(VecDeque::new()));
        let log_senders: Arc<Mutex<Vec<mpsc::Sender<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let running_poll = Arc::new(AtomicBool::new(true));
        let log_seq = Arc::new(AtomicU64::new(0));
        let last_activity = Arc::new(AtomicU64::new(0));
        let start_instant: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let start_epoch = Arc::new(AtomicU64::new(0));
        // Duraciones de cargas exitosas (clave = modelo + contexto), para ETA.
        let load_times_path = Self::load_times_path();
        let load_times: Arc<Mutex<HashMap<String, u64>>> =
            Arc::new(Mutex::new(load_times_load(&load_times_path).unwrap_or_default()));
        // Log a archivo con rotación (P30): perezoso, best-effort.
        let log_file = crate::filelog::logs_dir(&base_dir).join("localmind.log");

        // Background poller: monitors health AND child process liveness
        let status_clone = Arc::clone(&status);
        let child_clone = Arc::clone(&child);
        let logs_for_err = Arc::clone(&recent_logs);
        let poll_flag = Arc::clone(&running_poll);
        let cfg_poll = Arc::clone(&config);
        let last_activity_poll = Arc::clone(&last_activity);
        let recent_logs_clone = Arc::clone(&recent_logs);
        let senders_clone = Arc::clone(&log_senders);
        let seq_clone = Arc::clone(&log_seq);
        let start_instant_poll = Arc::clone(&start_instant);
        let start_epoch_poll = Arc::clone(&start_epoch);
        let load_times_poll = Arc::clone(&load_times);
        let load_times_path_poll = load_times_path.clone();
        let models_dir_poll = models_dir.clone();
        let log_file_poll = log_file.clone();

        thread::spawn(move || {
            let mut consecutive_failures = 0u32;
            // La puerta de aceptación corre una sola vez por arranque (D1).
            let mut gate_done = false;
            let mut seen_epoch = start_epoch_poll.load(Ordering::Relaxed);
            while poll_flag.load(Ordering::Relaxed) {
                // Arranque nuevo (o stop): rearmar la puerta de aceptación.
                let ep = start_epoch_poll.load(Ordering::Relaxed);
                if ep != seen_epoch {
                    seen_epoch = ep;
                    gate_done = false;
                    consecutive_failures = 0;
                }
                // 1. Check if child process has exited or crashed
                let exited = {
                    let mut cl = child_clone.lock();
                    if let Some(c) = cl.as_mut() {
                        c.try_wait().ok().flatten()
                    } else {
                        None
                    }
                };
                let mut auto_stop = false;
                if let Some(exit_status) = exited {
                    let code = exit_status.to_string();
                    let summary = {
                        let logs = logs_for_err.read();
                        let lines: Vec<String> = logs.iter().map(|l| l.line.clone()).collect();
                        crash_summary(&lines, &code)
                    };
                    let mut st = status_clone.write();
                    // Aviso P20 pendiente si el crash era visible (starting/running).
                    let mut crash_notify: Option<(String, String, String)> = None;
                    if st.status == "starting" || st.status == "running" {
                        st.status = "error".to_string();
                        st.is_healthy = false;
                        st.pid = None;
                        st.verifying = false;
                        st.starting_for_secs = 0;
                        st.eta_secs = 0;
                        st.last_error = Some(summary.clone());
                        crash_notify = Some((
                            "El motor falló".to_string(),
                            summary,
                            "engine-failure".to_string(),
                        ));
                    }
                    drop(st);
                    if let Some((title, body, tag)) = crash_notify {
                        let ncfg = cfg_poll.get().notifications;
                        if crate::notify::should_notify(ncfg.enabled, ncfg.on_failure) {
                            let logf = log_file_poll.clone();
                            std::thread::spawn(move || {
                                crate::notify::notify(&title, &body, &tag, |err| {
                                    crate::filelog::write_log_line(&logf, err);
                                });
                            });
                        }
                    }
                } else {
                    // 2. Poll health endpoint if still starting or running
                    let (current_status, port) = {
                        let st = status_clone.read();
                        (st.status.clone(), st.port)
                    };

                    if current_status == "starting" || current_status == "running" {
                        // Progreso/ETA (D4): sin bloquear el lock durante el HTTP.
                        let (elapsed_secs, model_snapshot, context_snapshot) = {
                            let st = status_clone.read();
                            let elapsed = start_instant_poll
                                .lock()
                                .as_ref()
                                .map(|t| t.elapsed().as_secs())
                                .unwrap_or(0);
                            (elapsed, st.model.clone(), st.context)
                        };
                        {
                            let mut st = status_clone.write();
                            st.starting_for_secs = if current_status == "starting" { elapsed_secs } else { 0 };
                            if current_status == "starting" {
                                let stored = load_times_poll.lock().get(&load_key(&model_snapshot, context_snapshot)).copied();
                                let size_gb = Self::model_size_gb(&models_dir_poll, &model_snapshot);
                                st.eta_secs = eta_secs(stored, size_gb);
                            } else {
                                st.eta_secs = 0;
                            }
                        }
                        let is_ok = Self::health_check(port);
                        // Puerta de aceptación (D1): una sola vez por arranque, sin el lock
                        // cogido durante la request de verificación (el status sigue legible).
                        let mut gate_outcome: Option<Result<(f64, u64), String>> = None;
                        if is_ok && current_status == "starting" && !gate_done {
                            gate_done = true;
                            let my_epoch = seen_epoch;
                            {
                                let mut st = status_clone.write();
                                st.verifying = true;
                            }
                            let res = Self::run_acceptance_gate(port);
                            // Si hubo un stop()/start() durante la verificación (120 s),
                            // el resultado es de otro arranque: se descarta.
                            if start_epoch_poll.load(Ordering::Relaxed) == my_epoch {
                                gate_outcome = Some(res);
                            }
                        }
                        let mut pending_ok_log: Option<String> = None;
                        // Avisos P20 pendientes (se emiten tras soltar el lock).
                        let mut pending_notify: Option<(String, String, String)> = None;
                        {
                            let mut st = status_clone.write();
                            st.is_healthy = is_ok;
                            if let Some(outcome) = gate_outcome {
                                match outcome {
                                    Ok((tps, _tokens)) => {
                                        // Carga exitosa: registrar duración (D4) y declarar running.
                                        let secs = start_instant_poll
                                            .lock()
                                            .as_ref()
                                            .map(|t| t.elapsed().as_secs())
                                            .unwrap_or(0);
                                        let key = load_key(&st.model, st.context);
                                        load_times_poll.lock().insert(key.clone(), secs.max(1));
                                        let _ = load_times_save(&load_times_path_poll, &load_times_poll.lock());
                                        st.status = "running".to_string();
                                        st.verifying = false;
                                        st.starting_for_secs = 0;
                                        st.eta_secs = 0;
                                        st.decode_tps = Some(tps);
                                        st.acceptance_ok = Some(true);
                                        st.acceptance_error = None;
                                        st.last_error = None;
                                        pending_ok_log = Some(format!("[LocalMind] Verificación de arranque OK: {} t/s", tps.round() as u64));
                                        // Aviso P20: motor listo con modelo y velocidad.
                                        pending_notify = Some((
                                            "Motor listo".to_string(),
                                            format!("«{}» cargando en la GPU ({} t/s)", st.model, tps.round() as u64),
                                            "engine-ready".to_string(),
                                        ));
                                    }
                                    Err(detail) => {
                                        st.status = "error".to_string();
                                        st.is_healthy = false;
                                        st.verifying = false;
                                        st.starting_for_secs = 0;
                                        st.eta_secs = 0;
                                        st.acceptance_ok = Some(false);
                                        st.acceptance_error = Some(detail.clone());
                                        st.last_error = Some(detail.clone());
                                        // Aviso P20: fallo de la puerta con el motivo.
                                        pending_notify = Some((
                                            "El motor falló".to_string(),
                                            detail,
                                            "engine-failure".to_string(),
                                        ));
                                        *start_instant_poll.lock() = None;
                                    }
                                }
                            }
                            consecutive_failures = if is_ok { 0 } else { consecutive_failures.saturating_add(1) };
                            // 10 fallos seguidos (~10s) con status running → el hijo murió sin exit visible
                            if consecutive_failures >= 10 && st.status == "running" {
                                st.status = "error".to_string();
                                st.last_error =
                                    Some("El motor dejó de responder el endpoint /health".to_string());
                            }
                            // Auto-stop por inactividad (solo si está running y healthy)
                            let timeout = cfg_poll.get().engine.idle_timeout_secs;
                            if timeout > 0 && st.status == "running" {
                                // Verificar si llama-server tiene slots procesando actualmente
                                let is_busy = Self::check_slots_busy(port);
                                let now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0);
                                if is_busy {
                                    // El motor está trabajando activamente: refrescar marca de actividad
                                    last_activity_poll.store(now, Ordering::Relaxed);
                                } else {
                                    let last = last_activity_poll.load(Ordering::Relaxed);
                                    if last > 0 && now.saturating_sub(last) >= timeout {
                                        st.status = "stopped".to_string();
                                        st.is_healthy = false;
                                        st.pid = None;
                                        auto_stop = true;
                                    }
                                }
                            }
                        }
                        if let Some(msg) = pending_ok_log {
                            Self::push_log_to(&recent_logs_clone, &senders_clone, &seq_clone, &msg, Some(&log_file_poll));
                        }
                        // Avisos P20 (fuera del lock): respeta la config viva.
                        if let Some((title, body, tag)) = pending_notify {
                            let ncfg = cfg_poll.get().notifications;
                            let flag = match tag.as_str() {
                                "engine-ready" => ncfg.on_ready,
                                "engine-failure" => ncfg.on_failure,
                                _ => true,
                            };
                            if crate::notify::should_notify(ncfg.enabled, flag) {
                                let logf = log_file_poll.clone();
                                std::thread::spawn(move || {
                                    crate::notify::notify(&title, &body, &tag, |err| {
                                        crate::filelog::write_log_line(&logf, err);
                                    });
                                });
                            }
                        }
                    }
                }

                if auto_stop {
                    // Kill del árbol del proceso desde el poller (no podemos usar &self aquí).
                    let pid = {
                        let cl = child_clone.lock();
                        cl.as_ref().map(|c| c.id())
                    };
                    if let Some(pid) = pid {
                        let mut tk = Command::new("taskkill");
                        tk.args(["/F", "/T", "/PID", &pid.to_string()]);
                        tk.creation_flags(CREATE_NO_WINDOW);
                        let _ = tk.output();
                        let _ = child_clone.lock().take().map(|mut c| { let _ = c.kill(); let _ = c.wait(); });
                        Self::push_log_to(&recent_logs_clone, &senders_clone, &seq_clone,
                            "[LocalMind] Auto-stop: motor apagado por inactividad. VRAM y memoria liberadas.",
                            Some(&log_file_poll));
                        // Aviso P20: auto-stop.
                        let ncfg = cfg_poll.get().notifications;
                        if crate::notify::should_notify(ncfg.enabled, ncfg.on_autostop) {
                            let logf = log_file_poll.clone();
                            std::thread::spawn(move || {
                                crate::notify::notify(
                                    "Motor apagado por inactividad",
                                    "El motor se detuvo solo: llevaba 25 min sin uso. La VRAM quedó libre.",
                                    "engine-autostop",
                                    |err| crate::filelog::write_log_line(&logf, err),
                                );
                            });
                        }
                    }
                }
                thread::sleep(Duration::from_millis(500));
            }
        });

        Self {
            child,
            status,
            recent_logs,
            log_senders,
            running_poll,
            log_seq,
            last_activity: Arc::clone(&last_activity),
            start_instant,
            start_epoch,
            load_times,
            log_file,
            config,
            base_dir,
            bin_dir,
            models_dir,
        }
    }

    fn health_check(port: u16) -> bool {
        matches!(
            ureq::get(&format!("http://127.0.0.1:{}/health", port))
                .timeout(Duration::from_millis(600))
                .call(),
            Ok(resp) if resp.status() == 200
        )
    }

    fn check_slots_busy(port: u16) -> bool {
        let url = format!("http://127.0.0.1:{}/slots", port);
        if let Ok(resp) = ureq::get(&url).timeout(Duration::from_millis(400)).call() {
            if let Ok(slots) = resp.into_json::<Vec<serde_json::Value>>() {
                return slots.iter().any(|s| {
                    s.get("is_processing")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                });
            }
        }
        false
    }

    /// Encontrar un puerto libre empezando en `preferred` (hasta +50 intentos).
    fn find_free_port(preferred: u16) -> u16 {
        for offset in 0..50 {
            let p = preferred + offset;
            if std::net::TcpListener::bind(("127.0.0.1", p)).is_ok() {
                return p;
            }
        }
        preferred
    }

    /// Puerta de aceptación (D1): una completion real que prueba velocidad GPU.
    /// Nunca paniquea: todo fallo se devuelve como `Err(detalle)` en español.
    /// La puerta usa `temperature: 0` FIJO a propósito (determinista): aunque
    /// `[generation]` (P16) exponga otra temperatura para el chat, la puerta no
    /// la consume.
    fn run_acceptance_gate(port: u16) -> Result<(f64, u64), String> {
        let url = format!("http://127.0.0.1:{}/v1/chat/completions", port);
        let body = serde_json::json!({
            "messages": [{"role": "user", "content": "Count from 1 to 60. Nothing else."}],
            "max_tokens": 200,
            "temperature": 0,
            "stream": false,
        });
        let start = Instant::now();
        let resp = ureq::post(&url)
            .timeout(Duration::from_secs(120))
            .send_json(body);
        let elapsed_ms = start.elapsed().as_millis().max(1) as u64;
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                return Err(format!(
                    "El modelo no respondió durante la verificación de arranque ({}).",
                    e
                ))
            }
        };
        let text = resp.into_string().unwrap_or_default();
        if text.is_empty() {
            return Err("El modelo no respondió durante la verificación de arranque (respuesta vacía).".to_string());
        }
        match parse_usage(&text) {
            Some((_, completion)) => match acceptance_verdict(completion, elapsed_ms) {
                Ok(tps) => Ok((tps, completion)),
                Err(kind) if kind == "tokens" => Err(format!(
                    "El modelo no respondió durante la verificación de arranque (solo {} completion tokens).",
                    completion
                )),
                Err(_) => Err(format!(
                    "El motor respondió a {} t/s (velocidad de CPU): la GPU no se está usando.",
                    completion.saturating_mul(1000) / elapsed_ms.max(1)
                )),
            },
            None => Err("El modelo no respondió durante la verificación de arranque (sin usage válido).".to_string()),
        }
    }

    /// Ruta del registro de duraciones de carga (D4).
    fn load_times_path() -> PathBuf {
        std::env::var("APPDATA")
            .map(|p| Path::new(&p).join("LocalMind").join("load-times.json"))
            .unwrap_or_else(|_| PathBuf::from("load-times.json"))
    }

    fn model_size_gb(models_dir: &Path, model: &str) -> f64 {
        if model.trim().is_empty() {
            return 0.0;
        }
        // Misma resolución segura que el arranque (basename o `rel`); si no
        // resuelve, tamaño desconocido (0) en vez de pánico o escape.
        resolve_model_path(models_dir, model)
            .ok()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| (m.len() as f64) / (1024.0 * 1024.0 * 1024.0))
            .unwrap_or(0.0)
    }

    /// Push con archivo opcional (P30): el poller y los lectores del motor
    /// pasan su `log_file`; `None` solo en tests.
    fn push_log_to(
        logs: &Arc<RwLock<VecDeque<LogEvent>>>,
        senders: &Arc<Mutex<Vec<mpsc::Sender<String>>>>,
        seq: &Arc<AtomicU64>,
        msg: &str,
        file: Option<&Path>,
    ) {
        {
            let mut l = logs.write();
            if l.len() >= 250 {
                l.pop_front();
            }
            l.push_back(LogEvent { seq: seq.fetch_add(1, Ordering::Relaxed), line: msg.to_string() });
        }
        for tx in senders.lock().iter() {
            let _ = tx.send(msg.to_string());
        }
        if let Some(f) = file {
            crate::filelog::write_log_line(f, msg);
        }
    }

    /// Marcar actividad de generación (llamado por el server en cada chat request).
    pub fn touch_activity(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.last_activity.store(now, Ordering::Relaxed);
    }

    pub fn subscribe_logs(&self) -> mpsc::Receiver<String> {
        let (tx, rx) = mpsc::channel();
        self.log_senders.lock().push(tx);
        rx
    }

    /// Historial bufferizado con seq (para el Cliente SSE que conecta tarde).
    pub fn get_log_events(&self) -> Vec<LogEvent> {
        self.recent_logs.read().iter().cloned().collect()
    }

    pub fn get_recent_logs(&self) -> Vec<String> {
        self.recent_logs
            .read()
            .iter()
            .map(|l| l.line.clone())
            .collect()
    }

    pub fn log(&self, msg: &str) {
        let seq = self.log_seq.fetch_add(1, Ordering::Relaxed);
        {
            let mut logs = self.recent_logs.write();
            if logs.len() >= 250 {
                logs.pop_front();
            }
            logs.push_back(LogEvent {
                seq,
                line: msg.to_string(),
            });
        }

        let mut senders = self.log_senders.lock();
        senders.retain(|tx| tx.send(msg.to_string()).is_ok());
        // Archivo con rotación (P30): best-effort, nunca falla al llamador.
        crate::filelog::write_log_line(&self.log_file, msg);
    }

    pub fn get_status(&self) -> ServerStatus {
        let mut st = self.status.read().clone();
        let timeout = self.config.get().engine.idle_timeout_secs;
        if timeout > 0 && st.status == "running" {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let last = self.last_activity.load(Ordering::Relaxed);
            if last > 0 {
                let elapsed = now.saturating_sub(last);
                st.idle_remaining_secs = Some(timeout.saturating_sub(elapsed));
            }
        }
        // Progreso en vivo sin tocar el poller (el poller también lo actualiza).
        if st.status == "starting" {
            let elapsed = self
                .start_instant
                .lock()
                .as_ref()
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0);
            st.starting_for_secs = elapsed;
            let stored = self.load_times.lock().get(&load_key(&st.model, st.context)).copied();
            st.eta_secs = eta_secs(stored, Self::model_size_gb(&self.models_dir, &st.model));
        } else {
            st.starting_for_secs = 0;
            st.eta_secs = 0;
        }
        st
    }

    pub fn detect_hardware(&self) -> HardwareInfo {
        let cpu_cores = num_cpus();
        let cpu_name = std::env::var("PROCESSOR_IDENTIFIER")
            .unwrap_or_else(|_| format!("CPU x86_64 ({} hilos)", cpu_cores));

        let mut gpus = Vec::new();
        let llama_bin = self.bin_dir.join("llama-server.exe");
        if llama_bin.exists() {
            let mut cmd = Command::new(&llama_bin);
            cmd.arg("--list-devices");
            cmd.creation_flags(CREATE_NO_WINDOW);
            if let Ok(output) = cmd.output() {
                let text = String::from_utf8_lossy(&output.stdout);
                for line in text.lines() {
                    let line = line.trim();
                    if line.contains(':') && !line.starts_with("Available") {
                        let mut parts = line.splitn(2, ':');
                        let dev_id = parts.next().unwrap_or("").trim().to_string();
                        let rest = parts.next().unwrap_or("").trim();
                        let (name, vram) = if let Some(idx) = rest.rfind('(') {
                            let n = rest[..idx].trim().to_string();
                            let v = rest[idx + 1..].trim_end_matches(')').trim().to_string();
                            (n, v)
                        } else {
                            (rest.to_string(), "Desconocido".to_string())
                        };
                        gpus.push(GpuDevice {
                            id: dev_id,
                            name,
                            vram,
                        });
                    }
                }
            }
        }
        HardwareInfo {
            cpu_cores,
            cpu_name,
            gpus,
        }
    }

    pub fn import_model_from_path(&self, source_path: &std::path::Path) -> Result<String, String> {
        if !source_path.exists() {
            return Err(format!("Archivo no encontrado: {:?}", source_path));
        }
        let filename = source_path
            .file_name()
            .ok_or_else(|| "Nombre de archivo inválido".to_string())?
            .to_string_lossy()
            .to_string();
        if !filename.ends_with(".gguf") {
            return Err("El archivo debe ser un modelo con extensión .gguf".to_string());
        }
        let dest = self.models_dir.join(&filename);
        std::fs::copy(source_path, &dest)
            .map_err(|e| format!("Error al copiar modelo: {}", e))?;
        self.log(&format!("[LocalMind] Modelo importado: {}", filename));
        Ok(filename)
    }

    pub fn models_dir(&self) -> &std::path::Path {
        &self.models_dir
    }

    pub fn list_models(&self) -> Vec<ModelInfo> {
        let cfg = self.config.get();
        let aliases: Vec<String> = cfg.engine.aliases.clone();
        let mut models = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.models_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                if name.ends_with(".gguf") && !name.to_lowercase().contains("mmproj") {
                    let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let size_gb = (size_bytes as f64) / (1024.0 * 1024.0 * 1024.0);
                    models.push(ModelInfo {
                        filename: name.clone(),
                        name: name.trim_end_matches(".gguf").to_string(),
                        size_gb: (size_gb * 100.0).round() / 100.0,
                        path: path.to_string_lossy().to_string(),
                    });
                }
            }
        }

        // Ordenar: primero los modelos cuyo nombre contiene un alias configurado.
        models.sort_by(|a, b| {
            let arank = aliases
                .iter()
                .position(|al| a.filename.to_lowercase().contains(&al.to_lowercase()))
                .unwrap_or(usize::MAX);
            let brank = aliases
                .iter()
                .position(|al| b.filename.to_lowercase().contains(&al.to_lowercase()))
                .unwrap_or(usize::MAX);
            match arank.cmp(&brank) {
                std::cmp::Ordering::Equal => a.filename.cmp(&b.filename),
                o => o,
            }
        });

        models
    }

    /// Elegir mmproj: first match de la lista de candidatos (o ninguno si auto=false).
    fn find_mmproj(&self) -> Option<PathBuf> {
        let cfg = self.config.get();
        if !cfg.mmproj.auto {
            return None;
        }
        cfg.mmproj
            .files
            .iter()
            .map(|f| self.models_dir.join(f))
            .find(|p| p.exists())
    }

    pub fn start(&self, req: StartRequest) -> Result<u32, String> {
        self.stop();

        // Brief delay to allow Windows to clear any TIME_WAIT TCP sockets
        thread::sleep(Duration::from_millis(150));

        let cfg = self.config.get();
        let engine = &cfg.engine;

        // Perfil: request > config.last > config.profiles
        let profile: HardwareProfile = if let Some(id) = req.profile.as_ref().filter(|s| !s.is_empty()) {
            crate::profiles::resolve_profile(&cfg.profiles, id)
        } else if let Some(last) = cfg.last.profile.as_ref().filter(|s| !s.is_empty()) {
            crate::profiles::resolve_profile(&cfg.profiles, last).clone()
        } else {
            cfg.profiles.first().cloned().unwrap_or_else(|| {
                crate::profiles::resolve_profile(&cfg.profiles, crate::config::DEFAULT_PROFILE_ID).clone()
            })
        };
        // `profile_id` es siempre un id real (resuelto arriba): nunca se
        // persiste ni se expone el fantasma (D-1) aunque el request o el TOML
        // traigan un id desconocido.
        let profile_id = profile.id.clone();

        let models = self.list_models();
        let model_filename = req
            .model
            .clone()
            .filter(|m| !m.is_empty())
            .or_else(|| engine.aliases.iter().find_map(|al| {
                models.iter().find(|m| m.filename.to_lowercase().contains(&al.to_lowercase())).map(|m| m.filename.clone())
            }))
            .or_else(|| models.first().map(|m| m.filename.clone()))
            .or_else(|| cfg.last.model.as_ref().filter(|s| !s.is_empty()).cloned())
            .ok_or_else(|| "No se encontró ningún modelo .gguf en la carpeta models/".to_string())?;

        // Acepta el basename plano (histórico) o el `rel` de /api/models
        // (`sub/model.gguf`); rechaza `..`, absolutas y escapes de models/.
        let model_path = resolve_model_path(&self.models_dir, &model_filename)?;

        // Precedencia de contexto: request explícito > perfil explícito > última
        // sesión (solo si coincide perfil+modelo) > contexto del perfil resuelto.
        let req_profile_opt = req.profile.as_ref().filter(|s| !s.is_empty()).map(|s| s.as_str());
        let context = resolve_context(
            req.context,
            req_profile_opt,
            profile.context,
            cfg.last.profile.as_deref(),
            cfg.last.model.as_deref(),
            cfg.last.context,
            &profile.id,
            &model_filename,
        );
        let threads = req
            .threads
            .clone()
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.parse::<usize>()
                    .unwrap_or_else(|_| cfg.engine.threads.unwrap_or_else(num_cpus))
            })
            .unwrap_or_else(|| cfg.engine.threads.unwrap_or_else(num_cpus));
        let threads_batch = cfg
            .engine
            .threads_batch
            .unwrap_or(if cfg.engine.limit_threads_batch {
                threads.min(4) // prompt batch: el pico de CPU; limitado para no disparar PSU/VRAM
            } else {
                threads
            });
        let priority = req
            .priority
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| cfg.engine.priority.clone());
        let process_priority_class = match cfg.engine.process_priority.to_lowercase().as_str() {
            "low" => 0x00000040u32, // IDLE_PRIORITY_CLASS
            "normal" => 0x00000020, // NORMAL_PRIORITY_CLASS
            _ => 0x00004000,        // BELOW_NORMAL_PRIORITY_CLASS
        };

        let llama_bin = self.bin_dir.join("llama-server.exe");
        if !llama_bin.exists() {
            return Err(format!("No se encontró llama-server en {:?}", llama_bin));
        }

        // Puerto dinámico del motor: preferido desde config; si está ocupado, +1 hasta libre.
        let llama_port = Self::find_free_port(cfg.engine.llama_port);

        // Performance tuning seguro: uBatch fijado en 512 para evitar transitorios de energía
        let ubatch = "512";
        let mut cmd = Command::new(&llama_bin);
        cmd.current_dir(&self.base_dir);
        cmd.creation_flags(CREATE_NO_WINDOW | process_priority_class);

        cmd.args([
            "-m",
            model_path.to_str().unwrap(),
            "-ngl",
            "99",
            "-c",
            &context.to_string(),
            "-ctk",
            "q4_0",
            "-ctv",
            "q4_0",
            "-a",
            "localmind",
            "--reuse-port",
            "-t",
            &threads.to_string(),
            "-tb",
            &threads_batch.to_string(),
            "--prio",
            &priority,
            "-b",
            &cfg.engine.batch.to_string(),
            "-ub",
            ubatch,
            "--device",
            &cfg.engine.device,
            "--split-mode",
            "none",
            "--host",
            "127.0.0.1",
            "--port",
            &llama_port.to_string(),
            "-np",
            "1",
            "--poll",
            &cfg.engine.poll.to_string(),
            "--prio-batch",
            &cfg.engine.priority_batch,
        ]);
        if cfg.engine.flash_attention {
            cmd.args(["-fa", "on"]);
        }

        if cfg.engine.metrics {
            cmd.arg("--metrics");
        }

        if let Some(spec) = cfg.engine.speculation.as_ref().filter(|s| s.enabled) {
            cmd.args([
                "--spec-type",
                &spec.ty,
                "--spec-draft-n-max",
                &spec.draft_n_max.to_string(),
                "--spec-draft-p-split",
                &spec.draft_p_split.to_string(),
            ]);
        }

        if cfg.engine.reasoning_preserve {
            cmd.arg("--reasoning-preserve");
        }

        if profile.cache_ram > 0 {
            cmd.args(["--cache-ram", &profile.cache_ram.to_string()]);
        }

        if cfg.engine.cache_reuse > 0 {
            cmd.args(["--cache-reuse", &cfg.engine.cache_reuse.to_string()]);
        }

        for flag in &profile.extra_flags {
            cmd.arg(flag);
        }

        for flag in &cfg.engine.extra_flags {
            cmd.arg(flag);
        }

        if let Some(mm) = self.find_mmproj() {
            cmd.args(["--mmproj", mm.to_str().unwrap()]);
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        self.log("================================================================");
        self.log(&format!("[LocalMind] Cargando modelo: {}", model_filename));
        self.log(&format!("[LocalMind] Perfil: {}", profile.name));
        self.log(&format!(
            "[LocalMind] Contexto: {} tokens | Hilos: {} | uBatch: {} | Puerto: {} | cache-reuse: {}",
            context, threads, ubatch, llama_port, cfg.engine.cache_reuse
        ));
        self.log("================================================================");

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Error al iniciar llama-server: {}", e))?;
        let pid = child.id();

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let logs_clone = Arc::clone(&self.recent_logs);
        let senders_clone = Arc::clone(&self.log_senders);
        let seq_clone = Arc::clone(&self.log_seq);
        let file_clone = self.log_file.clone();
        if let Some(out) = stdout {
            let l_c = Arc::clone(&logs_clone);
            let s_c = Arc::clone(&senders_clone);
            let q_c = Arc::clone(&seq_clone);
            let f_c = file_clone.clone();
            thread::spawn(move || {
                let reader = BufReader::new(out);
                for line in reader.lines().map_while(Result::ok) {
                    let seq = q_c.fetch_add(1, Ordering::Relaxed);
                    {
                        let mut logs = l_c.write();
                        if logs.len() >= 250 {
                            logs.pop_front();
                        }
                        logs.push_back(LogEvent { seq, line: line.clone() });
                    }
                    let mut s = s_c.lock();
                    s.retain(|tx| tx.send(line.clone()).is_ok());
                    // Archivo con rotación (P30): best-effort.
                    crate::filelog::write_log_line(&f_c, &line);
                }
            });
        }

        if let Some(err) = stderr {
            let l_c = Arc::clone(&logs_clone);
            let s_c = Arc::clone(&senders_clone);
            let q_c = Arc::clone(&seq_clone);
            let f_c = file_clone.clone();
            thread::spawn(move || {
                let reader = BufReader::new(err);
                for line in reader.lines().map_while(Result::ok) {
                    let seq = q_c.fetch_add(1, Ordering::Relaxed);
                    {
                        let mut logs = l_c.write();
                        if logs.len() >= 250 {
                            logs.pop_front();
                        }
                        logs.push_back(LogEvent { seq, line: line.clone() });
                    }
                    let mut s = s_c.lock();
                    s.retain(|tx| tx.send(line.clone()).is_ok());
                    // Archivo con rotación (P30): best-effort.
                    crate::filelog::write_log_line(&f_c, &line);
                }
            });
        }

        *self.child.lock() = Some(child);

        {
            let mut st = self.status.write();
            st.status = "starting".to_string();
            st.is_healthy = false;
            st.pid = Some(pid);
            st.model = model_filename.clone();
            st.context = context;
            st.profile = profile_id.clone();
            st.port = llama_port;
            st.last_error = None;
            // Estado inicial de la puerta de aceptación (D1) y del progreso (D4).
            st.verifying = false;
            st.starting_for_secs = 0;
            let stored = self.load_times.lock().get(&load_key(&model_filename, context)).copied();
            st.eta_secs = eta_secs(stored, Self::model_size_gb(&self.models_dir, &model_filename));
            st.decode_tps = None;
            st.acceptance_ok = None;
            st.acceptance_error = None;
        }
        *self.start_instant.lock() = Some(Instant::now());
        // Nuevo arranque: invalida cualquier puerta de aceptación en curso (D1).
        self.start_epoch.fetch_add(1, Ordering::Relaxed);
        // Sembrar el timer de inactividad al arrancar (D-20): sin esto
        // `last_activity` queda en 0 y el auto-stop por inactividad no dispara nunca.
        self.touch_activity();

        // Persistir "última configuración usada" para el próximo arranque.
        self.config.update(|c| {
            c.last = crate::config::LastSettings {
                model: Some(model_filename.clone()),
                profile: Some(profile_id.clone()),
                context: Some(context),
            };
            // (modelo/perfil ya se movieron arriba; usar referencias)
        });
        let _ = self.config.save();

        Ok(pid)
    }

    pub fn stop(&self) {
        {
            let mut child_lock = self.child.lock();
            if let Some(mut c) = child_lock.take() {
                let pid = c.id();
                // Kill del árbol del proceso: tty hijos heredados (tokenizer, etc.)
                let mut tk = Command::new("taskkill");
                tk.args(["/F", "/T", "/PID", &pid.to_string()]);
                tk.creation_flags(CREATE_NO_WINDOW);
                let _ = tk.output();
                let _ = c.kill();
                let _ = c.wait();
            }
        }

        let mut st = self.status.write();
        st.status = "stopped".to_string();
        st.is_healthy = false;
        st.pid = None;
        st.verifying = false;
        st.starting_for_secs = 0;
        st.eta_secs = 0;
        *self.start_instant.lock() = None;
        // Invalida cualquier puerta de aceptación en curso (D1).
        self.start_epoch.fetch_add(1, Ordering::Relaxed);

        self.log("[LocalMind] Servidor detenido. 100% de VRAM y memoria liberada.");
    }
}

impl Drop for ProcessManager {
    fn drop(&mut self) {
        self.running_poll.store(false, Ordering::Relaxed);
        self.stop();
    }
}
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(6)
}

// ---------------------------------------------------------------------------
// Puerta de aceptación + crash legible + ETA (funciones puras, testeables)
// ---------------------------------------------------------------------------

/// Extraer `(prompt_tokens, completion_tokens)` del `usage` de una respuesta
/// OpenAI-compatible. Acepta JSON no-stream y el chunk final SSE (`data: {...}`).
fn parse_usage(body: &str) -> Option<(u64, u64)> {
    let trimmed = body.trim();
    // Camino SSE: última línea `data:` con JSON que traiga `usage`.
    if trimmed.contains("data:") {
        let mut found = None;
        for line in trimmed.lines() {
            let line = line.trim();
            let payload = line.strip_prefix("data:").map(str::trim).unwrap_or(line);
            if payload == "[DONE]" || payload.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) {
                if v.get("usage").is_some() {
                    found = Some(v);
                }
            }
        }
        if let Some(v) = found {
            return usage_from_value(&v);
        }
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .ok()
        .and_then(|v| usage_from_value(&v))
}

fn usage_from_value(v: &serde_json::Value) -> Option<(u64, u64)> {
    let usage = v.get("usage")?;
    let prompt = usage.get("prompt_tokens").and_then(|n| n.as_u64()).unwrap_or(0);
    let completion = usage
        .get("completion_tokens")
        .and_then(|n| n.as_u64())
        .unwrap_or(0);
    if prompt == 0 && completion == 0 {
        return None;
    }
    Some((prompt, completion))
}

/// Veredicto de la puerta de aceptación: `>= 20` completion tokens y `>= 3` t/s
/// (redondeo hacia abajo). Devuelve el t/s medido o el motivo (`tokens`/`tps`).
fn acceptance_verdict(tokens: u64, ms: u64) -> Result<f64, String> {
    if tokens < 20 {
        return Err("tokens".to_string());
    }
    let tps = (tokens as f64) * 1000.0 / (ms.max(1) as f64);
    if tokens.saturating_mul(1000) / ms.max(1) < 3 {
        return Err("tps".to_string());
    }
    Ok(tps)
}

/// ETA en segundos: duración guardada para la misma clave; en frío,
/// `6 s × tamaño del modelo en GB` (0 si se desconoce el tamaño).
fn eta_secs(stored: Option<u64>, size_gb: f64) -> u64 {
    if let Some(s) = stored {
        return s;
    }
    if size_gb > 0.0 {
        (size_gb * 6.0).round() as u64
    } else {
        0
    }
}

/// Clave del registro de duraciones: nombre del modelo + contexto pedido (D2:
/// nunca se reduce el contexto a espaldas del usuario, así que la clave es exacta).
fn load_key(model: &str, context: usize) -> String {
    format!("{}|{}", model, context)
}

/// Resolver el modelo pedido contra `models_dir`: acepta el basename plano
/// (histórico, lo que manda la UI) o el `rel` de /api/models (`sub/m.gguf`).
/// Rechaza `..`, rutas absolutas, prefijo de unidad y escapes de `models_dir`.
/// Sin pánicos: todo fallo es `Err` en español estilo `start()`.
fn resolve_model_path(models_dir: &Path, requested: &str) -> Result<PathBuf, String> {
    let req = requested.trim();
    if req.is_empty() {
        return Err("No se encontró ningún modelo .gguf en la carpeta models/".to_string());
    }
    let norm = req.replace('\\', "/");
    if norm.starts_with('/') || norm.starts_with('\\') {
        return Err(format!("Ruta de modelo no válida: {:?}", requested));
    }
    if norm.contains(':') {
        return Err(format!("Ruta de modelo no válida: {:?}", requested));
    }
    if norm.split('/').any(|seg| seg == "..") {
        return Err(format!("Ruta de modelo no válida: {:?}", requested));
    }
    let candidate = models_dir.join(norm.replace('/', &std::path::MAIN_SEPARATOR.to_string()));
    // Cinturón: aunque el join no debería escapar tras los filtros, verificarlo.
    let base = models_dir;
    if candidate != *base && !candidate.starts_with(base) {
        return Err(format!("Ruta de modelo no válida: {:?}", requested));
    }
    if !candidate.is_file() {
        return Err(format!("Archivo no encontrado: {:?}", candidate));
    }
    Ok(candidate)
}

/// Precedencia de contexto al arrancar (un perfil explícito nunca pierde contra
/// una sesión vieja): 1) `req_ctx` explícito; 2) perfil explícito en el request
/// → contexto de ESE perfil; 3) sin perfil explícito y última sesión con el
/// mismo perfil+modelo → contexto guardado; 4) contexto del perfil resuelto.
fn resolve_context(
    req_ctx: Option<usize>,
    req_profile: Option<&str>,
    profile_ctx: usize,
    last_profile: Option<&str>,
    last_model: Option<&str>,
    last_ctx: Option<usize>,
    resolved_profile: &str,
    model: &str,
) -> usize {
    if let Some(c) = req_ctx {
        return c;
    }
    if req_profile.filter(|s| !s.is_empty()).is_some() {
        return profile_ctx;
    }
    if let Some(saved) = last_ctx {
        let same_profile = last_profile.filter(|s| !s.is_empty()) == Some(resolved_profile);
        let same_model = match last_model.filter(|s| !s.is_empty()) {
            Some(m) => m == model,
            None => true,
        };
        if same_profile && same_model {
            return saved;
        }
    }
    profile_ctx
}

/// Resumen legible de un crash (D3): últimas ≤30 líneas no vacías, preferencia a
/// las que mencionan error/fallo/abort/bind/memoria/excepción; si ninguna
/// coincide, las últimas 5; todo truncado a 600 chars + código de salida.
fn crash_summary(lines: &[String], status: &str) -> String {
    let nonempty: Vec<&str> = lines
        .iter()
        .map(|l| l.as_str())
        .filter(|l| !l.trim().is_empty())
        .collect();
    if nonempty.is_empty() {
        return format!(
            "El servidor de IA se detuvo inesperadamente (código: {})",
            status
        );
    }
    let tail: Vec<&str> = nonempty
        .iter()
        .rev()
        .take(30)
        .rev()
        .map(|s| *s)
        .collect();
    let mut picked: Vec<&str> = tail
        .iter()
        .filter(|l| {
            let lower = l.to_lowercase();
            lower.contains("error")
                || lower.contains("failed")
                || lower.contains("abort")
                || lower.contains("couldn't bind")
                || lower.contains("out of")
                || lower.contains("exception")
        })
        .map(|s| *s)
        .collect();
    if picked.is_empty() {
        picked = tail.iter().rev().take(5).rev().map(|s| *s).collect();
    }
    let mut joined = picked.join(" | ");
    if joined.chars().count() > 600 {
        joined = joined.chars().take(600).collect();
    }
    format!("{} (código: {})", joined, status)
}

const MAX_LOAD_TIMES: usize = 64;

fn load_times_load(path: &Path) -> Result<HashMap<String, u64>, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if raw.trim().is_empty() {
        return Ok(HashMap::new());
    }
    let mut map: HashMap<String, u64> =
        serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    // Acotar lecturas de archivos ajenos/más grandes (D-8: buffer acotado).
    if map.len() > MAX_LOAD_TIMES {
        map = map.into_iter().take(MAX_LOAD_TIMES).collect();
    }
    Ok(map)
}

fn load_times_save(path: &Path, map: &HashMap<String, u64>) -> Result<(), String> {
    let mut capped: HashMap<String, u64> = HashMap::new();
    // Sin orden estable en HashMap: conservar una muestra acotada cualquiera.
    for (k, v) in map.iter().take(MAX_LOAD_TIMES) {
        capped.insert(k.clone(), *v);
    }
    let text = serde_json::to_string_pretty(&capped).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    // Escritura atómica: tmp + rename (D4).
    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_non_stream() {
        let body = r#"{"id":"x","usage":{"prompt_tokens":8,"completion_tokens":64,"total_tokens":72}}"#;
        assert_eq!(parse_usage(body), Some((8, 64)));
    }

    #[test]
    fn usage_sse_final_chunk() {
        let body = "data: {\"id\":\"x\"}\n\ndata: {\"usage\":{\"prompt_tokens\":8,\"completion_tokens\":61,\"total_tokens\":69}}\n\ndata: [DONE]\n";
        assert_eq!(parse_usage(body), Some((8, 61)));
    }

    #[test]
    fn verdict_exactly_3tps_passes() {
        // 30 tokens en 10000 ms = 3 t/s exactos → pasa.
        let tps = acceptance_verdict(30, 10_000).expect("debe pasar");
        assert!((tps - 3.0).abs() < 1e-9);
    }

    #[test]
    fn verdict_19_tokens_fails() {
        assert_eq!(acceptance_verdict(19, 1000), Err("tokens".to_string()));
    }

    #[test]
    fn verdict_20_tokens_slow_fails() {
        // 20 tokens en 7000 ms = 2 t/s (redondeo abajo) → falla por velocidad.
        assert_eq!(acceptance_verdict(20, 7000), Err("tps".to_string()));
    }

    #[test]
    fn eta_stored_and_cold() {
        assert_eq!(eta_secs(Some(95), 13.0), 95);
        // Frío: 6 s × 13 GB = 78 s.
        assert_eq!(eta_secs(None, 13.0), 78);
        assert_eq!(eta_secs(None, 0.0), 0);
    }

    #[test]
    fn crash_summary_prefers_errors_and_trims() {
        let mut lines = Vec::new();
        for i in 0..300 {
            lines.push(format!("info de arranque {}", i));
        }
        lines.push("CUDA error: out of memory allocating 4 GB".to_string());
        lines.push("llama_model_load failed: bad weights".to_string());
        let summary = crash_summary(&lines, "1");
        assert!(summary.contains("(código: 1)"));
        assert!(summary.contains("out of memory") || summary.contains("failed"));
        assert!(summary.chars().count() <= 600 + " (código: 1)".chars().count());
        // Solo caben ≤30 líneas de entrada en la selección.
        assert!(summary.matches("info de arranque").count() <= 28);
    }

    #[test]
    fn crash_summary_fallback_last5() {
        let lines: Vec<String> = (0..10).map(|i| format!("linea {}", i)).collect();
        let summary = crash_summary(&lines, "exit code: 0");
        assert!(summary.contains("linea 9"));
        assert!(summary.contains("(código: exit code: 0)"));
    }

    #[test]
    fn load_times_roundtrip_cap64_and_evict65th() {
        let dir = std::env::temp_dir().join(format!("lm-loadtimes-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("load-times.json");
        let mut map = HashMap::new();
        for i in 0..64 {
            map.insert(format!("m.gguf|{}", 1000 + i), i as u64);
        }
        load_times_save(&path, &map).expect("save 64");
        let back = load_times_load(&path).expect("load 64");
        assert_eq!(back.len(), 64);
        // Clave 65: insertar + guardar recorta a 64 (evicción por cap).
        let mut map65 = back.clone();
        map65.insert("nuevo.gguf|32768".to_string(), 42);
        assert_eq!(map65.len(), 65);
        load_times_save(&path, &map65).expect("save 65");
        let back65 = load_times_load(&path).expect("load 65");
        assert_eq!(back65.len(), 64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ctx_explicit_wins_over_everything() {
        // Request explícito manda aunque perfil y sesión digan otra cosa.
        assert_eq!(
            resolve_context(Some(65536), Some("velocidad"), 32768,
                Some("velocidad"), Some("m.gguf"), Some(131072), "velocidad", "m.gguf"),
            65536
        );
    }

    #[test]
    fn ctx_explicit_profile_beats_stale_last() {
        // Regresión auditada: {"profile":"velocidad"} con last=131072 → 32768.
        assert_eq!(
            resolve_context(None, Some("velocidad"), 32768,
                Some("libros"), Some("m.gguf"), Some(131072), "velocidad", "m.gguf"),
            32768
        );
    }

    #[test]
    fn ctx_no_profile_matching_last_uses_last() {
        // Sin perfil en el request y misma sesión (perfil+modelo) → contexto guardado.
        assert_eq!(
            resolve_context(None, None, 32768,
                Some("velocidad"), Some("m.gguf"), Some(32768), "velocidad", "m.gguf"),
            32768
        );
    }

    #[test]
    fn ctx_no_profile_no_last_uses_default() {
        // Sin perfil ni sesión → contexto del perfil resuelto por defecto.
        assert_eq!(
            resolve_context(None, None, 32768, None, None, None, "velocidad", "m.gguf"),
            32768
        );
    }

    #[test]
    fn ctx_explicit_profile_same_as_last_still_profile() {
        // Perfil explícito igual al de la última sesión, sin contexto → perfil manda.
        assert_eq!(
            resolve_context(None, Some("velocidad"), 32768,
                Some("velocidad"), Some("m.gguf"), Some(131072), "velocidad", "m.gguf"),
            32768
        );
    }

    fn scratch_models(tag: &str) -> PathBuf {
        // Directorio único por test: los tests corren en paralelo en el mismo
        // proceso y compartirlo provocaba borrados cruzados (remove_dir_all).
        let dir = std::env::temp_dir().join(format!("lm-resolve-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(dir.join("sub"));
        let _ = std::fs::write(dir.join("plano.gguf"), b"x");
        let _ = std::fs::write(dir.join("sub").join("x.gguf"), b"y");
        dir
    }

    #[test]
    fn model_basename_resolves() {
        let dir = scratch_models("base");
        let p = resolve_model_path(&dir, "plano.gguf").expect("basename debe resolver");
        assert_eq!(p, dir.join("plano.gguf"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_rel_nested_resolves() {
        let dir = scratch_models("nested");
        let p = resolve_model_path(&dir, "sub/x.gguf").expect("rel anidado debe resolver");
        assert_eq!(p, dir.join("sub").join("x.gguf"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_traversal_rejected() {
        let dir = scratch_models("trav");
        assert!(resolve_model_path(&dir, "../x.gguf").is_err());
        assert!(resolve_model_path(&dir, "sub/../../x.gguf").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_absolute_rejected() {
        let dir = scratch_models("abs");
        assert!(resolve_model_path(&dir, "C:/abs/x.gguf").is_err());
        assert!(resolve_model_path(&dir, "/abs/x.gguf").is_err());
        assert!(resolve_model_path(&dir, "C:\\abs\\x.gguf").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_unknown_returns_not_found() {
        let dir = scratch_models("unknown");
        let err = resolve_model_path(&dir, "noexiste.gguf").unwrap_err();
        assert!(err.contains("Archivo no encontrado"), "inesperado: {}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_profile_id_is_real() {
        // D-1: el estado fresco nunca expone el fantasma `turbo`.
        let profiles = crate::config::built_in_profiles();
        let ids: Vec<&str> = profiles.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&crate::config::DEFAULT_PROFILE_ID));
        assert_ne!(crate::config::DEFAULT_PROFILE_ID, "turbo");
    }

    #[test]
    fn unknown_last_profile_falls_back_to_default() {
        // D-1: un id desconocido en el TOML cae al default real, no al fantasma.
        let profiles = crate::config::built_in_profiles();
        let resolved = crate::profiles::resolve_profile(&profiles, "turbo");
        assert_eq!(resolved.id, profiles[0].id);
        assert_eq!(profiles[0].id, crate::config::DEFAULT_PROFILE_ID);
    }
}
