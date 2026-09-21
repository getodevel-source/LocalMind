import os
import sys
import json
import time
import subprocess
import threading
import asyncio
from pathlib import Path
from typing import Optional, List
from fastapi import FastAPI, WebSocket, WebSocketDisconnect
from fastapi.responses import HTMLResponse, JSONResponse
from fastapi.staticfiles import StaticFiles
import uvicorn

BASE_DIR = Path(__file__).resolve().parent
BIN_DIR = BASE_DIR / "bin"
MODELS_DIR = BASE_DIR / "models"
LLAMA_SERVER = BIN_DIR / "llama-server.exe"

app = FastAPI(title="LocalMind Control Center")
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


# Global state
server_process: Optional[subprocess.Popen] = None
server_lock = threading.Lock()
active_config = {
    "model": "",
    "context": 131072,
    "mtp": True,
    "threads": 6,
    "status": "stopped",
    "pid": None,
    "started_at": None
}

log_subscribers: List[asyncio.Queue] = []
recent_logs: List[str] = []
MAX_RECENT_LOGS = 1000

def broadcast_log_sync(line: str):
    recent_logs.append(line)
    if len(recent_logs) > MAX_RECENT_LOGS:
        recent_logs.pop(0)
    for q in list(log_subscribers):
        try:
            q.put_nowait(line)
        except Exception:
            pass

def log_reader_thread(proc: subprocess.Popen):
    try:
        for line in iter(proc.stdout.readline, b""):
            if not line:
                break
            try:
                decoded = line.decode("utf-8", errors="replace").rstrip("\r\n")
            except Exception:
                decoded = str(line)
            broadcast_log_sync(decoded)
    except Exception as e:
        broadcast_log_sync(f"[System Error] Log stream closed: {e}")
    finally:
        with server_lock:
            if active_config["status"] == "running":
                active_config["status"] = "stopped"
                active_config["pid"] = None
        broadcast_log_sync("[System] Servidor detenido y VRAM liberada.")

def force_cleanup_all():
    global server_process
    with server_lock:
        if server_process and server_process.poll() is None:
            try:
                server_process.terminate()
                server_process.wait(timeout=2)
            except Exception:
                try:
                    server_process.kill()
                except Exception:
                    pass
        server_process = None
        active_config["status"] = "stopped"
        active_config["pid"] = None

    # Fallback guarantees 100% VRAM release
    subprocess.run(["taskkill", "/F", "/IM", "llama-server.exe"], capture_output=True, text=True)

@app.get("/api/models")
def list_models():
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
    return {"models": models}

@app.get("/api/status")
def get_status():
    is_healthy = False
    stats = {}
    try:
        import urllib.request
        req = urllib.request.Request("http://127.0.0.1:8080/health")
        with urllib.request.urlopen(req, timeout=1) as resp:
            if resp.status == 200:
                is_healthy = True
    except Exception:
        is_healthy = False

    if is_healthy:
        try:
            req_slots = urllib.request.Request("http://127.0.0.1:8080/slots")
            with urllib.request.urlopen(req_slots, timeout=1) as resp:
                data = json.loads(resp.read().decode())
                if data and isinstance(data, list) and len(data) > 0:
                    slot = data[0]
                    stats["is_processing"] = slot.get("is_processing", False)
                    stats["n_prompt_tokens"] = slot.get("n_prompt_tokens", 0)
                    stats["n_decoded"] = slot.get("n_decoded", 0)
        except Exception:
            pass

    return {
        "status": "running" if is_healthy else active_config["status"],
        "is_healthy": is_healthy,
        "pid": active_config["pid"],
        "model": active_config["model"],
        "context": active_config["context"],
        "mtp": active_config["mtp"],
        "started_at": active_config["started_at"],
        "stats": stats
    }

@app.post("/api/start")
def start_model(payload: dict):
    global server_process
    with server_lock:
        if server_process and server_process.poll() is None:
            return JSONResponse({"error": "El servidor ya está en ejecución."}, status_code=400)

        force_cleanup_all()

        default_model = "Qwen3.8-27B-IQ4_XS_4BPW.gguf"
        if not (MODELS_DIR / default_model).exists():
            models = [f.name for f in MODELS_DIR.glob("*.gguf") if "mmproj" not in f.name.lower()]
            default_model = models[0] if models else "Qwen3.8-27B-IQ4_XS_4BPW.gguf"

        model_name = payload.get("model") or default_model
        profile_id = payload.get("profile", "turbo")
        profile = HARDWARE_PROFILES.get(profile_id, HARDWARE_PROFILES["turbo"])

        context_size = int(payload.get("context") or profile["context"])
        threads = int(payload.get("threads", 6))

        model_file = MODELS_DIR / model_name
        if not model_file.exists():
            return JSONResponse({"error": f"Modelo no encontrado: {model_file}"}, status_code=404)

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
            "-t", str(threads),
            "-tb", str(threads),
            "--prio", "2",
            "--prio-batch", "2",
            "-b", "2048",
            "-ub", "512",
            "--device", "Vulkan0",
            "--split-mode", "none",
            "--host", "127.0.0.1",
            "--port", "8080",
            "-np", "1",
            "--reasoning-preserve",
            "--metrics"
        ]

        if profile.get("cache_ram", 0) > 0:
            cmd.extend(["--cache-ram", str(profile["cache_ram"])])

        if profile.get("extra_flags"):
            cmd.extend(profile["extra_flags"])

        if mmproj_file and mmproj_file.exists():
            cmd.extend(["--mmproj", str(mmproj_file)])

        broadcast_log_sync(f"[System] Iniciando {model_name} con {context_size} tokens de contexto en GPU...")

        creation_flags = 0
        if sys.platform == "win32":
            creation_flags = subprocess.CREATE_NO_WINDOW

        server_process = subprocess.Popen(
            cmd,
            cwd=str(BASE_DIR),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            creationflags=creation_flags,
            bufsize=1
        )

        active_config["model"] = model_name
        active_config["context"] = context_size
        active_config["mtp"] = use_mtp
        active_config["threads"] = threads
        active_config["status"] = "starting"
        active_config["pid"] = server_process.pid
        active_config["started_at"] = time.strftime("%Y-%m-%d %H:%M:%S")

        t = threading.Thread(target=log_reader_thread, args=(server_process,), daemon=True)
        t.start()

    return {"status": "starting", "pid": active_config["pid"]}

@app.post("/api/stop")
def stop_model():
    broadcast_log_sync("[System] Solicitud de detención. Liberando memoria...")
    force_cleanup_all()
    return {"status": "stopped", "message": "VRAM y RAM liberadas al 100%."}

@app.post("/api/open-omp")
def open_omp():
    try:
        wt_path = Path(os.environ.get("LOCALAPPDATA", "")) / "Microsoft/WindowsApps/wt.exe"
        cmd = 'omp --model bonsai/bonsai-2-27b'
        if wt_path.exists():
            subprocess.Popen([str(wt_path), "-w", "0", "new-tab", "cmd", "/k", cmd])
        else:
            subprocess.Popen(["cmd.exe", "/c", f"start {cmd}"], shell=True)
        return {"status": "ok", "message": "OMP abierto."}
    except Exception as e:
        return JSONResponse({"error": str(e)}, status_code=500)

@app.post("/api/open-chat")
def open_chat():
    try:
        import webbrowser
        webbrowser.open("http://127.0.0.1:8080")
        return {"status": "ok"}
    except Exception as e:
        return JSONResponse({"error": str(e)}, status_code=500)

@app.websocket("/ws/logs")
async def websocket_logs(websocket: WebSocket):
    await websocket.accept()
    q = asyncio.Queue()
    log_subscribers.append(q)
    try:
        for line in recent_logs:
            await websocket.send_text(line)
        while True:
            line = await q.get()
            await websocket.send_text(line)
    except WebSocketDisconnect:
        pass
    finally:
        if q in log_subscribers:
            log_subscribers.remove(q)

@app.get("/", response_class=HTMLResponse)
def serve_dashboard():
    html_file = BASE_DIR / "dashboard.html"
    if html_file.exists():
        return HTMLResponse(html_file.read_text(encoding="utf-8"))
    return HTMLResponse("<h1>Dashboard HTML no encontrado</h1>", status_code=404)

if __name__ == "__main__":
    force_cleanup_all()
    uvicorn.run(app, host="127.0.0.1", port=7860, log_level="warning")
