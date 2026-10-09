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

/// Generar 32 bytes aleatorios en hex (64 chars) con el CSPRNG del SO
/// (`getrandom`, BCryptGenRandom en Windows). Sin mezcla casera: la salida
/// del SO es la clave.
pub fn generate_key() -> String {
    let mut bytes = [0u8; 32];
    if getrandom::getrandom(&mut bytes).is_err() {
        // Sin entropía del SO no hay clave segura: abortar es mejor que una
        // clave predecible (el gateway nace sin secreto y `/api/unlock`
        // quedaría abierto a cualquiera que adivine tiempo/PID).
        panic!("sin entropía del SO para la clave del gateway");
    }
    let mut out = String::with_capacity(64);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
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

/// Comparar clave candidata con la vigente (Fase B4, desbloqueo LAN): igual y
/// no vacía, en tiempo constante (sin cortocircuito por primer byte distinto:
/// evita oráculo de temporización en la LAN). Función aparte para testear.
pub fn key_matches(candidate: &str, key: &str) -> bool {
    let a = candidate.as_bytes();
    let b = key.as_bytes();
    if b.is_empty() || a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Valor para `Set-Cookie` en `/`, `/index.html`, iconos y `/api/unlock`.
pub fn set_cookie_value(key: &str) -> String {
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Strict",
        KEY_COOKIE_NAME, key
    )
}

/// Freno de fuerza bruta en `/api/unlock`: 5 fallos seguidos bloquean 5 min.
/// Contador global del proceso (el gateway es un solo proceso; sin estado
/// por IP: el peer ya está acotado a loopback/red privada por el gate B2).
static UNLOCK_FAILS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static UNLOCK_BLOCKED_UNTIL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn unlock_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// ¿Se admite un intento de desbloqueo ahora? Falso durante el bloqueo.
pub fn unlock_allowed() -> bool {
    unlock_now_secs() >= UNLOCK_BLOCKED_UNTIL.load(std::sync::atomic::Ordering::Relaxed)
}

/// Registrar un fallo: al 5.º se bloquea 5 min y se resetea el contador.
pub fn unlock_failed() {
    let n = UNLOCK_FAILS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if n >= 5 {
        UNLOCK_FAILS.store(0, std::sync::atomic::Ordering::Relaxed);
        UNLOCK_BLOCKED_UNTIL.store(
            unlock_now_secs() + 300,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

/// Registrar un éxito: limpia fallos y bloqueos.
pub fn unlock_ok() {
    UNLOCK_FAILS.store(0, std::sync::atomic::Ordering::Relaxed);
    UNLOCK_BLOCKED_UNTIL.store(0, std::sync::atomic::Ordering::Relaxed);
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
            if key_matches(auth[7..].trim(), key) {
                return true;
            }
        }
    }
    // 2. x-api-key: <key> (clientes Anthropic)
    if let Some(k) = header_value(headers, "x-api-key") {
        if key_matches(k.trim(), key) {
            return true;
        }
    }
    // 3. Cookie: lm_key=<key>
    if let Some(cookie) =
        header_value(headers, "Cookie").or_else(|| header_value(headers, "cookie"))
    {
        for part in cookie.split(';') {
            let part = part.trim();
            if let Some(eq) = part.find('=') {
                let (n, v) = part.split_at(eq);
                if n.trim() == KEY_COOKIE_NAME && key_matches(v[1..].trim(), key) {
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
        assert!(!is_authorized(
            &[h("Authorization", "Bearer otra")],
            "abc123"
        ));
    }

    #[test]
    fn bearer_cookie_y_x_api_key_permitidos() {
        let key = "clave-de-prueba-123";
        assert!(is_authorized(
            &[h("Authorization", "Bearer clave-de-prueba-123")],
            key
        ));
        assert!(is_authorized(
            &[h("authorization", "bearer clave-de-prueba-123")],
            key
        ));
        assert!(is_authorized(&[h("x-api-key", "clave-de-prueba-123")], key));
        assert!(is_authorized(
            &[h("Cookie", "otra=1; lm_key=clave-de-prueba-123; x=2")],
            key
        ));
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

    #[test]
    fn desbloqueo_compara_clave_y_rechaza_vacia() {
        assert!(key_matches("abc123", "abc123"));
        assert!(!key_matches("otra", "abc123"));
        assert!(!key_matches("", "abc123"));
        assert!(!key_matches("abc123", ""));
        assert!(!key_matches("", ""));
        // Longitud distinta también es falso (sin pánico por slicing).
        assert!(!key_matches("abc1234", "abc123"));
    }

    #[test]
    fn freno_cinco_fallos_bloquea_y_exito_limpia() {
        unlock_ok();
        assert!(unlock_allowed());
        for _ in 0..4 {
            unlock_failed();
            assert!(unlock_allowed());
        }
        unlock_failed();
        assert!(!unlock_allowed(), "5 fallos deben bloquear");
        unlock_ok();
        assert!(unlock_allowed());
    }
}
