#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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

use process::ProcessManager;
use server::HttpServer;

fn get_base_dir() -> PathBuf {
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            // Check if parent has models/ or ui.html
            if parent.join("ui.html").exists() || parent.join("models").exists() {
                return parent.to_path_buf();
            }
            // If running from target/release or target/debug
            if let Some(grandparent) = parent.parent() {
                if let Some(root) = grandparent.parent() {
                    if root.join("ui.html").exists() || root.join("models").exists() {
                        return root.to_path_buf();
                    }
                }
            }
        }
    }
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn load_window_icon(base_dir: &PathBuf) -> Option<Icon> {
    let ico_path = base_dir.join("localmind.ico");
    let bytes = std::fs::read(ico_path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    Icon::from_rgba(img.into_raw(), width, height).ok()
}

fn main() {
    let base_dir = get_base_dir();
    let process_mgr = Arc::new(ProcessManager::new(base_dir.clone()));

    // Start embedded HTTP server
    let server = match HttpServer::start(Arc::clone(&process_mgr), base_dir.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error al iniciar servidor HTTP LocalMind: {}", e);
            return;
        }
    };

    let port = server.port();
    let server_url = format!("http://127.0.0.1:{}", port);

    let args: Vec<String> = env::args().collect();
    if args.iter().any(|a| a == "--server-only" || a == "--headless") {
        println!("LocalMind Server iniciado en {}", server_url);
        println!("Presiona Ctrl+C para detener.");
        // Block main thread
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
            eprintln!("Error al crear ventana nativa: {}", e);
            // Fallback to launching in browser if window creation fails
            let _ = std::process::Command::new("cmd.exe")
                .args(["/c", &format!("start {}", server_url)])
                .spawn();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
    };

    let webview = match WebViewBuilder::new(&window)
        .with_url(&server_url)
        .build()
    {
        Ok(wv) => wv,
        Err(e) => {
            eprintln!("Error al inicializar WebView2: {}", e);
            let _ = std::process::Command::new("cmd.exe")
                .args(["/c", &format!("start {}", server_url)])
                .spawn();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
    };

    // Keep webview alive during event loop
    let _wv = webview;

    let mgr_cleanup = Arc::clone(&process_mgr);
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                mgr_cleanup.stop();
                *control_flow = ControlFlow::Exit;
            }
            Event::LoopDestroyed => {
                mgr_cleanup.stop();
            }
            _ => (),
        }
    });
}
