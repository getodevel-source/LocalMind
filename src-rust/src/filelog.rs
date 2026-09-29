use std::path::{Path, PathBuf};

/// Rotación de logs a archivo (P30): 1 MiB por archivo, 3 archivos
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// (`allow(dead_code)`: documenta el contrato de rotación para la UI futura.)
#[allow(dead_code)]
pub const KEEP_FILES: usize = 3;

/// Nombre del archivo de log (P30). Constante porque el hook de pánico y el
/// `ProcessManager` tienen que escribir en el MISMO archivo: si divergieran,
/// un panic quedaría en un log que nadie lee.
pub const LOG_FILE_NAME: &str = "localmind.log";

/// Directorio de logs: `%APPDATA%\LocalMind\logs`.
///
/// `base_dir` es SOLO el fallback para cuando `%APPDATA%` no está definido, y
/// esa precedencia no es negociable: el log tiene que vivir en el perfil del
/// usuario, no junto al exe (que puede estar en un directorio de programa sin
/// permiso de escritura, o en un portable que el usuario borra).
///
/// Por eso un `base_dir` de test NO redirige el log, y esta precedencia no se
/// relaja: quien necesite otra ruta la pide EXPLÍCITA por `log_file`, como
/// `ConfigStore::load_from_path` hace con la config. Un `ProcessManager`
/// construido sobre un temporal escribía "Servidor detenido" en el log real
/// del dueño al soltar su `Drop` (medido 2026-09-28): un log con líneas que no
/// describen lo que pasó deja de servir como evidencia de nada.
pub fn logs_dir(base_dir: &Path) -> PathBuf {
    std::env::var("APPDATA")
        .map(|p| Path::new(&p).join("LocalMind").join("logs"))
        .unwrap_or_else(|_| base_dir.join("logs"))
}

/// Ruta del archivo de log que corresponde a `base_dir`. Solución única de
/// "dónde vive el log": la comparten el hook de pánico (`main.rs`) y el
/// `ProcessManager`, que antes repetían el `.join("localmind.log")`.
pub fn log_file(base_dir: &Path) -> PathBuf {
    logs_dir(base_dir).join(LOG_FILE_NAME)
}

/// Formatear una línea para el archivo: prefijo UTC + texto saneado (P30).
/// Sin inyección de NUL ni saltos: `\0` y `\n`/`\r` se sustituyen por espacio.
pub fn format_line(secs_epoch: u64, text: &str) -> String {
    let clean: String = text
        .chars()
        .map(|c| if c == '\0' || c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    format!("[{}] {}", secs_epoch, clean.trim_end())
}

/// Decidir si hay que rotar antes de anexar `incoming` bytes (P30):
/// `true` cuando el archivo actual supera 1 MiB. Pura y testeable.
pub fn needs_rotation(current_bytes: u64) -> bool {
    current_bytes > MAX_FILE_BYTES
}

/// Rotar: `.1`→`.2` (el `.2` viejo se descarta), base→`.1`. Best-effort.
pub fn rotate(base: &Path) {
    let suffixed = |n: u8| {
        let mut s = base.as_os_str().to_owned();
        s.push(format!(".{}", n));
        PathBuf::from(s)
    };
    // Solo 3 archivos: el más viejo (.2) se pierde.
    let _ = std::fs::remove_file(suffixed(2));
    let _ = std::fs::rename(suffixed(1), suffixed(2));
    let _ = std::fs::rename(base, suffixed(1));
}

/// Anexar una línea ya formateada, rotando si hace falta. Best-effort:
/// nunca falla al llamador (el motor no depende del log de disco).
pub fn append_line(base: &Path, line: &str) {
    if let Some(parent) = base.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    let current = std::fs::metadata(base).map(|m| m.len()).unwrap_or(0);
    if needs_rotation(current) {
        rotate(base);
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(base) {
        let _ = writeln!(f, "{}", line);
    }
}

fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Entrada perezosa del `ProcessManager`: formatea con hora UTC y anexa.
pub fn write_log_line(base: &Path, text: &str) {
    append_line(base, &format_line(now_epoch_secs(), text));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_trips_only_above_1mib() {
        assert!(!needs_rotation(0));
        assert!(!needs_rotation(MAX_FILE_BYTES));
        assert!(needs_rotation(MAX_FILE_BYTES + 1));
    }

    #[test]
    fn formatter_prefixes_utc_and_strips_injection() {
        let line = format_line(1700000000, "hola\nmundo\0fin\r\n");
        assert!(line.starts_with("[1700000000] "));
        assert!(!line.contains('\n'));
        assert!(!line.contains('\0'));
        assert!(!line.contains('\r'));
        assert!(line.contains("hola mundo fin"));
    }

    #[test]
    fn rotation_keeps_3_files_max() {
        let dir = std::env::temp_dir().join(format!("lm-filelog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let base = dir.join("localmind.log");
        std::fs::write(&base, b"base").unwrap();
        std::fs::write(dir.join("localmind.log.1"), b"viej1").unwrap();
        std::fs::write(dir.join("localmind.log.2"), b"viej2").unwrap();
        rotate(&base);
        assert!(!base.exists());
        assert_eq!(std::fs::read(dir.join("localmind.log.1")).unwrap(), b"base");
        assert_eq!(std::fs::read(dir.join("localmind.log.2")).unwrap(), b"viej1");
        // `.2` viejo ("viej2") descartado: solo 3 archivos como máximo.
        let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert_eq!(entries.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
