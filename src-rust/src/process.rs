use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

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
            profile: "turbo".to_string(),
            port: cfg.engine.llama_port,
            idle_remaining_secs: None,
            last_error: None,
        }));

        let recent_logs: Arc<RwLock<VecDeque<LogEvent>>> = Arc::new(RwLock::new(VecDeque::new()));
        let log_senders: Arc<Mutex<Vec<mpsc::Sender<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let running_poll = Arc::new(AtomicBool::new(true));
        let log_seq = Arc::new(AtomicU64::new(0));
        let last_activity = Arc::new(AtomicU64::new(0));

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

        thread::spawn(move || {
            let mut consecutive_failures = 0u32;
            while poll_flag.load(Ordering::Relaxed) {
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
                    let mut st = status_clone.write();
                    if st.status == "starting" || st.status == "running" {
                        st.status = "error".to_string();
                        st.is_healthy = false;
                        st.pid = None;
                        

                        let logs = logs_for_err.read();
                        let err_line = logs
                            .iter()
                            .rev()
                            .find(|l| {
                                l.line.contains("couldn't bind")
                                    || l.line.contains("error")
                                    || l.line.contains("failed")
                                    || l.line.contains("Exception")
                                    || l.line.contains("abort")
                            })
                            .map(|l| l.line.clone())
                            .unwrap_or_else(|| {
                                format!(
                                    "El servidor de IA se detuvo inesperadamente (código: {})",
                                    exit_status
                                )
                            });
                        st.last_error = Some(err_line);
                    }
                    *child_clone.lock() = None;
                    consecutive_failures = 0;
                } else {
                    // 2. Poll health endpoint if still starting or running
                    let (current_status, port) = {
                        let st = status_clone.read();
                        (st.status.clone(), st.port)
                    };

                    if current_status == "starting" || current_status == "running" {
                        let is_ok = Self::health_check(port);
                        let mut st = status_clone.write();
                        st.is_healthy = is_ok;
                        if is_ok && st.status == "starting" {
                            st.status = "running".to_string();
                            st.last_error = None;
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
                        let mut logs = recent_logs_clone.write();
                        if logs.len() >= 250 { logs.pop_front(); }
                        logs.push_back(LogEvent { seq: seq_clone.fetch_add(1, Ordering::Relaxed), line: "[LocalMind] Auto-stop: motor apagado por inactividad. VRAM y memoria liberadas.".to_string() });
                        for tx in senders_clone.lock().iter() { let _ = tx.send("[LocalMind] Auto-stop: motor apagado por inactividad. VRAM y memoria liberadas.".to_string()); }
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

    /// Marcar actividad de generación (llamado por el server en cada chat request).
    pub fn touch_activity(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.last_activity.store(now, Ordering::Relaxed);
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
                crate::profiles::resolve_profile(&cfg.profiles, "turbo").clone()
            })
        };
        let profile_id = req.profile.clone().unwrap_or_else(|| {
            cfg.last
                .profile
                .clone()
                .unwrap_or_else(|| profile.id.clone())
        });

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

        let model_path = self.models_dir.join(&model_filename);
        if !model_path.exists() {
            return Err(format!("Archivo no encontrado: {:?}", model_path));
        }

        let context = req.context.unwrap_or_else(|| cfg.last.context.unwrap_or(profile.context));
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
            &priority,
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
            "[LocalMind] Contexto: {} tokens | Hilos: {} | uBatch: {} | Puerto: {}",
            context, threads, ubatch, llama_port
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

        if let Some(out) = stdout {
            let l_c = Arc::clone(&logs_clone);
            let s_c = Arc::clone(&senders_clone);
            let q_c = Arc::clone(&seq_clone);
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
                }
            });
        }

        if let Some(err) = stderr {
            let l_c = Arc::clone(&logs_clone);
            let s_c = Arc::clone(&senders_clone);
            let q_c = Arc::clone(&seq_clone);
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
        }

        // Sembrar el timer de inactividad al arrancar el motor.
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
