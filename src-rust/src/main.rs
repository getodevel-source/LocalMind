#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agents;
mod auth;
mod config;
mod filelog;
mod launcher;
mod meta;
mod models;
mod notify;
mod process;
mod server;
mod update;
mod usage;

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
    // Fase C (OMNI): icono nuevo con fallback al histórico durante la transición.
    for name in ["omni.ico", "localmind.ico"] {
        if let Ok(bytes) = std::fs::read(base_dir.join(name)) {
            if let Ok(img) = image::load_from_memory(&bytes) {
                let img = img.to_rgba8();
                let (width, height) = img.dimensions();
                if let Ok(icon) = Icon::from_rgba(img.into_raw(), width, height) {
                    return Some(icon);
                }
            }
        }
    }
    None
}

/// Formatear la línea que el hook de pánico escribe en el log (D-46).
/// Pura: el hook en sí (que captura `PanicHookInfo`, no `Send + Sync`) queda
/// fuera, así que esto se puede testear sin matar el proceso de test.
///
/// `panic = "abort"` mata el proceso entero sin desenrollar. El stderr del
/// MOTOR sí queda persistido: `attach_reader_threads` (`process.rs`) escribe
/// stdout y stderr del hijo en `logs/localmind.log` por
/// `filelog::write_log_line` desde 2840237. Lo que NO tiene esa ruta es un
/// pánico del PROPIO LocalMind: el archivo se escribe solo por llamadas
/// explícitas (nada lo hereda por herencia), la app es GUI sin consola
/// (`windows_subsystem = "windows"`), y `abort()` no desenrolla ni deja que un
/// `Drop` vacíe nada. El mensaje del pánico no llegaría, literalmente, a
/// ningún lado: sin esta línea, un pánico de LocalMind no deja rastro. Es una
/// breadcrumb, NO un crash dump: dice qué y dónde, no el estado de memoria.
fn panic_log_line(secs_epoch: u64, payload: &str, file: &str, line: u32) -> String {
    format!(
        "[{}] [LocalMind] PANIC: {} ({}:{})",
        secs_epoch, payload, file, line
    )
}

/// Instalar el hook de pánico (D-46). Envuelve el hook previo en vez de
/// reemplazarlo, así el backtrace por defecto de Rust se sigue viendo.
///
/// ¿Corre con `panic = "abort"`? Sí, verificado empíricamente: una sonda
/// release con `panic = "abort", lto, strip, opt-level=3` escribió su
/// marcador desde el hook y luego murió con 0xC0000409. El hook se invoca
/// antes del `abort()`.
///
/// Lo que NO cubre: `abort()` explícito, SIGSEGV/stack overflow y un panic
/// dentro de un `Drop` durante el desenrollado quedan fuera. Sigue siendo una
/// breadcrumb, no un volcado.
fn install_panic_hook(log_file: PathBuf) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<payload no downcast>".to_string());
        let (file, line) = info
            .location()
            .map(|l| (l.file().to_string(), l.line()))
            .unwrap_or_else(|| ("<sin ubicacion>".to_string(), 0));
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Primero el archivo: es la evidencia que sobrevive al proceso. Si el
        // archivo no se puede escribir, el hook previo (stderr) sigue después.
        crate::filelog::write_log_line(&log_file, &panic_log_line(secs, &payload, &file, line));
        prev(info);
    }));
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
            // Fase C (OMNI): el exe instalado se llama OMNI.exe; se acepta el
            // nombre histórico para no romper el foco durante la transición.
            let _ = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    "$h = Get-Process OMNI,LocalMind -ErrorAction SilentlyContinue | Select-Object -First 1; \
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
    // D-46: ANTES de cualquier otra cosa, para que un pánico temprano (config,
    // servidor, WebView) también deje rastro. Usa el MISMO archivo que
    // `ProcessManager::log`: no hay un canal nuevo ni un archivo nuevo.
    install_panic_hook(crate::filelog::log_file(&base_dir));
    let config = Arc::new(ConfigStore::load(&base_dir));
    let cfg_now = config.get();

    let process_mgr = Arc::new(ProcessManager::new(base_dir.clone(), Arc::clone(&config)));
    // Migración tuning: una línea de log con los perfiles refrescados (P30).
    if let Some(note) = config.take_migration_note() {
        process_mgr.log(&note);
    }

    // Start embedded HTTP server
    let server = match HttpServer::start(
        Arc::clone(&process_mgr),
        Arc::clone(&config),
        base_dir.clone(),
    ) {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("Error al iniciar servidor HTTP: {}", e);
            crate::filelog::write_log_line(&crate::filelog::log_file(&base_dir), &msg);
            // Sin consola (GUI): el fallo debe verse. `rfd` ya es dependencia.
            let _ = rfd::MessageDialog::new()
                .set_title("OMNI no pudo arrancar")
                .set_description(&msg)
                .show();
            return;
        }
    };

    let port = server.port();
    let server_url = format!("http://127.0.0.1:{}", port);

    // Actualización automática (Fase Prod): chequeo silencioso al arrancar
    // (30 s de gracia para no competir con el motor) + re-chequeo cada 6 h
    // si `check_on_startup`. Con `auto_download`, una novedad se descarga
    // sola (la instalación siempre espera al reinicio y la pide el dueño).
    {
        let cfg_bg = Arc::clone(&config);
        let mgr_bg = Arc::clone(&process_mgr);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(30));
            let log: std::sync::Arc<dyn Fn(String) + Send + Sync> =
                std::sync::Arc::new(move |line: String| mgr_bg.log(&line));
            loop {
                let c = cfg_bg.get();
                if c.update.check_on_startup {
                    crate::update::spawn_check(c.update.feed.clone(), true, Arc::clone(&log));
                }
                std::thread::sleep(std::time::Duration::from_secs(6 * 3600));
            }
        });
    }

    // Native Window Mode
    let event_loop = EventLoop::new();

    let mut window_builder = WindowBuilder::new()
        .with_title("OMNI")
        .with_inner_size(LogicalSize::new(1180.0, 820.0))
        .with_min_inner_size(LogicalSize::new(960.0, 680.0));

    if let Some(icon) = load_window_icon(&base_dir) {
        window_builder = window_builder.with_window_icon(Some(icon));
    }

    let window = match window_builder.build(&event_loop) {
        Ok(w) => w,
        Err(e) => {
            let msg = format!("Error al crear ventana: {}", e);
            crate::filelog::write_log_line(&crate::filelog::log_file(&base_dir), &msg);
            let _ = rfd::MessageDialog::new()
                .set_title("OMNI no pudo arrancar")
                .set_description(&msg)
                .show();
            return;
        }
    };

    let webview = match WebViewBuilder::new(&window).with_url(&server_url).build() {
        Ok(wv) => wv,
        Err(e) => {
            let msg = format!("Error al crear webview (falta WebView2): {}", e);
            crate::filelog::write_log_line(&crate::filelog::log_file(&base_dir), &msg);
            let _ = rfd::MessageDialog::new()
                .set_title("OMNI necesita WebView2")
                .set_description(&format!(
                    "{}\n\nInstala Microsoft Edge WebView2 y reintenta.",
                    msg
                ))
                .show();
            return;
        }
    };

    // Keep webview alive during event loop
    let _wv = webview;
    let _ = cfg_now; // (leído para materializar el TOML default en el primer arranque)

    window.set_focus();

    let mgr_cleanup = Arc::clone(&process_mgr);
    let base_cleanup = base_dir.clone();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested | WindowEvent::Destroyed,
            ..
        } = event
        {
            mgr_cleanup.stop();
            // Actualización lista: swap al salir (cmd desacoplado, el exe ya
            // liberó su lock). Sin pendiente: salida normal.
            if let Some(pending) = crate::update::pending_update() {
                match crate::update::prepare_install_on_exit(&base_cleanup) {
                    Ok(script) => {
                        let _ = std::process::Command::new("cmd.exe")
                            .args(["/C", &script.to_string_lossy().to_string()])
                            .spawn();
                    }
                    Err(e) => {
                        crate::filelog::write_log_line(
                            &crate::filelog::log_file(&base_cleanup),
                            &format!(
                                "[LocalMind] No se pudo instalar la actualización {}: {}",
                                pending.version, e
                            ),
                        );
                    }
                }
            }
            *control_flow = ControlFlow::Exit;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D-46: la línea de la breadcrumb de pánico. Un test del formateo puro:
    /// no aborta el proceso de test (que sería imposible de asertar).
    #[test]
    fn panic_line_dice_que_where_y_cuando() {
        let l = panic_log_line(1700000000, "index out of bounds", "src\\process.rs", 412);
        assert!(l.starts_with("[1700000000] "), "{:?}", l);
        assert!(l.contains("PANIC"), "{:?}", l);
        assert!(l.contains("index out of bounds"), "{:?}", l);
        assert!(l.contains("src\\process.rs:412"), "{:?}", l);
    }

    /// El payload de un pánico no siempre es un `&str` ni un `String`: hay
    /// tipos propios. La línea debe salir igual, no romperse.
    #[test]
    fn panic_line_tolera_payload_no_texto() {
        let l = panic_log_line(1, "<payload no downcast>", "<sin ubicacion>", 0);
        assert!(l.contains("<payload no downcast>"), "{:?}", l);
        assert!(l.contains("<sin ubicacion>:0"), "{:?}", l);
    }

    /// La breadcrumb va al archivo que rota `filelog`, no a uno nuevo: la línea
    /// tiene que pasar por el mismo formato que el resto del log.
    #[test]
    fn panic_line_pasa_por_el_formato_de_filelog() {
        let dir = std::env::temp_dir().join(format!("lm-panic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let base = dir.join("localmind.log");
        // Un payload con salto de línea: el formateo de `filelog` lo sanea, así
        // que un panic con mensaje multilínea no rompe el archivo de log.
        crate::filelog::write_log_line(&base, &panic_log_line(7, "a\nb\0c", "f.rs", 9));
        let text = std::fs::read_to_string(&base).unwrap();
        // `write_log_line` antepone SU propio epoch UTC (no el de la línea) y
        // `format_line` sanea \n, \r y \0 a espacio: el archivo tiene UNA línea
        // lógica, que es lo que permite seguir un pánico al leer el log.
        assert!(
            text.contains("[LocalMind] PANIC: a b c (f.rs:9)"),
            "{:?}",
            text
        );
        assert_eq!(text.matches('\n').count(), 1, "una sola linea: {:?}", text);
        assert!(!text.contains('\0'), "{:?}", text);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
