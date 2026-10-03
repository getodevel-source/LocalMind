//! Clave local del gateway (D-7 / LM-NF-6).
//!
//! - Se genera una clave aleatoria de 32 bytes en hex en el primer arranque y se
//!   persiste en `%APPDATA%\LocalMind\gateway.key`.
//! - Intento de modo 0600: en Windows las ACLs no se tocan (se documenta y se deja
//!   el archivo con los permisos por defecto del perfil del usuario).
//! - Se acepta por `Authorization: Bearer <key>`, `x-api-key: <key>` (clientes
//!   Anthropic) o cookie `lm_key=<key>`.

use std::path::PathBuf;
use std::sync::OnceLock;
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

/// Rotar la clave del gateway (Fase B3, pairing): genera una nueva, la persiste
/// y la devuelve. Toma efecto al REINICIAR: el gateway usa la instantánea del
/// arranque y el motor su `--api-key` del spawn (más el caché `gateway_key()`).
/// El llamador responde `restart_required: true` y lo loguea.
pub fn rotate_key() -> Result<String, String> {
    rotate_key_at(&gateway_key_path())
}

/// Núcleo testeable de `rotate_key` (no toca la ruta real en tests).
pub fn rotate_key_at(path: &std::path::Path) -> Result<String, String> {
    let key = generate_key();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, format!("{}\n", key)).map_err(|e| e.to_string())?;
    Ok(key)
}

/// Valor para `Set-Cookie` en `/`, `/index.html`, `/localmind.ico`, `/localmind.png`.
pub fn set_cookie_value(key: &str) -> String {
    format!("{}={}; Path=/; HttpOnly; SameSite=Strict", KEY_COOKIE_NAME, key)
}

/// La MISMA clave del gateway, cacheada por proceso.
///
/// `load_or_create_key()` va al disco en cada llamada. El motor la necesita en
/// dos sitios que no pueden arrastrarla por firma: `build_engine_cmd` es una
/// `fn` asociada (no método) y la alimentan el reintento MTP y el poller; los
/// handlers del proxy tampoco la reciben por parámetro. En vez de cambiar esas
/// firmas se cachea una vez y se reparte desde aquí.
///
/// El valor no puede quedar obsoleto en la práctica: el servidor ya toma una
/// instantánea al arrancar (`server.rs`) y la usa durante toda la vida del
/// proceso, así que cachear aquí solo replica ese comportamiento.
static CACHED_KEY: OnceLock<String> = OnceLock::new();

pub fn gateway_key() -> &'static str {
    CACHED_KEY.get_or_init(load_or_create_key)
}

/// Cabecera `Authorization` que el MOTOR espera cuando se le pasa
/// `--api-key` (D-45). Mismo esquema `Bearer` que ya valida el gateway: una
/// sola forma de hablar con la clave, no dos literales repartidos.
pub fn bearer(key: &str) -> String {
    format!("Bearer {}", key)
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

    #[test]
    fn rotar_clave_genera_distinta_y_persiste() {
        // Nunca contra la ruta real: solo el temporal del test.
        let dir = std::env::temp_dir().join(format!("lm-rotate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("gateway.key");
        let k1 = rotate_key_at(&path).expect("rota");
        assert_eq!(k1.len(), 64);
        let k2 = rotate_key_at(&path).expect("re-rota");
        assert_eq!(k2.len(), 64);
        assert_ne!(k1, k2);
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), k2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
