#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod process;
mod profiles;
mod server;

use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::{Icon, WindowBuilder};
use wry::WebViewBuilder;

use config::ConfigStore;
use process::ProcessManager;
use server::HttpServer;

fn get_base_dir() -> PathBuf {
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            // In “installed” layout: exe junto a models/ y ui.html.
            if parent.join("models").is_dir() {
                return parent.to_path_buf();
            }
        }
    }
    // Desarrollo / server-only: cwd debe contener models/ (raíz del proyecto).
    if let Ok(cwd) = env::current_dir() {
        if cwd.join("models").is_dir() {
            return cwd;
        }
    }
    env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default()
}

fn load_window_icon(base_dir: &PathBuf) -> Option<Icon> {
    let ico_path = base_dir.join("localmind.ico");
    let bytes = std::fs::read(ico_path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    Icon::from_rgba(img.into_raw(), width, height).ok()
}

/// Single-instance: intento crear un lock file exclusivo en %TEMP%; si ya existe,
/// enfocar la ventana existente (trae al frente vía PowerShell) y salir.
fn acquire_single_instance() -> Option<std::fs::File> {
    let lock_path = env::temp_dir().join("localmind.lock");
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    // share_mode(0) → lock exclusivo mientras el proceso viva; el handle se libera al morir,
    // pero el archivo remanente necesita limpieza con create_new.
    let _ = std::fs::remove_file(&lock_path); // lock huérfano de un crash previo
    match OpenOptions::new()
        .create_new(true)
        .write(true)
        .share_mode(0)
        .open(&lock_path)
    {
        Ok(f) => Some(f),
        Err(_) => {
            // Ya vive otra instancia: enfocar ventana existente y salir.
            let _ = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    "$h = Get-Process LocalMind -ErrorAction SilentlyContinue | Select-Object -First 1; \
                     Add-Type -MemberDefinition '[DllImport(\"user32.dll\")] public static extern bool SetForegroundWindow(IntPtr h);' -Name U32 -Namespace W; \
                     [W.U32]::SetForegroundWindow($h.MainWindowHandle)",
                ])
                .spawn();
            None
        }
    }
}

fn main() {
    // Single instance: si ya corre otra instancia, enfoca y sale.
    if acquire_single_instance().is_none() {
        return;
    }

    let base_dir = get_base_dir();
    let config = Arc::new(ConfigStore::load(&base_dir));
    let cfg_now = config.get();

    let process_mgr = Arc::new(ProcessManager::new(base_dir.clone(), Arc::clone(&config)));

    // Start embedded HTTP server
    let server = match HttpServer::start(Arc::clone(&process_mgr), Arc::clone(&config), base_dir.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error al iniciar servidor HTTP: {}", e);
            return;
        }
    };

    let port = server.port();
    let server_url = format!("http://127.0.0.1:{}", port);

    let args: Vec<String> = env::args().collect();
    if args.iter().any(|a| a == "--server-only" || a == "--headless") {
        println!("LocalMind server-only escuchando en {}", server_url);
        // Modo headless: mantener vivo el proceso mientras el manager exista.
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }

    // Native Window Mode
    let event_loop = EventLoop::new();

    let mut window_builder = WindowBuilder::new()
        .with_title("LocalMind Studio")
        .with_inner_size(LogicalSize::new(1180.0, 820.0))
        .with_min_inner_size(LogicalSize::new(960.0, 680.0));

    if let Some(icon) = load_window_icon(&base_dir) {
        window_builder = window_builder.with_window_icon(Some(icon));
    }

    let window = match window_builder.build(&event_loop) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Error al crear ventana: {}", e);
            return;
        }
    };

    let webview = match WebViewBuilder::new(&window).with_url(&server_url).build() {
        Ok(wv) => wv,
        Err(e) => {
            eprintln!("Error al crear webview: {}", e);
            return;
        }
    };

    // Keep webview alive during event loop
    let _wv = webview;
    let _ = cfg_now; // (leído para materializar el TOML default en el primer arranque)

    window.set_focus();

    let mgr_cleanup = Arc::clone(&process_mgr);

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested | WindowEvent::Destroyed,
            ..
        } = event
        {
            mgr_cleanup.stop();
            *control_flow = ControlFlow::Exit;
        }
    });
}
