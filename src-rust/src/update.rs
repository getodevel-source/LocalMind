//! Actualización automática de OMNI (canal GitHub Releases).
//!
//! - Origen: `GET {feed}/latest` (por defecto el repo del binario:
//!   `getodevel-source/OMNI`, release GitHub). El JSON trae
//!   `tag_name`, `body` y `assets` (`name` + `browser_download_url`).
//! - Elegibilidad: el asset debe llamarse `OMNI-portable-<semver>-<fecha>.zip`
//!   y su versión ser MAYOR que `CARGO_PKG_VERSION` (comparación semver
//!   estricta; los tags que no parseen se ignoran).
//! - Descarga: a `%TEMP%\omni-update\<tag>\<asset>` con resume (`Range`) y
//!   verificación por tamaño + `sha256:<hex>` leído del `body` de la release
//!   (línea `SHA256 <asset> <hex>`). Sin checksum en el body: se rechaza.
//! - Instalación: el ZIP se extrae a staging y se aplica con un swap de
//!   directorios (`app` → `app.prev`, staging → `app`) ejecutado por un
//!   `cmd` desacoplado tras salir la app (el exe en marcha no se puede
//!   sobrescribir en Windows). `models/` y `%APPDATA%\LocalMind` jamás se
//!   tocan. Revierte al `.prev` si el swap falla a medias.
//! - Sin `unwrap` en rutas de request/IO/JSON/red: todo error es `Err(String)`
//!   en español. Sin dependencias nuevas: `ureq` + `zip` ya están en el árbol.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

// ---------------------------------------------------------------------------
// Configuración del canal
// ---------------------------------------------------------------------------

/// Repo por defecto del canal (dueño del binario). El repo se mudó de
/// `LocalMind` a `OMNI`: el default apunta al nombre vivo (GitHub redirige el
/// viejo, pero no se depende de la redirección).
pub const DEFAULT_FEED_REPO: &str = "getodevel-source/OMNI";

/// URL base de la API de releases (testeable: los tests apuntan a un stub).
pub fn releases_api_base() -> String {
    std::env::var("OMNI_UPDATE_API").unwrap_or_else(|_| "https://api.github.com".to_string())
}

/// URL del `latest` para un repo `owner/name`.
pub fn latest_url(repo: &str, api_base: &str) -> String {
    format!(
        "{}/repos/{}/releases/latest",
        api_base.trim_end_matches('/'),
        repo
    )
}

/// Repo efectivo: configurado o el defecto. Vacío = defecto.
pub fn feed_repo(configured: &str) -> Option<String> {
    let r = if configured.trim().is_empty() {
        DEFAULT_FEED_REPO.to_string()
    } else {
        configured.trim().to_string()
    };
    if r.is_empty() {
        None
    } else {
        Some(r)
    }
}

/// ¿Versión candidata válida? `X.Y.Z` estricto (semver; sin `v`).
pub fn parse_version(v: &str) -> Option<semver::Version> {
    let t = v.trim().trim_start_matches('v').trim();
    semver::Version::parse(t).ok()
}

/// ¿`candidate` es más nueva que `current`? Ambas `X.Y.Z` estrictas.
pub fn is_newer(current: &str, candidate: &str) -> bool {
    match (parse_version(current), parse_version(candidate)) {
        (Some(a), Some(b)) => b > a,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Release remota: parseo puro del JSON de `releases/latest`
// ---------------------------------------------------------------------------

/// Asset elegible: nombre `OMNI-portable-<semver>-<fecha>.zip` + URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAsset {
    pub version: String,
    pub name: String,
    pub url: String,
}

/// Extraer `(versión, fecha)` de `OMNI-portable-2.1.0-20261008.zip`.
/// `None` = no es un asset del canal.
pub fn parse_asset_name(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix("OMNI-portable-")?.strip_suffix(".zip")?;
    let (ver, date) = rest.rsplit_once('-')?;
    parse_version(ver)?;
    if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((ver.trim_start_matches('v').to_string(), date.to_string()))
}

/// Mejor asset elegible del `latest`: el de mayor versión > `current`.
/// Puro: se testea con JSON capturado, sin red.
pub fn pick_update_asset(latest_json: &str, current: &str) -> Option<UpdateAsset> {
    let v: serde_json::Value = serde_json::from_str(latest_json).ok()?;
    let body = v
        .get("body")
        .and_then(|b| b.as_str())
        .unwrap_or("")
        .to_string();
    let assets = v.get("assets")?.as_array()?;
    let mut best: Option<UpdateAsset> = None;
    for a in assets {
        let name = a.get("name").and_then(|n| n.as_str()).unwrap_or("");
        let url = a
            .get("browser_download_url")
            .and_then(|u| u.as_str())
            .unwrap_or("");
        let (ver, _) = match parse_asset_name(name) {
            Some(p) => p,
            None => continue,
        };
        if url.is_empty() || !is_newer(current, &ver) {
            continue;
        }
        let replace = match &best {
            None => true,
            Some(b) => is_newer(&b.version, &ver),
        };
        if replace {
            best = Some(UpdateAsset {
                version: ver,
                name: name.to_string(),
                url: url.to_string(),
            });
        }
    }
    let b = best?;
    // El checksum vive en el body: `SHA256 <asset> <hex>`. Sin él no hay
    // instalación (el ZIP sin verificar jamás se aplica).
    if find_checksum(&body, &b.name).is_some() {
        Some(b)
    } else {
        None
    }
}

/// `SHA256 <asset> <64hex>` en el body de la release (una línea).
pub fn find_checksum(body: &str, asset: &str) -> Option<String> {
    for line in body.lines() {
        let mut it = line.split_whitespace();
        match (it.next(), it.next(), it.next()) {
            (Some("SHA256"), Some(name), Some(hex)) if name == asset => {
                let h = hex.trim().to_lowercase();
                if h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Some(h);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Estado del trabajo (un solo job; global del proceso)
// ---------------------------------------------------------------------------

/// `idle|checking|available|downloading|ready|installing|done|error`.
#[derive(Debug, Clone)]
pub struct UpdateState {
    pub state: String,
    pub current: String,
    pub latest: String,
    pub notes: String,
    pub percent: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub error: String,
    /// Minutos hasta el próximo chequeo programado (solo informativo).
    pub next_check_mins: u64,
}

impl UpdateState {
    pub fn idle() -> Self {
        Self {
            state: "idle".to_string(),
            current: env!("CARGO_PKG_VERSION").to_string(),
            latest: String::new(),
            notes: String::new(),
            percent: 0,
            bytes_done: 0,
            bytes_total: 0,
            error: String::new(),
            next_check_mins: 0,
        }
    }

    pub fn json(&self) -> String {
        serde_json::json!({
            "state": self.state,
            "current": self.current,
            "latest": self.latest,
            "notes": self.notes,
            "percent": self.percent,
            "bytes_done": self.bytes_done,
            "bytes_total": self.bytes_total,
            "error": self.error,
            "next_check_mins": self.next_check_mins,
        })
        .to_string()
    }
}

struct UpdateInner {
    state: Mutex<UpdateState>,
    busy: AtomicBool,
    cancel: AtomicBool,
    bytes_n: std::sync::atomic::AtomicU64,
    last_check_secs: std::sync::atomic::AtomicU64,
    pending: Mutex<Option<PendingUpdate>>,
}

/// Paquete descargado y verificado, listo para instalar al reiniciar.
/// `zip_path`/`notes` son forenses (origen y notas de la release).
#[derive(Debug, Clone)]
pub struct PendingUpdate {
    pub version: String,
    #[allow(dead_code)]
    pub zip_path: String,
    pub staging_dir: String,
    #[allow(dead_code)]
    pub notes: String,
}

static UPD: LazyLock<UpdateInner> = LazyLock::new(|| UpdateInner {
    state: Mutex::new(UpdateState::idle()),
    busy: AtomicBool::new(false),
    cancel: AtomicBool::new(false),
    bytes_n: std::sync::atomic::AtomicU64::new(0),
    last_check_secs: std::sync::atomic::AtomicU64::new(0),
    pending: Mutex::new(None),
});

fn inner() -> &'static UpdateInner {
    &UPD
}

pub fn update_snapshot() -> UpdateState {
    let mut s = inner()
        .state
        .lock()
        .map(|g| g.clone())
        .unwrap_or_else(|p| p.into_inner().clone());
    s.bytes_done = inner().bytes_n.load(Ordering::Relaxed);
    s.percent = match s.bytes_done.saturating_mul(100).checked_div(s.bytes_total) {
        Some(p) => p.min(100),
        None => {
            if s.state == "ready" || s.state == "done" {
                100
            } else {
                0
            }
        }
    };
    s
}

fn update_set(f: impl FnOnce(&mut UpdateState)) {
    if let Ok(mut g) = inner().state.lock() {
        f(&mut g);
    }
}

/// ¿Hay trabajo de update en curso? (para 409 ante un segundo POST).
pub fn update_busy() -> bool {
    inner().busy.load(Ordering::Relaxed)
}

pub fn pending_update() -> Option<PendingUpdate> {
    inner().pending.lock().ok()?.clone()
}

fn set_pending(p: Option<PendingUpdate>) {
    if let Ok(mut g) = inner().pending.lock() {
        *g = p;
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Minutos desde el último chequeo (u64::MAX si nunca se chequeó).
#[allow(dead_code)]
pub fn mins_since_check() -> u64 {
    let last = inner().last_check_secs.load(Ordering::Relaxed);
    if last == 0 {
        return u64::MAX;
    }
    now_secs().saturating_sub(last) / 60
}
// ---------------------------------------------------------------------------
// Red: `latest` + descarga con resume
// ---------------------------------------------------------------------------

/// Espacio libre en bytes del volumen que contiene `path` (Windows, best-effort).
/// Sin APIs nuevas: `fsutil volume diskfree` y parseo de `Byte libres`. `None`
/// si no se pudo medir (el llamador NO bloquea: sin dato no hay chequeo).
pub fn free_bytes_for(path: &std::path::Path) -> Option<u64> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let anchor = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let vol = anchor
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .unwrap_or_else(|| "\\".to_string());
    let out = std::process::Command::new("fsutil")
        .args(["volume", "diskfree", vol.as_str()])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // Forma: "Byte libres             : 123456789". Tomar la primera línea con
    // dígitos tras los dos puntos que mencione libres/free.
    for line in text.lines() {
        let low = line.to_lowercase();
        if !(low.contains("libr") || low.contains("free")) {
            continue;
        }
        if let Some(after) = line.split(':').nth(1) {
            let digits: String = after.chars().filter(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<u64>() {
                return Some(n);
            }
        }
    }
    None
}

/// ¿Cabe una descarga de `need` bytes con margen 2×? `None` en tamaño o en
/// espacio = sin veredicto (`None`): el llamador sigue sin bloquear.
pub fn fits_download(need: Option<u64>, free: Option<u64>) -> Option<bool> {
    match (need, free) {
        (Some(n), Some(f)) => Some(f >= n.saturating_mul(2)),
        _ => None,
    }
}

/// Barrer stagings huérfanos de arranques previos (apagón duro entre la
/// descarga y el swap): todo `%TEMP%\omni-update\<tag>\staging` con
/// `OMNI.exe` + `ui.html` se registra como `pending` para que el próximo
/// reinicio normal lo instale. Devuelve las versiones recuperadas.
/// Puro en efectos salvo el `set_pending`: sin red, sin borrados.
pub fn recover_stale_stagings() -> Vec<String> {
    let mut found = Vec::new();
    let mut base = std::env::temp_dir();
    base.push("omni-update");
    let dirs = std::fs::read_dir(&base)
        .map(|r| r.filter_map(|e| e.ok()).collect::<Vec<_>>())
        .unwrap_or_default();
    for d in dirs {
        let staging = d.path().join("staging");
        let ok = staging.join("OMNI.exe").is_file() && staging.join("ui.html").is_file();
        if !ok {
            continue;
        }
        let tag = d.file_name().to_string_lossy().to_string();
        let ver = tag.trim_start_matches('v').to_string();
        if parse_version(&ver).is_none() {
            continue;
        }
        set_pending(Some(PendingUpdate {
            version: ver.clone(),
            zip_path: String::new(),
            staging_dir: staging.to_string_lossy().to_string(),
            notes: "recuperado de un staging previo".to_string(),
        }));
        update_set(|s| {
            s.state = "ready".to_string();
            s.latest = ver.clone();
            s.percent = 100;
        });
        found.push(ver);
    }
    found
}

fn get_text(url: &str, timeout_secs: u64) -> Result<String, String> {
    ureq::get(url)
        .set("User-Agent", &format!("OMNI/{}", env!("CARGO_PKG_VERSION")))
        .set("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, resp) => {
                let mut b = String::new();
                let _ = resp.into_reader().read_to_string(&mut b);
                format!(
                    "El canal de actualizaciones devolvió {}: {}",
                    code,
                    snippet(&b)
                )
            }
            ureq::Error::Transport(t) => {
                format!("Error de red con el canal de actualizaciones: {}", t)
            }
        })?
        .into_string()
        .map_err(|e| format!("Error al leer el canal de actualizaciones: {}", e))
}

fn snippet(s: &str) -> String {
    const MAX: usize = 200;
    let t = s.trim();
    if t.len() <= MAX {
        t.to_string()
    } else {
        format!("{}…", &t[..MAX])
    }
}

/// Directorio de trabajo del update: `%TEMP%\omni-update\<tag>`.
pub fn update_work_dir(tag: &str) -> PathBuf {
    let mut base = std::env::temp_dir();
    base.push("omni-update");
    base.push(safe_tag(tag));
    base
}

/// Saneado del tag para usarlo como nombre de dir (sin `..` ni separadores).
fn safe_tag(tag: &str) -> String {
    let t: String = tag
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if t.is_empty() || t == "." || t == ".." {
        "update".to_string()
    } else {
        t.chars().take(64).collect()
    }
}

/// Chequear el canal (hilo worker). Devuelve el asset si hay algo nuevo y
/// verificado; actualiza el estado global en todos los casos.
pub fn check_for_update(
    repo: &str,
    api_base: &str,
    current: &str,
) -> Result<Option<UpdateAsset>, String> {
    let url = latest_url(repo, api_base);
    let body = get_text(&url, 20)?;
    match pick_update_asset(&body, current) {
        Some(a) => {
            inner().last_check_secs.store(now_secs(), Ordering::Relaxed);
            Ok(Some(a))
        }
        None => {
            inner().last_check_secs.store(now_secs(), Ordering::Relaxed);
            Ok(None)
        }
    }
}

/// Descargar el asset con resume a `dest` (`.part` + rename). Informa
/// progreso en el estado global. Verifica tamaño y sha256 esperados.
pub fn download_asset(
    url: &str,
    dest: &Path,
    expected_size: Option<u64>,
    expected_sha256: &str,
    cancel: &AtomicBool,
) -> Result<u64, String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("No se pudo crear {}: {}", parent.display(), e))?;
    }
    // Tamaño esperado: del asset remoto si se conoce; si no, del `.part`.
    let part = dest.with_extension("part");
    let mut start: u64 = 0;
    if let Ok(meta) = std::fs::metadata(&part) {
        start = meta.len();
        if expected_size.is_some_and(|n| start > n) {
            let _ = std::fs::remove_file(&part);
            start = 0;
        }
    }
    let mut req = ureq::get(url)
        .set("User-Agent", &format!("OMNI/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(3600));
    if start > 0 {
        req = req.set("Range", &format!("bytes={}-", start));
    }
    let resp = req.call().map_err(|e| match e {
        ureq::Error::Status(code, r) => {
            let mut b = String::new();
            let _ = r.into_reader().read_to_string(&mut b);
            format!("La descarga devolvió {}: {}", code, snippet(&b))
        }
        ureq::Error::Transport(t) => format!("Error de red al descargar la actualización: {}", t),
    })?;
    let resumed = resp.status() == 206;
    if !resumed {
        start = 0;
        let _ = std::fs::remove_file(&part);
    }
    if let Ok(len_str) = resp.header("Content-Length").unwrap_or("").parse::<u64>() {
        let total = len_str + start;
        update_set(|s| {
            s.bytes_total = total;
        });
    } else if let Some(n) = expected_size {
        update_set(|s| {
            s.bytes_total = n;
        });
    }
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(resumed)
        .write(!resumed)
        .truncate(!resumed)
        .open(&part)
        .map_err(|e| format!("No se pudo escribir {}: {}", part.display(), e))?;
    let mut reader = resp.into_reader();
    let mut buf = [0u8; 65536];
    let mut written = start;
    inner().bytes_n.store(start, Ordering::Relaxed);
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(out);
            return Err("cancelado".to_string());
        }
        use std::io::{Read, Write};
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Error al descargar la actualización: {}", e))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .map_err(|e| format!("Error al guardar {}: {}", part.display(), e))?;
        written += n as u64;
        inner().bytes_n.fetch_add(n as u64, Ordering::Relaxed);
    }
    drop(out);
    let meta = std::fs::metadata(&part)
        .map_err(|e| format!("No se pudo medir {}: {}", part.display(), e))?;
    if let Some(n) = expected_size {
        if meta.len() != n {
            let _ = std::fs::remove_file(&part);
            return Err(format!(
                "Tamaño inesperado en la actualización: {} bytes frente a {} esperados",
                meta.len(),
                n
            ));
        }
    }
    let got = crate::models::sha256_file(&part)?;
    if got != expected_sha256.to_lowercase() {
        let _ = std::fs::remove_file(&part);
        return Err(
            "SHA-256 no coincide en la actualización (descarga corrupta o manipulada)".to_string(),
        );
    }
    std::fs::rename(&part, dest)
        .map_err(|e| format!("No se pudo mover la actualización: {}", e))?;
    Ok(written)
}

// ---------------------------------------------------------------------------
// Instalación: staging + swap al reiniciar
// ---------------------------------------------------------------------------

/// Entradas del runtime que el swap reemplaza (relativas a la app).
/// `models/` y los datos de `%APPDATA%` jamás se tocan. El swap mueve el
/// directorio completo, así que la lista es documentación del contrato.
#[allow(dead_code)]
pub const RUNTIME_TOP_LEVEL: &[&str] = &[
    "OMNI.exe",
    "ui.html",
    "ui_fallback.html",
    "omni.ico",
    "omni.png",
    "bin",
    "version.txt",
    "LEEME.txt",
];

/// Extraer el ZIP verificado a `staging/`, validando su contenido:
/// debe traer `OMNI.exe` y `ui.html`; se IGNORA (omite, no falla) cualquier
/// entrada bajo `models/` —el ZIP oficial no trae pesos ni puntero, pero un
/// ZIP viejo con `models/PON_TUS_MODELOS_AQUI.txt` (v2.0.4, que rompía el
/// canal) debe instalar igual sin tocar los modelos del usuario.
/// Se rechaza: `..`, rutas absolutas y cualquier `.gguf` fuera de models/
/// (un ZIP que intente colar pesos en otra ruta no es del canal).
pub fn stage_zip(zip_path: &Path, staging: &Path) -> Result<(), String> {
    let f = std::fs::File::open(zip_path)
        .map_err(|e| format!("No se pudo abrir {}: {}", zip_path.display(), e))?;
    let mut archive =
        zip::ZipArchive::new(f).map_err(|e| format!("ZIP de actualización inválido: {}", e))?;
    if archive.is_empty() {
        return Err("ZIP de actualización vacío".to_string());
    }
    let mut seen_exe = false;
    let mut seen_ui = false;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("ZIP de actualización ilegible: {}", e))?;
        let name = entry.name().replace('\\', "/");
        if name.is_empty() || name.starts_with('/') || name.contains("..") {
            return Err(format!(
                "Entrada insegura en la actualización: {}",
                entry.name()
            ));
        }
        let lower = name.to_lowercase();
        // models/ se omite: ni el puntero ni (mucho menos) pesos pisan lo del
        // usuario. El swap mueve el dir completo, así que lo omitido en staging
        // se repone desde el respaldo (ver `swap_script` + healing).
        if lower == "models" || lower.starts_with("models/") {
            continue;
        }
        // `.gguf` fuera de models/ = ZIP ajeno al canal: se rechaza.
        if lower.ends_with(".gguf") {
            return Err(format!(
                "La actualización trae un peso fuera de models/: {}",
                entry.name()
            ));
        }
        if name == "OMNI.exe" {
            seen_exe = true;
        }
        if name == "ui.html" {
            seen_ui = true;
        }
        let out = staging.join(&name);
        // Contención: el destino debe quedar dentro del staging.
        let root: Vec<_> = staging.components().collect();
        let mut cur: Vec<_> = Vec::new();
        for c in out.components() {
            use std::path::Component;
            match c {
                Component::ParentDir => return Err(format!("Ruta fuera del staging: {}", name)),
                Component::CurDir => {}
                _ => cur.push(c),
            }
        }
        if cur.len() < root.len() {
            return Err(format!("Ruta fuera del staging: {}", name));
        }
        if entry.is_dir() {
            std::fs::create_dir_all(&out)
                .map_err(|e| format!("No se pudo preparar {}: {}", out.display(), e))?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("No se pudo preparar {}: {}", parent.display(), e))?;
            }
            let mut w = std::fs::File::create(&out)
                .map_err(|e| format!("No se pudo escribir {}: {}", out.display(), e))?;
            std::io::copy(&mut entry, &mut w)
                .map_err(|e| format!("No se pudo extraer {}: {}", name, e))?;
        }
    }
    if !seen_exe || !seen_ui {
        return Err("La actualización no trae OMNI.exe + ui.html".to_string());
    }
    Ok(())
}

/// Generar el script de swap que el `cmd` desacoplado ejecuta tras la
/// salida: `app` → `app.prev-<ver>` (respaldo), `staging` → `app`, y
/// arranque del exe nuevo. Revierte al respaldo si el segundo rename falla.
/// `models/` se repone desde el respaldo tras el swap: el staging nunca trae
/// models/ (stage_zip la omite), así que sin esta línea el dir nuevo quedaría
/// sin carpeta de modelos. Puro (el llamador lo escribe y lo lanza).
pub fn swap_script(app_dir: &Path, staging_dir: &Path, version: &str) -> String {
    let prev = app_dir.with_extension(format!("prev-{}", safe_tag(version)));
    let prev_models = prev.join("models");
    // `ping -n 3` ≈ 2 s de espera a que el exe salga y libere el lock.
    // `robocopy prev/models app/models /E` repone pesos + puntero del usuario;
    // `if exist` guarda el caso de instalación sin models/ previa.
    format!(
        "@echo off\r\nping -n 3 127.0.0.1 >nul\r\nif exist \"{prev}\" rd /s /q \"{prev}\"\r\nmove \"{app}\" \"{prev}\" >nul\r\nif errorlevel 1 exit /b 1\r\nmove \"{staging}\" \"{app}\" >nul\r\nif errorlevel 1 move \"{prev}\" \"{app}\" >nul\r\nif errorlevel 1 exit /b 1\r\nif exist \"{prev_models}\" robocopy \"{prev_models}\" \"{app_models}\" /E /NFL /NDL >nul\r\nstart \"\" \"{exe}\"\r\ndel \"%~f0\"\r\n",
        app = app_dir.display(),
        prev = prev.display(),
        staging = staging_dir.display(),
        prev_models = prev_models.display(),
        app_models = app_dir.join("models").display(),
        exe = app_dir.join("OMNI.exe").display(),
    )
}

/// ¿Instalación en curso o pendiente? (para bloquear reinstalaciones encima).
#[allow(dead_code)]
pub fn update_pending_or_busy() -> bool {
    update_busy() || pending_update().is_some()
}

/// Salida ordenada pedida por `POST /api/update/restart` (P0 2026-10-09): el
/// handler HTTP no puede salir del event loop de tao; arma este flag y el
/// tick del loop (cada 5 s en `main.rs`) ejecuta el `do_quit` real (stop +
/// swap + Exit). `take` atómico: un solo disparo.
static RESTART_ARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Armar la salida ordenada para instalar (la ejecuta el loop, no el handler).
pub fn arm_restart() {
    RESTART_ARMED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// ¿Pidió la UI reiniciar para instalar? Consume el flag (un disparo).
pub fn take_restart_armed() -> bool {
    RESTART_ARMED.swap(false, std::sync::atomic::Ordering::SeqCst)
}

/// Reponer `models/` desde el respaldo `.prev-<ver>` si falta junto al exe
/// (healing al arrancar): si el swap instaló un dir sin modelos (ZIPs viejos
/// con staging sin models/ + script sin robocopy), la app arranca igual por
/// la rama 3 de `get_base_dir`, pero el motor no encuentra `.gguf`. Puro en
/// decisión (`heal_models_decision`), efecto (`heal_models`) separado.
/// Devuelve el origen repuesto o `None` si no había nada que hacer.
pub fn heal_models_decision(app_dir: &Path) -> Option<PathBuf> {
    if app_dir.join("models").is_dir() {
        return None;
    }
    let parent = app_dir.parent()?;
    let stem = app_dir.file_name()?.to_string_lossy().to_string();
    let mut best: Option<PathBuf> = None;
    if let Ok(entries) = std::fs::read_dir(parent) {
        for e in entries.flatten() {
            let p = e.path();
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if p.is_dir()
                && name.starts_with(&format!("{}.prev-", stem))
                && p.join("models").is_dir()
            {
                best = Some(p);
            }
        }
    }
    best
}

/// Efecto del healing: copia `prev/models` → `app/models` (solo lo ausente).
/// Best-effort: si falla, `Err` en español y la app sigue (el motor dirá qué
/// falta al arrancar).
pub fn heal_models(app_dir: &Path) -> Result<Option<String>, String> {
    let prev = match heal_models_decision(app_dir) {
        Some(p) => p,
        None => return Ok(None),
    };
    let src = prev.join("models");
    let dst = app_dir.join("models");
    copy_dir_contents(&src, &dst).map_err(|e| format!("No se pudo reponer models/: {}", e))?;
    Ok(Some(prev.to_string_lossy().to_string()))
}

/// Copia recursiva de contenidos (sin mover el dir): crea lo ausente, no
/// pisa ficheros existentes (los pesos del usuario mandan).
fn copy_dir_contents(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let name = e.file_name();
        let s = e.path();
        let d = dst.join(&name);
        let ft = e.file_type()?;
        if ft.is_dir() {
            copy_dir_contents(&s, &d)?;
        } else if !d.exists() {
            std::fs::copy(&s, &d)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Orquestación de alto nivel (hilos worker; el llamador es `server.rs`)
// ---------------------------------------------------------------------------

/// Lanzar chequeo en segundo plano. `feed`: repo `owner/name` (vacío =
/// defecto). `auto`: marca el estado como chequeo automático (silencioso).
/// `log`: logger con niveles por prefijo (`[ERROR]`/`[WARN]` van a disco con
/// nivel; el resto como INFO). Los llama `server.rs` con `mgr.log_*`.
pub fn spawn_check(feed: String, auto: bool, log: Arc<dyn Fn(String) + Send + Sync>) {
    if inner().busy.swap(true, Ordering::SeqCst) {
        return;
    }
    inner().cancel.store(false, Ordering::Relaxed);
    let current = env!("CARGO_PKG_VERSION").to_string();
    update_set(|s| {
        s.state = "checking".to_string();
        s.error = String::new();
        if !auto {
            s.latest = String::new();
            s.notes = String::new();
        }
    });
    std::thread::spawn(move || {
        let repo = feed_repo(&feed).unwrap_or_else(|| DEFAULT_FEED_REPO.to_string());
        let api = releases_api_base();
        match check_for_update(&repo, &api, &current) {
            Ok(Some(a)) => {
                // Notas cortas del body (primera línea no vacía, ≤280 chars).
                update_set(|s| {
                    s.state = "available".to_string();
                    s.current = current.clone();
                    s.latest = a.version.clone();
                    s.notes = String::new();
                    s.next_check_mins = 0;
                });
                if !auto {
                    log(format!(
                        "[LocalMind] Actualización disponible: OMNI {} (actual {}).",
                        a.version, current
                    ));
                }
            }
            Ok(None) => {
                update_set(|s| {
                    s.state = "idle".to_string();
                    s.current = current.clone();
                    s.latest = current.clone();
                    s.next_check_mins = 360;
                });
                if !auto {
                    log("[LocalMind] OMNI está al día.".to_string());
                }
            }
            Err(e) => {
                update_set(|s| {
                    s.state = "error".to_string();
                    s.error = e.clone();
                    s.next_check_mins = 60;
                });
                if !auto {
                    log(format!(
                        "[LocalMind] No se pudo comprobar actualizaciones: {}",
                        e
                    ));
                }
            }
        }
        inner().busy.store(false, Ordering::SeqCst);
    });
}

/// Lanzar descarga + verificación + staging en segundo plano.
/// Requiere un chequeo previo con asset (`latest`, `url`, `size`, `sha256`).
pub fn spawn_download(
    asset_name: String,
    asset_url: String,
    version: String,
    size: Option<u64>,
    sha256: String,
    notes: String,
    log: Arc<dyn Fn(String) + Send + Sync>,
) {
    if inner().busy.swap(true, Ordering::SeqCst) {
        return;
    }
    inner().cancel.store(false, Ordering::Relaxed);
    inner().bytes_n.store(0, Ordering::Relaxed);
    update_set(|s| {
        s.state = "downloading".to_string();
        s.latest = version.clone();
        s.notes = notes.clone();
        s.error = String::new();
        s.bytes_total = size.unwrap_or(0);
    });
    std::thread::spawn(move || {
        let err: Option<String> = (|| -> Option<String> {
            let work = update_work_dir(&version);
            let dest = work.join(&asset_name);
            let staging = work.join("staging");
            let _ = std::fs::remove_dir_all(&staging);
            if let Err(e) = download_asset(&asset_url, &dest, size, &sha256, &inner().cancel) {
                return Some(e);
            }
            if let Err(e) = stage_zip(&dest, &staging) {
                return Some(e);
            }
            set_pending(Some(PendingUpdate {
                version: version.clone(),
                zip_path: dest.to_string_lossy().to_string(),
                staging_dir: staging.to_string_lossy().to_string(),
                notes: notes.clone(),
            }));
            None
        })();
        match err {
            None => {
                update_set(|s| {
                    s.state = "ready".to_string();
                    s.percent = 100;
                    s.error = String::new();
                });
                log(format!(
                    "[LocalMind] Actualización {} lista: se instalará al reiniciar la app.",
                    version
                ));
            }
            Some(e) => {
                set_pending(None);
                update_set(|s| {
                    s.state = "error".to_string();
                    s.error = e.clone();
                });
                log(format!(
                    "[LocalMind] Error al descargar la actualización: {}",
                    e
                ));
            }
        }
        inner().busy.store(false, Ordering::SeqCst);
    });
}

/// Cancelar el trabajo en curso (chequeo o descarga).
pub fn update_cancel() -> UpdateState {
    inner().cancel.store(true, Ordering::Relaxed);
    update_snapshot()
}

/// Preparar la instalación al salir: escribe el script de swap en `%TEMP%`
/// y devuelve su ruta. El llamador (`main.rs`) lo lanza desacoplado tras
/// detener el motor y salir del bucle de eventos.
///
/// Cinturón P0 (2026-10-09, drill real): `app_dir` DEBE ser el directorio del
/// ejecutable en ejecución. Si apunta al repo de desarrollo (contiene
/// `src-rust/Cargo.toml` o `.git`) o no contiene `OMNI.exe`, se rechaza: el
/// swap movería el árbol equivocado y la app quedaría pidiendo reinicio para
/// siempre. Puro y testeable vía `check_app_dir`.
pub fn prepare_install_on_exit(app_dir: &Path) -> Result<PathBuf, String> {
    check_app_dir(app_dir)?;
    let p =
        pending_update().ok_or_else(|| "No hay actualización lista para instalar".to_string())?;
    let staging = PathBuf::from(&p.staging_dir);
    if !staging.is_dir() {
        return Err("El paquete de actualización ya no está (staging perdido)".to_string());
    }
    let script = swap_script(app_dir, &staging, &p.version);
    let path = std::env::temp_dir().join(format!("omni-install-{}.cmd", safe_tag(&p.version)));
    std::fs::write(&path, script)
        .map_err(|e| format!("No se pudo preparar la instalación: {}", e))?;
    update_set(|s| {
        s.state = "installing".to_string();
    });
    Ok(path)
}

/// ¿Es `app_dir` un directorio de instalación válido? (puro, testeable).
/// Válido = contiene `OMNI.exe` y NO es un árbol de desarrollo (sin
/// `src-rust/Cargo.toml` ni `.git`). El llamador (`main.rs`) pasa SIEMPRE el
/// dir del exe en ejecución (`app_dir()`), nunca el `base_dir` de datos.
pub fn check_app_dir(app_dir: &Path) -> Result<(), String> {
    if !app_dir.join("OMNI.exe").is_file() {
        return Err(format!(
            "Directorio de instalación inválido (sin OMNI.exe): {}",
            app_dir.display()
        ));
    }
    if app_dir.join("src-rust").join("Cargo.toml").is_file() || app_dir.join(".git").exists() {
        return Err(format!(
            "Directorio de instalación inválido (árbol de desarrollo, no se toca): {}",
            app_dir.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LATEST_FIXTURE: &str = r#"{
        "tag_name": "v2.1.0",
        "body": "Novedades\nSHA256 OMNI-portable-2.1.0-20261008.zip abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\n",
        "assets": [
            {"name": "OMNI-portable-2.1.0-20261008.zip", "browser_download_url": "https://example.invalid/a.zip", "size": 123},
            {"name": "LEEME.txt", "browser_download_url": "https://example.invalid/l.txt", "size": 9}
        ]
    }"#;

    #[test]
    fn versiones_semver_estricto() {
        assert!(is_newer("2.0.0", "2.1.0"));
        assert!(!is_newer("2.1.0", "2.1.0"));
        assert!(!is_newer("2.1.0", "2.0.9"));
        assert!(parse_version("v2.1.0").is_some());
        assert!(parse_version("2.1").is_none());
        assert!(!is_newer("2.0.0", "no-version"));
    }

    /// El canal default apunta al repo vivo (`OMNI`, no el viejo `LocalMind`
    /// mudado): un tercero con config limpia chequea contra el repo real.
    #[test]
    fn canal_default_apunta_al_repo_omni() {
        assert_eq!(DEFAULT_FEED_REPO, "getodevel-source/OMNI");
        assert_eq!(feed_repo(""), Some(DEFAULT_FEED_REPO.to_string()));
        assert_eq!(
            latest_url(DEFAULT_FEED_REPO, "https://api.github.com"),
            "https://api.github.com/repos/getodevel-source/OMNI/releases/latest"
        );
    }

    #[test]
    fn asset_elegible_y_checksum() {
        assert_eq!(
            parse_asset_name("OMNI-portable-2.1.0-20261008.zip"),
            Some(("2.1.0".to_string(), "20261008".to_string()))
        );
        assert_eq!(parse_asset_name("OMNI-portable-2.1-20261008.zip"), None);
        assert_eq!(parse_asset_name("otro-2.1.0-20261008.zip"), None);
        assert_eq!(parse_asset_name("OMNI-portable-2.1.0-ayer.zip"), None);
        let a = pick_update_asset(LATEST_FIXTURE, "2.0.0").expect("debe elegir el zip");
        assert_eq!(a.version, "2.1.0");
        assert!(pick_update_asset(LATEST_FIXTURE, "2.1.0").is_none());
        assert!(pick_update_asset(LATEST_FIXTURE, "9.9.9").is_none());
        // Sin línea SHA256 no hay instalación.
        let no_sum = LATEST_FIXTURE.replace("SHA256 ", "NOSUM ");
        assert!(pick_update_asset(&no_sum, "2.0.0").is_none());
    }

    #[test]
    fn swap_repone_models_y_no_toca_appdata() {
        let app = Path::new("C:\\OMNI");
        let st = Path::new("C:\\TEMP\\staging");
        let s = swap_script(app, st, "2.1.0");
        assert!(s.contains("OMNI.exe"));
        // models/ se REPONE desde el respaldo (robocopy prev→app), no se pisa:
        // el staging nunca la trae y sin esta línea el dir nuevo quedaría sin
        // carpeta de modelos.
        assert!(s.contains("robocopy"), "{}", s);
        assert!(s.contains("prev-2.1.0"), "{}", s);
        assert!(s.to_lowercase().contains("models"), "{}", s);
        assert!(!s.to_lowercase().contains("appdata"));
    }

    #[test]
    fn staging_rechaza_zip_hostil() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("omni-upd-test-{}", now_secs()));
        let _ = std::fs::create_dir_all(&dir);
        let zp = dir.join("evil.zip");
        {
            let f = std::fs::File::create(&zp).unwrap();
            let mut w = zip::ZipWriter::new(f);
            w.start_file("../evil.exe", zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(b"x").unwrap();
            w.finish().unwrap();
        }
        let out = stage_zip(&zp, &dir.join("staging"));
        assert!(out.is_err(), "el zip con .. debe rechazarse");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// El ZIP viejo con `models/PON_TUS_MODELOS_AQUI.txt` (v2.0.4, rompía el
    /// canal) instala igual: models/ se omite, exe+ui pasan.
    #[test]
    fn staging_omite_models_y_acepta_puntero_viejo() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("omni-upd-skip-{}", now_secs()));
        let _ = std::fs::create_dir_all(&dir);
        let zp = dir.join("old.zip");
        {
            let f = std::fs::File::create(&zp).unwrap();
            let mut w = zip::ZipWriter::new(f);
            for name in [
                "OMNI.exe",
                "ui.html",
                "models/PON_TUS_MODELOS_AQUI.txt",
                "models/",
            ] {
                if name.ends_with('/') {
                    w.add_directory(name, zip::write::SimpleFileOptions::default())
                        .unwrap();
                } else {
                    w.start_file(name, zip::write::SimpleFileOptions::default())
                        .unwrap();
                    w.write_all(b"x").unwrap();
                }
            }
            w.finish().unwrap();
        }
        let stg = dir.join("staging");
        stage_zip(&zp, &stg).expect("el puntero viejo no debe romper el canal");
        assert!(stg.join("OMNI.exe").is_file());
        assert!(stg.join("ui.html").is_file());
        assert!(!stg.join("models").exists(), "models/ no debe staged");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `.gguf` fuera de models/ = ZIP ajeno: se rechaza aunque traiga exe+ui.
    #[test]
    fn staging_rechaza_gguf_fuera_de_models() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("omni-upd-gguf-{}", now_secs()));
        let _ = std::fs::create_dir_all(&dir);
        let zp = dir.join("smug.zip");
        {
            let f = std::fs::File::create(&zp).unwrap();
            let mut w = zip::ZipWriter::new(f);
            for name in ["OMNI.exe", "ui.html", "pesos-colados.gguf"] {
                w.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                w.write_all(b"x").unwrap();
            }
            w.finish().unwrap();
        }
        let out = stage_zip(&zp, &dir.join("staging"));
        assert!(out.is_err(), "gguf fuera de models/ debe rechazarse");
        assert!(out.unwrap_err().contains("fuera de models/"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P0 drill real (2026-10-09): el swap intentó mover `C:\PROYECTOS\OMNI`
    /// (repo dev) en vez de la instalación y la app quedó pidiendo reinicio
    /// para siempre. `check_app_dir` rechaza: sin OMNI.exe, y árbol dev.
    #[test]
    fn check_app_dir_rechaza_repo_dev_y_dir_vacio() {
        let base = std::env::temp_dir().join(format!("omni-appdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // Dir con OMNI.exe pero con cara de repo dev → se rechaza.
        let dev = base.join("dev");
        std::fs::create_dir_all(dev.join("src-rust")).unwrap();
        std::fs::write(dev.join("OMNI.exe"), b"x").unwrap();
        std::fs::write(dev.join("src-rust").join("Cargo.toml"), b"[package]").unwrap();
        assert!(check_app_dir(&dev).is_err());
        // Dir sin OMNI.exe → se rechaza.
        let empty = base.join("vacio");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(check_app_dir(&empty).is_err());
        // Instalación válida → pasa.
        let app = base.join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("OMNI.exe"), b"x").unwrap();
        assert!(check_app_dir(&app).is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Healing: dir sin models/ + respaldo `.prev-<ver>` con modelos → decide
    /// el respaldo y `heal_models` copia sin pisar lo existente.
    #[test]
    fn heal_models_repone_desde_respaldo_sin_pisar() {
        let base = std::env::temp_dir().join(format!("omni-heal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let app = base.join("OMNI");
        let prev = base.join("OMNI.prev-2.0.4");
        std::fs::create_dir_all(app.join("bin")).unwrap();
        std::fs::create_dir_all(prev.join("models")).unwrap();
        std::fs::write(prev.join("models").join("a.gguf"), b"a").unwrap();
        std::fs::write(prev.join("models").join("b.gguf"), b"b").unwrap();
        assert_eq!(heal_models_decision(&app), Some(prev.clone()));
        let src = heal_models(&app).expect("healing");
        assert_eq!(src, Some(prev.to_string_lossy().to_string()));
        assert_eq!(
            std::fs::read(app.join("models").join("a.gguf")).unwrap(),
            b"a"
        );
        assert_eq!(
            std::fs::read(app.join("models").join("b.gguf")).unwrap(),
            b"b"
        );
        // Con models/ presente ya no hay nada que hacer.
        assert_eq!(heal_models_decision(&app), None);
        assert_eq!(heal_models(&app).unwrap(), None);
        let _ = std::fs::remove_dir_all(&base);
    }
    #[test]
    fn fits_exige_doble_y_sin_dato_no_bloquea() {
        assert_eq!(fits_download(Some(100), Some(200)), Some(true));
        assert_eq!(fits_download(Some(100), Some(199)), Some(false));
        assert_eq!(fits_download(None, Some(1)), None);
        assert_eq!(fits_download(Some(1), None), None);
        assert_eq!(fits_download(None, None), None);
    }

    #[test]
    fn free_bytes_no_aborta_y_recover_ignora_basura() {
        // Best-effort: puede dar Some o None según la máquina, pero nunca pánico.
        let _ = free_bytes_for(&std::env::temp_dir());
        // Un dir omni-update con staging incompleto no se recupera.
        let base = std::env::temp_dir().join("omni-update");
        let tag = format!("v9.9.9-test{}", std::process::id());
        let staging = base.join(&tag).join("staging");
        let _ = std::fs::create_dir_all(&staging);
        let _ = std::fs::write(staging.join("OMNI.exe"), b"x");
        // Falta ui.html → no se registra como pending. Como `recover` escanea
        // el %TEMP% real (puede haber stagings válidos de la máquina: el drill
        // P0 los dejó), se filtra por el tag propio en vez de exigir None.
        set_pending(None);
        let found = recover_stale_stagings();
        assert!(!found.iter().any(|v| v.contains("9.9.9")), "{:?}", found);
        set_pending(None);
        let _ = std::fs::remove_dir_all(base.join(&tag));
    }
}
