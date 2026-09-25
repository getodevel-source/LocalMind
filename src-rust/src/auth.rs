//! Clave local del gateway (D-7 / LM-NF-6).
//!
//! - Se genera una clave aleatoria de 32 bytes en hex en el primer arranque y se
//!   persiste en `%APPDATA%\LocalMind\gateway.key`.
//! - Intento de modo 0600: en Windows las ACLs no se tocan (se documenta y se deja
//!   el archivo con los permisos por defecto del perfil del usuario).
//! - Se acepta por `Authorization: Bearer <key>`, `x-api-key: <key>` (clientes
//!   Anthropic) o cookie `lm_key=<key>`.

use std::path::PathBuf;
use tiny_http::Header;

/// Nombre de la cookie que lleva la clave para la WebView (mismo origen).
pub const KEY_COOKIE_NAME: &str = "lm_key";

/// Ruta de la clave: `%APPDATA%\LocalMind\gateway.key`.
pub fn gateway_key_path() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata).join("LocalMind").join("gateway.key")
    } else {
        // Sin APPDATA (entorno mínimo): caer al dir temporal para no abortar.
        std::env::temp_dir().join("LocalMind").join("gateway.key")
    }
}

/// Generar 32 bytes aleatorios en hex (64 chars) sin crates nuevas.
///
/// Usa `RandomState::new()` como fuente de entropía del SO (en Windows se
/// siembra con `BCryptGenRandom`) mezclada con tiempo/PID/contador, y hashea
/// para obtener 4×u64 que se vuelcan en hex.
pub fn generate_key() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};

    static CTR: AtomicU64 = AtomicU64::new(0);

    let mut out = String::with_capacity(64);
    for i in 0..4 {
        let rs = RandomState::new();
        let mut h = rs.build_hasher();
        let now_nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let ctr = CTR.fetch_add(1, Ordering::Relaxed);
        h.write_u64(now_nanos.wrapping_add(ctr.wrapping_mul(0x9E3779B97F4A7C15)));
        h.write_u32(std::process::id());
        h.write_usize(i as usize);
        // Dirección de pila como entropía extra (no secreta, solo mezcla).
        let stack_salt: usize = &i as *const _ as usize;
        h.write_usize(stack_salt);
        let v = h.finish();
        out.push_str(&format!("{:016x}", v));
    }
    out
}
/// Cargar la clave existente o crearla en el primer arranque (best-effort).
pub fn load_or_create_key() -> String {
    let path = gateway_key_path();
    if let Ok(raw) = std::fs::read_to_string(&path) {
        let k = raw.trim().to_string();
        if k.len() >= 32 {
            return k;
        }
    }
    let key = generate_key();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Intento 0600: en Windows no se tocan ACLs; queda con permisos del usuario.
    let _ = std::fs::write(&path, format!("{}\n", key));
    key
}

/// Valor para `Set-Cookie` en `/`, `/index.html`, `/localmind.ico`, `/localmind.png`.
pub fn set_cookie_value(key: &str) -> String {
    format!("{}={}; Path=/; HttpOnly; SameSite=Strict", KEY_COOKIE_NAME, key)
}

fn header_value<'a>(headers: &'a [Header], name: &str) -> Option<&'a str> {
    for h in headers {
        if h.field.as_str().as_str().eq_ignore_ascii_case(name) {
            return Some(h.value.as_str());
        }
    }
    None
}

/// ¿La petición trae credencial válida? Sin `unwrap` en rutas de request.
pub fn is_authorized(headers: &[Header], key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    // 1. Authorization: Bearer <key>
    if let Some(auth) = header_value(headers, "Authorization") {
        let auth = auth.trim();
        if auth.len() > 7 && auth[..7].eq_ignore_ascii_case("bearer ") {
            if auth[7..].trim() == key {
                return true;
            }
        }
    }
    // 2. x-api-key: <key> (clientes Anthropic)
    if let Some(k) = header_value(headers, "x-api-key") {
        if k.trim() == key {
            return true;
        }
    }
    // 3. Cookie: lm_key=<key>
    if let Some(cookie) = header_value(headers, "Cookie").or_else(|| header_value(headers, "cookie")) {
        for part in cookie.split(';') {
            let part = part.trim();
            if let Some(eq) = part.find('=') {
                let (n, v) = part.split_at(eq);
                if n.trim() == KEY_COOKIE_NAME && v[1..].trim() == key {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(field: &str, value: &str) -> Header {
        Header::from_bytes(field.as_bytes(), value.as_bytes()).unwrap()
    }

    #[test]
    fn sin_credenciales_401() {
        assert!(!is_authorized(&[], "abc123"));
        assert!(!is_authorized(&[h("Authorization", "Bearer otra")], "abc123"));
    }

    #[test]
    fn bearer_cookie_y_x_api_key_permitidos() {
        let key = "clave-de-prueba-123";
        assert!(is_authorized(&[h("Authorization", "Bearer clave-de-prueba-123")], key));
        assert!(is_authorized(&[h("authorization", "bearer clave-de-prueba-123")], key));
        assert!(is_authorized(&[h("x-api-key", "clave-de-prueba-123")], key));
        assert!(is_authorized(&[h("Cookie", "otra=1; lm_key=clave-de-prueba-123; x=2")], key));
        assert!(!is_authorized(&[h("Cookie", "lm_key=otra")], key));
    }

    #[test]
    fn generar_clave_64_hex() {
        let k = generate_key();
        assert_eq!(k.len(), 64);
        assert!(k.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
