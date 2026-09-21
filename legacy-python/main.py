import os
import sys
from pathlib import Path

# Safe stream redirection for GUI mode (pythonw.exe)
if sys.stdout is None:
    sys.stdout = open(os.devnull, "w", encoding="utf-8")
if sys.stderr is None:
    sys.stderr = open(os.devnull, "w", encoding="utf-8")

import json
import time
import queue
import subprocess
import threading
import urllib.request
import webbrowser
import webview

BASE_DIR = Path(__file__).resolve().parent
BIN_DIR = BASE_DIR / "bin"
MODELS_DIR = BASE_DIR / "models"
LLAMA_SERVER = BIN_DIR / "llama-server.exe"
UI_FILE = BASE_DIR / "ui.html"
ICON_PATH = BASE_DIR / "localmind.ico"

CREATE_NO_WINDOW = 0x08000000 if sys.platform == "win32" else 0
HARDWARE_PROFILES = {
    "turbo": {
        "id": "turbo",
        "name": "Turbo VRAM (32K · Máxima Velocidad T/S)",
        "description": "100% en VRAM (RX 6800 XT). Máximo rendimiento y mínima latencia.",
        "context": 32768,
        "cache_ram": 0,
        "extra_flags": ["--cache-reuse", "256"]
    },
    "balanced": {
        "id": "balanced",
        "name": "Equilibrado Pro (64K · Documentos y Código)",
        "description": "Pesos en VRAM + KV Cache extendido en VRAM/RAM DDR5.",
        "context": 65536,
        "cache_ram": 4096,
        "extra_flags": ["--cache-reuse", "256"]
    },
    "deep": {
        "id": "deep",
        "name": "Extendido 128K (Libros / Repositorios)",
        "description": "128K tokens usando VRAM + RAM compartida optimizada.",
        "context": 131072,
        "cache_ram": 6144,
        "extra_flags": ["--cache-reuse", "256"]
    },
    "ultra": {
        "id": "ultra",
        "name": "Límite Hardware 262K (Máximo Contexto Físico)",
        "description": "Ventana nativa máxima (262,144 tokens) combinando 100% VRAM + RAM DDR5 segura.",
        "context": 262144,
        "cache_ram": 6144,
        "extra_flags": ["-kvu", "--cache-reuse", "512"]
    }
}

class StudioApi:
    def __init__(self):
        self.proc = None
        self.window = None
        self.is_running = False
        self.last_error = None
        self.active_config = {
            "model": "",
            "context": 131072,
            "threads": 6,
            "spec": "none",
            "pid": None,
            "status": "stopped"
        }
        self.cached_status = {
            "status": "stopped",
            "is_healthy": False,
            "pid": None,
            "model": "",
            "context": 131072,
            "stats": {},
            "last_error": None
        }
        self.lock = threading.RLock()
        self.log_queue = queue.Queue()
        self.recent_logs = []
        self.running_poll = True

        # Background status polling worker
        self.poller_thread = threading.Thread(target=self._status_poll_worker, daemon=True)
        self.poller_thread.start()

        # Background log batch dispatch worker
        self.log_dispatcher_thread = threading.Thread(target=self._log_dispatch_worker, daemon=True)
        self.log_dispatcher_thread.start()

    def set_window(self, window):
        self.window = window

    def log(self, text: str):
        self.log_queue.put(text)
        with self.lock:
            self.recent_logs.append(text)
            if len(self.recent_logs) > 60:
                self.recent_logs.pop(0)

    def _log_dispatch_worker(self):
        """Batches log lines to avoid freezing the WebView2 IPC channel"""
        while True:
            batch = []
            try:
                first = self.log_queue.get(timeout=0.15)
                batch.append(first)
                while len(batch) < 30:
                    try:
                        item = self.log_queue.get_nowait()
                        batch.append(item)
                    except queue.Empty:
                        break
            except queue.Empty:
                continue

            if batch and self.window:
                try:
                    payload = json.dumps(batch)
                    self.window.evaluate_js(f"window.onLogsReceived && window.onLogsReceived({payload});")
                except Exception:
                    pass

    def _status_poll_worker(self):
        """Asynchronous poller so get_status() never blocks the UI thread"""
        while self.running_poll:
            is_healthy = False
            stats = {}

            if self.is_running or self.active_config["status"] in ("starting", "running"):
                try:
                    req = urllib.request.Request("http://127.0.0.1:8080/health")
                    with urllib.request.urlopen(req, timeout=0.35) as resp:
                        if resp.status == 200:
                            is_healthy = True
                except Exception:
                    is_healthy = False

                if is_healthy:
                    try:
                        req_slots = urllib.request.Request("http://127.0.0.1:8080/slots")
                        with urllib.request.urlopen(req_slots, timeout=0.35) as resp:
                            data = json.loads(resp.read().decode())
                            if data and isinstance(data, list) and len(data) > 0:
                                slot = data[0]
                                stats["is_processing"] = slot.get("is_processing", False)
                                stats["n_prompt_tokens"] = slot.get("n_prompt_tokens", 0)
                                stats["n_decoded"] = slot.get("n_decoded", 0)
                    except Exception:
                        pass

            with self.lock:
                # If health check passed, state is definitely running
                if is_healthy:
                    self.active_config["status"] = "running"
                    self.last_error = None

                current_status = self.active_config["status"]

                self.cached_status = {
                    "status": current_status,
                    "is_healthy": is_healthy,
                    "pid": self.active_config["pid"],
                    "model": self.active_config["model"],
                    "context": self.active_config["context"],
                    "stats": stats,
                    "last_error": self.last_error
                }

            time.sleep(1.0)

    def get_models(self):
        models = []
        if MODELS_DIR.exists():
            for f in MODELS_DIR.glob("*.gguf"):
                if "mmproj" not in f.name.lower():
                    size_gb = round(f.stat().st_size / (1024**3), 2)
                    models.append({
                        "filename": f.name,
                        "name": f.name.replace(".gguf", ""),
                        "size_gb": size_gb,
                        "path": str(f)
                    })
        return models
    def get_profiles(self):
        return list(HARDWARE_PROFILES.values())


    def get_status(self):
        with self.lock:
            return dict(self.cached_status)

    def start_server(self, config: dict):
        with self.lock:
            if self.proc and self.proc.poll() is None:
                return {"error": "El servidor ya está en ejecución"}

            self._force_cleanup_locked()

            default_model = "Qwen3.8-27B-IQ4_XS_4BPW.gguf"
            if not (MODELS_DIR / default_model).exists():
                # Fallback to any existing model
                models = [f.name for f in MODELS_DIR.glob("*.gguf") if "mmproj" not in f.name.lower()]
                default_model = models[0] if models else "Qwen3.8-27B-IQ4_XS_4BPW.gguf"

            model_name = config.get("model") or default_model
            profile_id = config.get("profile", "turbo")
            profile = HARDWARE_PROFILES.get(profile_id, HARDWARE_PROFILES["turbo"])

            context_size = int(config.get("context") or profile["context"])
            threads = str(config.get("threads", 6))
            priority = str(config.get("priority", "2"))
            model_file = MODELS_DIR / model_name
            if not model_file.exists():
                err_msg = f"Archivo de modelo no encontrado: {model_file}"
                self.log(f"[Error] {err_msg}")
                self.last_error = err_msg
                return {"error": err_msg}

            # Detect mmproj if available
            mmproj_candidates = [
                MODELS_DIR / "mmproj-BF16.gguf",
                MODELS_DIR / "mmproj-F16.gguf",
                MODELS_DIR / "Ternary-Bonsai-2-27B-mmproj-Q8_0.gguf"
            ]
            mmproj_file = None
            for c in mmproj_candidates:
                if c.exists():
                    mmproj_file = c
                    break

            cmd = [
                str(LLAMA_SERVER),
                "-m", str(model_file),
                "-ngl", "99",
                "-c", str(context_size),
                "-ctk", "q4_0",
                "-ctv", "q4_0",
                "-fa", "on",
                "-a", "localmind,qwen3.8-27b,bonsai-2-27b",
                "--spec-type", "draft-mtp",
                "--spec-draft-n-max", "2",
                "--spec-draft-p-split", "0.1",
                "-t", threads,
                "-tb", threads,
                "--prio", priority,
                "--prio-batch", priority,
                "-b", "2048",
                "-ub", "512",
                "--host", "127.0.0.1",
                "--port", "8080",
                "-np", "1",
                "--device", "Vulkan0",
                "--split-mode", "none",
                "--reasoning-preserve",
                "--metrics"
            ]

            if profile.get("cache_ram", 0) > 0:
                cmd.extend(["--cache-ram", str(profile["cache_ram"])])

            if profile.get("extra_flags"):
                cmd.extend(profile["extra_flags"])

            if mmproj_file and mmproj_file.exists():
                cmd.extend(["--mmproj", str(mmproj_file)])
            self.last_error = None
            self.recent_logs.clear()

            self.log("=" * 64)
            self.log(f"[LocalMind] Cargando modelo: {model_name}")
            self.log(f"[LocalMind] Perfil: {profile['name']}")
            self.log(f"[LocalMind] Contexto: {context_size} tokens | Hilos Zen 4: {threads} | MTP: n-max=2")
            self.log("=" * 64)

            try:
                self.proc = subprocess.Popen(
                    cmd,
                    cwd=str(BASE_DIR),
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    creationflags=CREATE_NO_WINDOW,
                    bufsize=1
                )
                self.active_config["model"] = model_name
                self.active_config["context"] = context_size
                self.active_config["status"] = "starting"
                self.active_config["pid"] = self.proc.pid
                self.is_running = True

                self.cached_status["status"] = "starting"
                self.cached_status["is_healthy"] = False
                self.cached_status["pid"] = self.proc.pid
                self.cached_status["last_error"] = None

                t = threading.Thread(target=self._log_stream_worker, daemon=True)
                t.start()
            except Exception as e:
                err_msg = str(e)
                self.log(f"[Error de proceso] {err_msg}")
                self.active_config["status"] = "error"
                self.last_error = err_msg
                self.cached_status["status"] = "error"
                self.cached_status["last_error"] = err_msg
                return {"error": err_msg}

        return {"status": "starting", "pid": self.active_config["pid"]}

    def _log_stream_worker(self):
        try:
            for line in iter(self.proc.stdout.readline, b""):
                if not line:
                    break
                try:
                    decoded = line.decode("utf-8", errors="replace").rstrip("\r\n")
                except Exception:
                    decoded = str(line)
                self.log(decoded)
        except Exception:
            pass
        finally:
            with self.lock:
                self.is_running = False
                exit_code = self.proc.poll() if self.proc else None

                # Find any error in recent logs if exited with failure
                if exit_code not in (None, 0):
                    found_err = None
                    for l in reversed(self.recent_logs):
                        if any(k in l.lower() for k in ("error", "failed", "exception", "cannot", "abort")):
                            found_err = l
                            break
                    self.last_error = found_err or f"El servidor se detuvo con código {exit_code}."
                    self.active_config["status"] = "error"
                    self.cached_status["status"] = "error"
                    self.cached_status["last_error"] = self.last_error
                    self.log(f"[LocalMind Error] {self.last_error}")
                else:
                    self.active_config["status"] = "stopped"
                    self.cached_status["status"] = "stopped"
                    self.cached_status["last_error"] = None
                    self.log("[LocalMind] Servidor detenido. Memoria VRAM y RAM liberada.")

                self.active_config["pid"] = None
                self.cached_status["is_healthy"] = False
                self.cached_status["pid"] = None

    def stop_server(self):
        self.log("[LocalMind] Deteniendo servidor y purgando tensores...")
        with self.lock:
            self._force_cleanup_locked()
        return {"status": "stopped"}

    def _force_cleanup_locked(self):
        """Must be called while holding self.lock"""
        pid = self.proc.pid if self.proc else None
        if pid:
            try:
                subprocess.run(
                    ["taskkill", "/F", "/T", "/PID", str(pid)],
                    creationflags=CREATE_NO_WINDOW,
                    capture_output=True,
                    text=True
                )
            except Exception:
                pass

        subprocess.run(
            ["taskkill", "/F", "/IM", "llama-server.exe"],
            creationflags=CREATE_NO_WINDOW,
            capture_output=True,
            text=True
        )

        self.proc = None
        self.is_running = False
        self.active_config["status"] = "stopped"
        self.active_config["pid"] = None

        self.cached_status["status"] = "stopped"
        self.cached_status["is_healthy"] = False
        self.cached_status["pid"] = None

    def _force_cleanup(self):
        with self.lock:
            self._force_cleanup_locked()

    def launch_omp(self):
        """Explicitly launches terminal for user interaction with OMP"""
        try:
            wt_path = Path(os.environ.get("LOCALAPPDATA", "")) / "Microsoft/WindowsApps/wt.exe"
            cmd = 'omp --model bonsai/bonsai-2-27b'
            if wt_path.exists():
                subprocess.Popen([str(wt_path), "-w", "0", "new-tab", "cmd", "/k", cmd])
            else:
                subprocess.Popen(["cmd.exe", "/c", f"start {cmd}"], shell=True)
            self.log("[LocalMind] Terminal OMP lanzada con modelo local (27B).")
            return {"status": "ok"}
        except Exception as e:
            self.log(f"[Error al lanzar OMP] {e}")
            return {"error": str(e)}

    def open_browser_chat(self):
        try:
            webbrowser.open("http://127.0.0.1:8080")
            self.log("[LocalMind] Interfaz oficial abierta en navegador (http://127.0.0.1:8080).")
            return {"status": "ok"}
        except Exception as e:
            return {"error": str(e)}

def on_closing(api):
    api.running_poll = False
    api.stop_server()

def main():
    api = StudioApi()
    api._force_cleanup()

    window = webview.create_window(
        title="LocalMind Studio",
        url=str(UI_FILE),
        js_api=api,
        width=1160,
        height=820,
        min_size=(960, 680),
        background_color="#000000"
    )
    api.set_window(window)
    window.events.closed += lambda: on_closing(api)

    icon_arg = str(ICON_PATH) if ICON_PATH.exists() else None
    webview.start(gui="edgechromium", debug=False, icon=icon_arg)

if __name__ == "__main__":
    main()
