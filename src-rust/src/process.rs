use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use crate::profiles::find_profile;

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
    pub last_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StartRequest {
    pub model: Option<String>,
    pub profile: Option<String>,
    pub context: Option<usize>,
    pub threads: Option<String>,
    pub priority: Option<String>,
}

pub struct ProcessManager {
    child: Arc<Mutex<Option<Child>>>,
    status: Arc<RwLock<ServerStatus>>,
    recent_logs: Arc<RwLock<VecDeque<String>>>,
    log_senders: Arc<Mutex<Vec<mpsc::Sender<String>>>>,
    running_poll: Arc<AtomicBool>,
    base_dir: PathBuf,
    bin_dir: PathBuf,
    models_dir: PathBuf,
}

impl ProcessManager {
    pub fn new(base_dir: PathBuf) -> Self {
        let bin_dir = base_dir.join("bin");
        let models_dir = base_dir.join("models");

        let status = Arc::new(RwLock::new(ServerStatus {
            status: "stopped".to_string(),
            is_healthy: false,
            pid: None,
            model: "".to_string(),
            context: 32768,
            profile: "turbo".to_string(),
            last_error: None,
        }));

        let recent_logs: Arc<RwLock<VecDeque<String>>> = Arc::new(RwLock::new(VecDeque::new()));
        let log_senders = Arc::new(Mutex::new(Vec::new()));
        let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let running_poll = Arc::new(AtomicBool::new(true));

        // Background poller: monitors health AND child process liveness
        let status_clone = Arc::clone(&status);
        let child_clone = Arc::clone(&child);
        let logs_for_err = Arc::clone(&recent_logs);
        let poll_flag = Arc::clone(&running_poll);

        thread::spawn(move || {
            while poll_flag.load(Ordering::Relaxed) {
                // 1. Check if child process has exited or crashed
                let exited = {
                    let mut cl = child_clone.lock();
                    if let Some(c) = cl.as_mut() {
                        match c.try_wait() {
                            Ok(Some(exit_status)) => Some(exit_status),
                            Ok(None) => None,
                            Err(_) => None,
                        }
                    } else {
                        None
                    }
                };

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
                                l.contains("couldn't bind")
                                    || l.contains("error")
                                    || l.contains("failed")
                                    || l.contains("Exception")
                                    || l.contains("abort")
                            })
                            .cloned()
                            .unwrap_or_else(|| {
                                format!("El servidor de IA se detuvo inesperadamente (código: {})", exit_status)
                            });
                        st.last_error = Some(err_line);
                    }
                    *child_clone.lock() = None;
                } else {
                    // 2. Poll health endpoint if still starting or running
                    let current_status = {
                        let st = status_clone.read();
                        st.status.clone()
                    };

                    if current_status == "starting" || current_status == "running" {
                        let is_ok = match ureq::get("http://127.0.0.1:8080/health")
                            .timeout(Duration::from_millis(600))
                            .call()
                        {
                            Ok(resp) => resp.status() == 200,
                            Err(_) => false,
                        };

                        let mut st = status_clone.write();
                        st.is_healthy = is_ok;
                        if is_ok && st.status == "starting" {
                            st.status = "running".to_string();
                            st.last_error = None;
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
            base_dir,
            bin_dir,
            models_dir,
        }
    }

    pub fn log(&self, msg: &str) {
        let mut logs = self.recent_logs.write();
        if logs.len() >= 250 {
            logs.pop_front();
        }
        logs.push_back(msg.to_string());

        let mut senders = self.log_senders.lock();
        senders.retain(|tx| tx.send(msg.to_string()).is_ok());
    }

    pub fn subscribe_logs(&self) -> mpsc::Receiver<String> {
        let (tx, rx) = mpsc::channel();
        self.log_senders.lock().push(tx);
        rx
    }

    pub fn get_recent_logs(&self) -> Vec<String> {
        self.recent_logs.read().iter().cloned().collect()
    }

    pub fn get_status(&self) -> ServerStatus {
        self.status.read().clone()
    }

    pub fn list_models(&self) -> Vec<ModelInfo> {
        let mut models = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.models_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
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
        }

        models.sort_by(|a, b| {
            let a_qwen = a.filename.to_lowercase().contains("qwen3.8");
            let b_qwen = b.filename.to_lowercase().contains("qwen3.8");
            match (a_qwen, b_qwen) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.filename.cmp(&b.filename),
            }
        });

        models
    }

    pub fn start(&self, req: StartRequest) -> Result<u32, String> {
        self.stop();

        // Brief delay to allow Windows to clear any TIME_WAIT TCP sockets
        thread::sleep(Duration::from_millis(150));

        let profile_id = req.profile.unwrap_or_else(|| "turbo".to_string());
        let profile = find_profile(&profile_id);

        let models = self.list_models();
        let model_filename = req
            .model
            .filter(|m| !m.is_empty())
            .or_else(|| models.first().map(|m| m.filename.clone()))
            .ok_or_else(|| "No se encontró ningún modelo .gguf en la carpeta models/".to_string())?;

        let model_path = self.models_dir.join(&model_filename);
        if !model_path.exists() {
            return Err(format!("Archivo no encontrado: {:?}", model_path));
        }

        let context = req.context.unwrap_or(profile.context);
        let threads = req.threads.unwrap_or_else(|| "6".to_string());
        let priority = req.priority.unwrap_or_else(|| "2".to_string());

        let llama_bin = self.bin_dir.join("llama-server.exe");
        if !llama_bin.exists() {
            return Err(format!("No se encontró llama-server en {:?}", llama_bin));
        }

        let mmproj_candidates = [
            self.models_dir.join("mmproj-BF16.gguf"),
            self.models_dir.join("mmproj-F16.gguf"),
            self.models_dir.join("Ternary-Bonsai-2-27B-mmproj-Q8_0.gguf"),
        ];
        let mmproj_file = mmproj_candidates.into_iter().find(|p| p.exists());

        // Performance tuning: Use larger physical ubatch for pure VRAM profiles
        let ubatch = if context <= 65536 { "1024" } else { "512" };

        let mut cmd = Command::new(&llama_bin);
        cmd.current_dir(&self.base_dir);
        cmd.creation_flags(CREATE_NO_WINDOW);

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
            "-fa",
            "on",
            "-a",
            "localmind,qwen3.8-27b,bonsai-2-27b",
            "--reuse-port",
            "--spec-type",
            "draft-mtp",
            "--spec-draft-n-max",
            "2",
            "--spec-draft-p-split",
            "0.1",
            "-t",
            &threads,
            "-tb",
            &threads,
            "--prio",
            &priority,
            "--prio-batch",
            &priority,
            "-b",
            "2048",
            "-ub",
            ubatch,
            "--device",
            "Vulkan0",
            "--split-mode",
            "none",
            "--host",
            "127.0.0.1",
            "--port",
            "8080",
            "-np",
            "1",
            "--reasoning-preserve",
            "--metrics",
        ]);

        if profile.cache_ram > 0 {
            cmd.args(["--cache-ram", &profile.cache_ram.to_string()]);
        }

        for flag in &profile.extra_flags {
            cmd.arg(flag);
        }

        if let Some(mm) = &mmproj_file {
            cmd.args(["--mmproj", mm.to_str().unwrap()]);
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        self.log("================================================================");
        self.log(&format!("[LocalMind] Cargando modelo: {}", model_filename));
        self.log(&format!("[LocalMind] Perfil: {}", profile.name));
        self.log(&format!(
            "[LocalMind] Contexto: {} tokens | Hilos: {} | uBatch: {} | MTP n=2",
            context, threads, ubatch
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

        if let Some(out) = stdout {
            let l_c = Arc::clone(&logs_clone);
            let s_c = Arc::clone(&senders_clone);
            thread::spawn(move || {
                let reader = BufReader::new(out);
                for line in reader.lines().flatten() {
                    let mut logs = l_c.write();
                    if logs.len() >= 250 {
                        logs.pop_front();
                    }
                    logs.push_back(line.clone());
                    let mut s = s_c.lock();
                    s.retain(|tx| tx.send(line.clone()).is_ok());
                }
            });
        }

        if let Some(err) = stderr {
            let l_c = Arc::clone(&logs_clone);
            let s_c = Arc::clone(&senders_clone);
            thread::spawn(move || {
                let reader = BufReader::new(err);
                for line in reader.lines().flatten() {
                    let mut logs = l_c.write();
                    if logs.len() >= 250 {
                        logs.pop_front();
                    }
                    logs.push_back(line.clone());
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
            st.model = model_filename;
            st.context = context;
            st.profile = profile_id;
            st.last_error = None;
        }

        Ok(pid)
    }

    pub fn stop(&self) {
        {
            let mut child_lock = self.child.lock();
            if let Some(mut c) = child_lock.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }

        // Safety taskkill
        let mut kill_cmd = Command::new("taskkill");
        kill_cmd.args(["/F", "/IM", "llama-server.exe"]);
        kill_cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = kill_cmd.output();

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
