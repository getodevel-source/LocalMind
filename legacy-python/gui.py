import os
import sys
import time
import subprocess
import threading
import webbrowser
from pathlib import Path
import tkinter as tk
import customtkinter as ctk

BASE_DIR = Path(__file__).resolve().parent
BIN_DIR = BASE_DIR / "bin"
MODELS_DIR = BASE_DIR / "models"
LLAMA_SERVER = BIN_DIR / "llama-server.exe"
ICON_PATH = BASE_DIR / "localmind.ico"

ctk.set_appearance_mode("dark")
ctk.set_default_color_theme("blue")

class LocalMindStudio(ctk.CTk):
    def __init__(self):
        super().__init__()

        self.title("LocalMind Studio · Local AI Engine")
        self.geometry("980x760")
        self.minsize(860, 640)

        # Set Window Icon
        if ICON_PATH.exists():
            try:
                self.iconbitmap(str(ICON_PATH))
            except Exception:
                pass

        self.proc = None
        self.reader_thread = None
        self.is_running = False
        self.auto_scroll = True
        self.total_tokens_decoded = 0

        self.protocol("WM_DELETE_WINDOW", self.on_exit)

        self.setup_ui()
        self.refresh_models()
        self.poll_server_status()

    def setup_ui(self):
        # 1. Top Navigation Bar
        top_bar = ctk.CTkFrame(self, height=64, corner_radius=0, fg_color="#161b22")
        top_bar.pack(fill="x", side="top")

        # Brand / Title
        brand_frame = ctk.CTkFrame(top_bar, fg_color="transparent")
        brand_frame.pack(side="left", padx=20, pady=10)

        ctk.CTkLabel(
            brand_frame,
            text="⚡ LocalMind Studio",
            font=ctk.CTkFont(size=20, weight="bold"),
            text_color="#f0f6fc"
        ).pack(anchor="w")

        ctk.CTkLabel(
            brand_frame,
            text="Ternary 1.76-bit Architecture · Qwen 3.8 27B · Motor Vulkan",
            font=ctk.CTkFont(size=11),
            text_color="#8b949e"
        ).pack(anchor="w")

        # Badges (Right side)
        badges_frame = ctk.CTkFrame(top_bar, fg_color="transparent")
        badges_frame.pack(side="right", padx=20, pady=10)

        # Hardware Badge
        hw_badge = ctk.CTkLabel(
            badges_frame,
            text="AMD Radeon RX 6800 XT (16 GB)",
            font=ctk.CTkFont(size=12, weight="bold"),
            text_color="#a5b4fc",
            fg_color="#1e1b4b",
            corner_radius=8,
            padx=12,
            pady=5
        )
        hw_badge.pack(side="left", padx=(0, 10))

        # Status Badge
        self.status_badge = ctk.CTkLabel(
            badges_frame,
            text="● Inactivo (VRAM Libre)",
            font=ctk.CTkFont(size=12, weight="bold"),
            text_color="#f87171",
            fg_color="#450a0a",
            corner_radius=8,
            padx=12,
            pady=5
        )
        self.status_badge.pack(side="left")

        # 2. Tabs Workspace
        self.tabview = ctk.CTkTabview(
            self,
            fg_color="#0d1117",
            segmented_button_fg_color="#161b22",
            segmented_button_selected_color="#238636",
            segmented_button_selected_hover_color="#2ea043",
            segmented_button_unselected_color="#21262d",
            segmented_button_unselected_hover_color="#30363d",
            text_color="#f0f6fc"
        )
        self.tabview.pack(fill="both", expand=True, padx=16, pady=(10, 14))

        self.tab_dashboard = self.tabview.add("🎮 Panel de Control")
        self.tab_console = self.tabview.add("📟 Consola en Vivo")
        self.tab_info = self.tabview.add("ℹ️ Hardware y Arquitectura")

        self.build_dashboard_tab()
        self.build_console_tab()
        self.build_info_tab()

    def build_dashboard_tab(self):
        # Grid with 2 columns
        grid = ctk.CTkFrame(self.tab_dashboard, fg_color="transparent")
        grid.pack(fill="both", expand=True, padx=6, pady=6)
        grid.grid_columnconfigure(0, weight=3)
        grid.grid_columnconfigure(1, weight=2)
        grid.grid_rowconfigure(0, weight=1)

        # Left Column: Configuration Card
        cfg_card = ctk.CTkFrame(grid, fg_color="#161b22", border_width=1, border_color="#30363d", corner_radius=12)
        cfg_card.grid(row=0, column=0, sticky="nsew", padx=(0, 10), pady=0)

        ctk.CTkLabel(
            cfg_card,
            text="⚙️ CONFIGURACIÓN DE CARGA EN GPU",
            font=ctk.CTkFont(size=13, weight="bold"),
            text_color="#58a6ff"
        ).pack(anchor="w", padx=18, pady=(16, 12))

        # Model selector
        ctk.CTkLabel(cfg_card, text="Modelo GGUF:", font=ctk.CTkFont(size=12, weight="bold"), text_color="#8b949e").pack(anchor="w", padx=18, pady=(0, 3))
        self.model_menu = ctk.CTkOptionMenu(
            cfg_card,
            values=["Buscando modelos..."],
            fg_color="#21262d",
            button_color="#30363d",
            button_hover_color="#388bfd",
            text_color="#f0f6fc",
            height=36
        )
        self.model_menu.pack(fill="x", padx=18, pady=(0, 12))

        # Context selector
        ctk.CTkLabel(cfg_card, text="Ventana de Contexto Máxima:", font=ctk.CTkFont(size=12, weight="bold"), text_color="#8b949e").pack(anchor="w", padx=18, pady=(0, 3))
        self.context_menu = ctk.CTkOptionMenu(
            cfg_card,
            values=[
                "32,768 tokens (32K) — Turbo VRAM (Máxima Velocidad T/S)",
                "64K tokens (65,536) — Equilibrado Pro (VRAM + RAM)",
                "128K tokens (131,072) — Extendido (Libros y Repositorios)",
                "262,144 tokens (262K) — Límite Hardware (Máximo Nativo)"
            ],
            fg_color="#21262d",
            button_color="#30363d",
            button_hover_color="#388bfd",
            text_color="#f0f6fc",
            height=36
        )
        self.context_menu.pack(fill="x", padx=18, pady=(0, 12))

        # Two sub-columns: CPU Threads and Speculative
        sub_row = ctk.CTkFrame(cfg_card, fg_color="transparent")
        sub_row.pack(fill="x", padx=18, pady=(0, 12))

        th_box = ctk.CTkFrame(sub_row, fg_color="transparent")
        th_box.pack(side="left", fill="x", expand=True, padx=(0, 8))
        ctk.CTkLabel(th_box, text="Hilos CPU (Zen 4):", font=ctk.CTkFont(size=12, weight="bold"), text_color="#8b949e").pack(anchor="w", pady=(0, 3))
        self.threads_menu = ctk.CTkOptionMenu(
            th_box,
            values=["6 hilos (6 Núcleos Físicos)", "8 hilos", "12 hilos (SMT)"],
            fg_color="#21262d",
            button_color="#30363d",
            height=34
        )
        self.threads_menu.pack(fill="x")

        spec_box = ctk.CTkFrame(sub_row, fg_color="transparent")
        spec_box.pack(side="right", fill="x", expand=True, padx=(8, 0))
        ctk.CTkLabel(spec_box, text="Modo Especulativo:", font=ctk.CTkFont(size=12, weight="bold"), text_color="#8b949e").pack(anchor="w", pady=(0, 3))
        self.spec_menu = ctk.CTkOptionMenu(
            spec_box,
            values=[
                "Desactivado (Estable / Por defecto)",
                "N-Gram Cache (Aceleración sintaxis)"
            ],
            fg_color="#21262d",
            button_color="#30363d",
            height=34
        )
        self.spec_menu.pack(fill="x")

        # Status line in card
        self.cfg_info_label = ctk.CTkLabel(
            cfg_card,
            text="Flash Attention activado · KV Cache 4-bit · Vulkan 100% offload",
            font=ctk.CTkFont(size=11),
            text_color="#7ee787"
        )
        self.cfg_info_label.pack(anchor="w", padx=18, pady=(0, 14))

        # Main Action Buttons
        btn_box = ctk.CTkFrame(cfg_card, fg_color="transparent")
        btn_box.pack(fill="x", padx=18, pady=(10, 18), side="bottom")

        self.btn_start = ctk.CTkButton(
            btn_box,
            text="🚀 INICIAR MODELO EN GPU",
            font=ctk.CTkFont(size=14, weight="bold"),
            fg_color="#1f6feb",
            hover_color="#388bfd",
            height=46,
            corner_radius=10,
            command=self.start_engine
        )
        self.btn_start.pack(fill="x", pady=(0, 10))

        self.btn_stop = ctk.CTkButton(
            btn_box,
            text="🛑 DETENER Y LIBERAR VRAM",
            font=ctk.CTkFont(size=14, weight="bold"),
            fg_color="#da3633",
            hover_color="#f85149",
            height=46,
            corner_radius=10,
            state="disabled",
            command=self.stop_engine
        )
        self.btn_stop.pack(fill="x")

        # Right Column: Launchers & Live Stats
        right_card = ctk.CTkFrame(grid, fg_color="#161b22", border_width=1, border_color="#30363d", corner_radius=12)
        right_card.grid(row=0, column=1, sticky="nsew", padx=(10, 0), pady=0)

        ctk.CTkLabel(
            right_card,
            text="🚀 ACCESOS DIRECTOS",
            font=ctk.CTkFont(size=13, weight="bold"),
            text_color="#58a6ff"
        ).pack(anchor="w", padx=18, pady=(16, 12))

        # OMP Button
        self.btn_omp = ctk.CTkButton(
            right_card,
            text="💻  Abrir OMP (Terminal Windows)\nInicia sesión de código con modelo local (27B)",
            font=ctk.CTkFont(size=13, weight="bold"),
            fg_color="#238636",
            hover_color="#2ea043",
            height=58,
            corner_radius=10,
            state="disabled",
            command=self.launch_omp
        )
        self.btn_omp.pack(fill="x", padx=18, pady=(0, 10))

        # Web Chat Button
        self.btn_chat = ctk.CTkButton(
            right_card,
            text="💬  Abrir Interfaz Web Oficial (:8080)\nChat visual con visión e imágenes",
            font=ctk.CTkFont(size=13, weight="bold"),
            fg_color="#8957e5",
            hover_color="#a371f7",
            height=58,
            corner_radius=10,
            state="disabled",
            command=self.launch_web_chat
        )
        self.btn_chat.pack(fill="x", padx=18, pady=(0, 16))

        # Stats Deck
        ctk.CTkLabel(
            right_card,
            text="📊 TELEMETRÍA EN VIVO",
            font=ctk.CTkFont(size=13, weight="bold"),
            text_color="#58a6ff"
        ).pack(anchor="w", padx=18, pady=(0, 10))

        stats_deck = ctk.CTkFrame(right_card, fg_color="#0d1117", border_width=1, border_color="#30363d", corner_radius=10)
        stats_deck.pack(fill="x", padx=18, pady=(0, 16))

        def create_stat_row(parent, label, val_id):
            f = ctk.CTkFrame(parent, fg_color="transparent")
            f.pack(fill="x", padx=14, pady=8)
            ctk.CTkLabel(f, text=label, font=ctk.CTkFont(size=12), text_color="#8b949e").pack(side="left")
            lbl = ctk.CTkLabel(f, text="--", font=ctk.CTkFont(size=12, weight="bold"), text_color="#f0f6fc")
            lbl.pack(side="right")
            return lbl

        self.lbl_stat_gpu = create_stat_row(stats_deck, "Aceleración GPU:", "stat_gpu")
        self.lbl_stat_gpu.configure(text="Vulkan0 (RX 6800 XT)", text_color="#7ee787")

        self.lbl_stat_state = create_stat_row(stats_deck, "Estado del Motor:", "stat_state")
        self.lbl_stat_state.configure(text="En reposo", text_color="#8b949e")

        self.lbl_stat_tokens = create_stat_row(stats_deck, "Tokens Procesados:", "stat_tokens")
        self.lbl_stat_tokens.configure(text="0 tokens", text_color="#79c0ff")

        self.lbl_stat_endpoint = create_stat_row(stats_deck, "Endpoint Local:", "stat_endpoint")
        self.lbl_stat_endpoint.configure(text="http://127.0.0.1:8080", text_color="#d2a8ff")

    def build_console_tab(self):
        toolbar = ctk.CTkFrame(self.tab_console, fg_color="transparent")
        toolbar.pack(fill="x", padx=4, pady=(4, 8))

        ctk.CTkLabel(
            toolbar,
            text="Terminal de Salida en Directo de llama-server",
            font=ctk.CTkFont(size=13, weight="bold"),
            text_color="#8b949e"
        ).pack(side="left")

        self.btn_autoscroll = ctk.CTkButton(
            toolbar,
            text="Auto-scroll: ON",
            width=110,
            height=28,
            fg_color="#21262d",
            hover_color="#30363d",
            command=self.toggle_autoscroll
        )
        self.btn_autoscroll.pack(side="right", padx=(8, 0))

        ctk.CTkButton(
            toolbar,
            text="Copiar Logs",
            width=90,
            height=28,
            fg_color="#21262d",
            hover_color="#30363d",
            command=self.copy_logs
        ).pack(side="right", padx=(8, 0))

        ctk.CTkButton(
            toolbar,
            text="Limpiar",
            width=70,
            height=28,
            fg_color="#21262d",
            hover_color="#30363d",
            command=self.clear_logs
        ).pack(side="right")

        self.console_box = ctk.CTkTextbox(
            self.tab_console,
            font=ctk.CTkFont(family="Consolas", size=12),
            text_color="#38bdf8",
            fg_color="#030712",
            border_width=1,
            border_color="#1f2937",
            corner_radius=8
        )
        self.console_box.pack(fill="both", expand=True, padx=4, pady=4)
        self.console_box.insert("end", "⚡ LocalMind Studio inicializado.\nPresiona 'INICIAR MODELO EN GPU' para encender el motor.\n\n")

    def build_info_tab(self):
        info_scroll = ctk.CTkScrollableFrame(self.tab_info, fg_color="#161b22", corner_radius=12)
        info_scroll.pack(fill="both", expand=True, padx=4, pady=4)

        def add_info_section(title, text):
            ctk.CTkLabel(info_scroll, text=title, font=ctk.CTkFont(size=15, weight="bold"), text_color="#58a6ff").pack(anchor="w", padx=16, pady=(16, 6))
            ctk.CTkLabel(info_scroll, text=text, font=ctk.CTkFont(size=12), text_color="#c9d1d9", justify="left", wraplength=880).pack(anchor="w", padx=16, pady=(0, 10))

        add_info_section(
            "1. ¿Cómo funciona la arquitectura de este modelo 27B?",
            "El motor implementa la arquitectura ternaria de PrismML basada en Qwen 3.8 27B. "
            "Sus pesos matriciales están cuantizados en {-1, 0, +1} (1.76 bits efectivos), lo que permite que un modelo insignia "
            "de 27 mil millones de parámetros pese únicamente 5.54 GB en disco y quepa completo en la memoria de la tarjeta gráfica."
        )

        add_info_section(
            "2. ¿Por qué se alcanza un contexto tan masivo (hasta 262K)?",
            "A diferencia de los Transformers tradicionales donde todas las capas acumulan memoria de contexto cuadrática, "
            "el motor utiliza una arquitectura híbrida: 48 de sus 64 capas son de atención lineal Gated DeltaNet (memoria fija que no crece con la longitud del texto). "
            "Solo 16 capas generan KV Cache, el cual está cuantizado a 4 bits (q4_0) con Flash Attention. "
            "Esto permite manejar entre 131,072 tokens (128K) en pura VRAM o hasta 262,144 tokens (262K) compartiendo con la memoria RAM DDR5."
        )

        add_info_section(
            "3. Integración con OMP (Oh My Pi)",
            "El servidor expone automáticamente una API compatible con OpenAI en http://127.0.0.1:8080/v1 con los IDs 'localmind' y 'bonsai-2-27b'. "
            "En tu archivo ~/.omp/agent/models.yml ya está registrado el proveedor. "
            "Simplemente haz clic en el botón 'Abrir OMP' en el panel de control o escribe '/model bonsai/bonsai-2-27b' en tu terminal."
        )

        add_info_section(
            "4. Ciclo de vida y Cero Residuos",
            "Al presionar 'Detener y Liberar VRAM' o al cerrar esta ventana con la 'X', la aplicación se encarga de terminar "
            "cualquier subproceso de llama-server y forzar la liberación completa de la memoria gráfica (VRAM) y del sistema (RAM). "
            "Tu AMD Radeon RX 6800 XT siempre volverá a sus ~15,500 MiB libres."
        )

    def log(self, text):
        self.after(0, self._append_log, text)

    def _append_log(self, text):
        self.console_box.insert("end", text + "\n")
        if self.auto_scroll:
            self.console_box.see("end")

    def toggle_autoscroll(self):
        self.auto_scroll = not self.auto_scroll
        self.btn_autoscroll.configure(text=f"Auto-scroll: {'ON' if self.auto_scroll else 'OFF'}")

    def clear_logs(self):
        self.console_box.delete("1.0", "end")

    def copy_logs(self):
        try:
            content = self.console_box.get("1.0", "end")
            self.clipboard_clear()
            self.clipboard_append(content)
            self.btn_autoscroll.configure(text="¡Copiado!")
            self.after(1500, lambda: self.btn_autoscroll.configure(text=f"Auto-scroll: {'ON' if self.auto_scroll else 'OFF'}"))
        except Exception:
            pass

    def refresh_models(self):
        models = []
        if MODELS_DIR.exists():
            for f in MODELS_DIR.glob("*.gguf"):
                if "mmproj" not in f.name.lower():
                    models.append(f.name)
        if models:
            # Sort so Qwen3.8 is prioritized
            models.sort(key=lambda x: (0 if "qwen3.8" in x.lower() else 1, x))
            self.model_menu.configure(values=models)
            self.model_menu.set(models[0])
        else:
            self.model_menu.configure(values=["No se encontraron modelos"])

    def start_engine(self):
        if self.is_running:
            return

        model_name = self.model_menu.get()
        model_file = MODELS_DIR / model_name
        if not model_file.exists():
            self.log(f"[Error] Archivo de modelo no encontrado: {model_file}")
            return

        ctx_choice = self.context_menu.get()
        cache_ram = 0
        extra_flags = ["--cache-reuse", "256"]
        if "32K" in ctx_choice:
            context_size = 32768
            cache_ram = 0
        elif "64K" in ctx_choice:
            context_size = 65536
            cache_ram = 4096
        elif "128K" in ctx_choice:
            context_size = 131072
            cache_ram = 6144
        elif "262K" in ctx_choice:
            context_size = 262144
            cache_ram = 6144
            extra_flags = ["-kvu", "--cache-reuse", "512"]
        else:
            context_size = 32768
        th_choice = self.threads_menu.get()
        threads = "6"
        if "8" in th_choice:
            threads = "8"
        elif "12" in th_choice:
            threads = "12"

        spec_choice = self.spec_menu.get()
        mmproj_file = MODELS_DIR / "Ternary-Bonsai-2-27B-mmproj-Q8_0.gguf"

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

        if cache_ram > 0:
            cmd.extend(["--cache-ram", str(cache_ram)])
        if extra_flags:
            cmd.extend(extra_flags)

        if mmproj_file.exists():
            cmd.extend(["--mmproj", str(mmproj_file)])

        # Speculative selection
        if "N-Gram" in spec_choice:
            cmd.extend(["--spec-type", "ngram-cache"])

        self.btn_start.configure(state="disabled", text="⏳ CARGANDO EN VRAM...")
        self.status_badge.configure(text="● Cargando en GPU...", text_color="#fbbf24", fg_color="#451a03")
        self.lbl_stat_state.configure(text="Iniciando...", text_color="#fbbf24")

        self.log(f"======================================================================")
        self.log(f"[LocalMind] Cargando modelo: {model_name}")
        self.log(f"[LocalMind] Contexto: {context_size} tokens | Hilos Zen 4: {threads} | Especulativo: {spec_choice}")
        self.log(f"======================================================================")

        def runner():
            try:
                creation_flags = subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0
                self.proc = subprocess.Popen(
                    cmd,
                    cwd=str(BASE_DIR),
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    creationflags=creation_flags,
                    bufsize=1
                )
                for line in iter(self.proc.stdout.readline, b""):
                    if not line:
                        break
                    try:
                        decoded = line.decode("utf-8", errors="replace").rstrip("\r\n")
                    except Exception:
                        decoded = str(line)
                    self.log(decoded)
            except Exception as e:
                self.log(f"[Error de ejecución] {e}")
            finally:
                self.is_running = False
                self.after(0, self._on_engine_exit)

        self.reader_thread = threading.Thread(target=runner, daemon=True)
        self.reader_thread.start()

    def _on_engine_exit(self):
        self.status_badge.configure(text="● Inactivo (VRAM Libre)", text_color="#f87171", fg_color="#450a0a")
        self.btn_start.configure(state="normal", text="🚀 INICIAR MODELO EN GPU")
        self.btn_stop.configure(state="disabled")
        self.btn_omp.configure(state="disabled")
        self.btn_chat.configure(state="disabled")
        self.lbl_stat_state.configure(text="En reposo", text_color="#8b949e")
        self.log("[LocalMind] Servidor detenido. Memoria VRAM y RAM liberada al 100%.")

    def stop_engine(self):
        self.btn_stop.configure(state="disabled", text="⏳ DETENIENDO...")
        self.log("[LocalMind] Deteniendo servidor y purgando tensores de la GPU...")

        def killer():
            if self.proc and self.proc.poll() is None:
                try:
                    self.proc.terminate()
                    self.proc.wait(timeout=2)
                except Exception:
                    try:
                        self.proc.kill()
                    except Exception:
                        pass
            subprocess.run(["taskkill", "/F", "/IM", "llama-server.exe"], capture_output=True, text=True)
            self.after(0, lambda: self.btn_stop.configure(text="🛑 DETENER Y LIBERAR VRAM"))

        threading.Thread(target=killer, daemon=True).start()

    def launch_omp(self):
        try:
            wt_path = Path(os.environ.get("LOCALAPPDATA", "")) / "Microsoft/WindowsApps/wt.exe"
            cmd = 'omp --model bonsai/bonsai-2-27b'
            if wt_path.exists():
                subprocess.Popen([str(wt_path), "-w", "0", "new-tab", "cmd", "/k", cmd])
            else:
                subprocess.Popen(["cmd.exe", "/c", f"start {cmd}"], shell=True)
            self.log("[LocalMind] Terminal OMP lanzada con modelo local (27B).")
        except Exception as e:
            self.log(f"[Error al lanzar OMP] {e}")

    def launch_web_chat(self):
        webbrowser.open("http://127.0.0.1:8080")
        self.log("[LocalMind] Abierta interfaz web en http://127.0.0.1:8080.")

    def poll_server_status(self):
        def worker():
            import urllib.request
            is_up = False
            stats = {}
            try:
                req = urllib.request.Request("http://127.0.0.1:8080/health")
                with urllib.request.urlopen(req, timeout=1) as resp:
                    if resp.status == 200:
                        is_up = True
            except Exception:
                is_up = False

            if is_up:
                try:
                    req_slots = urllib.request.Request("http://127.0.0.1:8080/slots")
                    with urllib.request.urlopen(req_slots, timeout=1) as resp:
                        data = json.loads(resp.read().decode())
                        if data and isinstance(data, list) and len(data) > 0:
                            slot = data[0]
                            stats["is_processing"] = slot.get("is_processing", False)
                            stats["n_decoded"] = slot.get("n_decoded", 0)
                except Exception:
                    pass

            def apply():
                if is_up:
                    if not self.is_running:
                        self.is_running = True
                    self.status_badge.configure(text="● Servidor Activo (:8080)", text_color="#34d399", fg_color="#064e3b")
                    self.btn_start.configure(state="disabled", text="⚡ MOTOR EN EJECUCIÓN")
                    self.btn_stop.configure(state="normal")
                    self.btn_omp.configure(state="normal")
                    self.btn_chat.configure(state="normal")

                    if stats:
                        if stats.get("is_processing", False):
                            self.lbl_stat_state.configure(text="⚡ Procesando...", text_color="#f59e0b")
                        else:
                            self.lbl_stat_state.configure(text="En reposo (Listo)", text_color="#7ee787")
                        if "n_decoded" in stats:
                            self.lbl_stat_tokens.configure(text=f"{stats['n_decoded']} tokens")
                else:
                    if self.is_running:
                        self.is_running = False
                        self._on_engine_exit()

            self.after(0, apply)

        threading.Thread(target=worker, daemon=True).start()
        self.after(1500, self.poll_server_status)

    def on_exit(self):
        if self.proc and self.proc.poll() is None:
            try:
                self.proc.terminate()
            except Exception:
                pass
        subprocess.run(["taskkill", "/F", "/IM", "llama-server.exe"], capture_output=True, text=True)
        self.destroy()

if __name__ == "__main__":
    subprocess.run(["taskkill", "/F", "/IM", "llama-server.exe"], capture_output=True, text=True)
    app = LocalMindStudio()
    app.mainloop()
