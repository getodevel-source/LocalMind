//! Metadatos del gateway: versión del motor, sellos UTC y política CORS.
//!
//! - `parse_engine_version`: parser puro de la salida de
//!   `bin/llama-server.exe --version` (forma real:
//!   `version: 0.2.0-dev (build 10683, commit d8f26eec7)`).
//! - `engine_info`: ejecuta el binario una vez (timeout 5 s) y cachea el
//!   resultado en memoria para toda la vida del proceso; ante cualquier
//!   problema devuelve campos `None` (la ruta nunca falla por esto).
//! - `is_loopback_origin`: predicado puro para CORS (SRS P29): solo refleja
//!   orígenes `http://127.0.0.1|localhost|[::1][:puerto]`.
//! - `utc_stamp_now`: `YYYYMMDD-HHMMSS` UTC sin crates de fecha (para el
//!   nombre del export de logs).

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use tiny_http::Header;

// ---------------------------------------------------------------------------
// Versión del motor (parser puro + spawn con timeout y caché)
// ---------------------------------------------------------------------------

/// `(build, commit)` de una salida como
/// `version: 0.2.0-dev (build 10683, commit d8f26eec7)`.
/// `None` si no aparecen ambos campos (salida malformada).
pub fn parse_engine_version(output: &str) -> Option<(String, String)> {
    let mut build: Option<String> = None;
    let mut commit: Option<String> = None;
    for line in output.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        let mut i = 0;
        while i < toks.len() {
            // El token puede traer el paréntesis pegado ("(build").
            let key = toks[i].trim_matches(['(', ',']);
            if key == "build" && i + 1 < toks.len() {
                let v = toks[i + 1].trim_matches([',', ')']).to_string();
                if !v.is_empty() {
                    build = Some(v);
                }
            } else if key == "commit" && i + 1 < toks.len() {
                let v = toks[i + 1].trim_matches([',', ')']).to_string();
                if !v.is_empty() {
                    commit = Some(v);
                }
            }
            i += 1;
        }
    }
    match (build, commit) {
        (Some(b), Some(c)) => Some((b, c)),
        _ => None,
    }
}

/// Campos del motor para `GET /api/version` (`None` = nulo en el JSON).
#[derive(Debug, Clone)]
pub struct EngineInfo {
    pub build: Option<String>,
    pub commit: Option<String>,
    pub path: Option<String>,
}

static ENGINE_CACHE: LazyLock<Mutex<Option<EngineInfo>>> = LazyLock::new(|| Mutex::new(None));

fn cached_info() -> Option<EngineInfo> {
    match ENGINE_CACHE.lock() {
        Ok(g) => g.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn store_info(info: &EngineInfo) {
    match ENGINE_CACHE.lock() {
        Ok(mut g) => *g = Some(info.clone()),
        Err(poisoned) => *poisoned.into_inner() = Some(info.clone()),
    }
}

/// Ruta del binario del motor para un `base_dir` dado.
pub fn engine_bin_path(base_dir: &Path) -> PathBuf {
    base_dir.join("bin").join("llama-server.exe")
}

/// Consulta el motor una vez y cachea el resultado en memoria.
/// Nunca falla: ante cualquier problema devuelve todo `None`.
pub fn engine_info(base_dir: &Path) -> EngineInfo {
    if let Some(hit) = cached_info() {
        return hit;
    }
    let info = query_engine(base_dir);
    store_info(&info);
    info
}

fn query_engine(base_dir: &Path) -> EngineInfo {
    let failed = EngineInfo {
        build: None,
        commit: None,
        path: None,
    };
    let bin = engine_bin_path(base_dir);
    let path_str = bin.to_string_lossy().to_string();
    let mut cmd = std::process::Command::new(&bin);
    cmd.arg("--version");
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    // llama-server escribe `version: …` en STDERR, no en stdout.
    cmd.stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return failed,
    };
    // Timeout de 5 s sin bloquear: sondeo con `try_wait`.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut exited = false;
    while std::time::Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => {
                exited = true;
                break;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(_) => break,
        }
    }
    if !exited {
        let _ = child.kill();
        let _ = child.wait();
        return failed;
    }
    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return failed,
    };
    if !output.status.success() {
        return failed;
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    match parse_engine_version(&text) {
        Some((build, commit)) => EngineInfo {
            build: Some(build),
            commit: Some(commit),
            path: Some(path_str),
        },
        None => failed,
    }
}

// ---------------------------------------------------------------------------
// Origen de la petición (para CORS estricto, SRS P29)
// ---------------------------------------------------------------------------

/// Valor del header `Origin` de la petición, si trae uno no vacío.
pub fn request_origin(headers: &[Header]) -> Option<String> {
    for h in headers {
        if h.field.as_str().as_str().eq_ignore_ascii_case("Origin") {
            let v = h.value.as_str().trim();
            if v.is_empty() {
                return None;
            }
            return Some(v.to_string());
        }
    }
    None
}

/// ¿Es un origen loopback (`http://127.0.0.1|localhost|[::1][:puerto]`)?
/// Todo lo demás (`https`, dominios, `null`, vacío, con path/userinfo) se rechaza.
pub fn is_loopback_origin(origin: &str) -> bool {
    let rest = match origin.trim().strip_prefix("http://") {
        Some(r) => r,
        None => return false,
    };
    if rest.is_empty() {
        return false;
    }
    // Sin userinfo, path, query ni fragmento.
    if rest.contains(['@', '/', '?', '#']) {
        return false;
    }
    let (host, port) = if let Some(stripped) = rest.strip_prefix('[') {
        // Literal IPv6 `[::1]` con puerto opcional.
        let end = match stripped.find(']') {
            Some(i) => i,
            None => return false,
        };
        let host = &stripped[..end];
        let after = &stripped[end + 1..];
        let port = if after.is_empty() {
            None
        } else if let Some(p) = after.strip_prefix(':') {
            Some(p)
        } else {
            return false;
        };
        (host, port)
    } else {
        // IPv6 sin corchetes trae varios `:` → se rechaza.
        let mut parts = rest.split(':');
        let host = parts.next().unwrap_or("");
        match parts.next() {
            None => (host, None),
            Some(p) => {
                if parts.next().is_some() {
                    return false;
                }
                (host, Some(p))
            }
        }
    };
    let host_ok = host.eq_ignore_ascii_case("127.0.0.1")
        || host.eq_ignore_ascii_case("localhost")
        || host == "::1";
    if !host_ok {
        return false;
    }
    match port {
        None => true,
        Some(p) => !p.is_empty() && p.len() <= 5 && p.chars().all(|c| c.is_ascii_digit()),
    }
}

// ---------------------------------------------------------------------------
// Red privada del dueño (Fase B1, diseño B2/B3/B7): puras y testeables.
// Sin cablear todavía: ningún handler las llama en esta ronda.
// ---------------------------------------------------------------------------

/// ¿IP de la red privada del dueño? RFC1918 (`10/8`, `172.16/12`, `192.168/16`
/// vía `is_private`) + CGNAT de Tailscale (`100.64/10`, red privada del dueño
/// según diseño B7). Loopback NO entra acá: tiene su propia regla que siempre
/// pasa. IPv6 ULA queda fuera de v1 a propósito.
pub fn ip_red_privada(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_private() {
                return true;
            }
            // `100.64.0.0/10` = 100.64.0.0–100.127.255.255.
            let o = v4.octets();
            o[0] == 100 && (o[1] & 0xC0) == 0x40
        }
        IpAddr::V6(_) => false,
    }
}

/// Regla del peer (diseño B2): loopback siempre permitido (WebView, CLIs,
/// túnel local que entra por localhost); red privada solo con `lan.enabled`;
/// internet directa jamás, incluso con el socket en `0.0.0.0`.
/// El llamador la aplica ANTES de auth, con `remote_addr()` del socket.
pub fn peer_permitido(ip: IpAddr, lan_enabled: bool) -> bool {
    ip.is_loopback() || (lan_enabled && ip_red_privada(ip))
}

/// Host de un `Origin` con la misma gramática estricta que `is_loopback_origin`
/// (solo `http://`, sin userinfo/path/query/fragmento, puerto numérico).
/// `None` = malformado o no-http (p. ej. `https:`, `null`, vacío).
pub fn origin_host(origin: &str) -> Option<String> {
    let rest = origin.trim().strip_prefix("http://")?;
    if rest.is_empty() || rest.contains(['@', '/', '?', '#']) {
        return None;
    }
    if let Some(stripped) = rest.strip_prefix('[') {
        let end = stripped.find(']')?;
        let host = &stripped[..end];
        if host.is_empty() {
            return None;
        }
        let after = &stripped[end + 1..];
        if after.is_empty() {
            return Some(host.to_string());
        }
        let p = after.strip_prefix(':')?;
        if !p.is_empty() && p.len() <= 5 && p.chars().all(|c| c.is_ascii_digit()) {
            return Some(host.to_string());
        }
        return None;
    }
    let mut parts = rest.split(':');
    let host = parts.next().unwrap_or("");
    if host.is_empty() {
        return None;
    }
    match parts.next() {
        None => Some(host.to_string()),
        Some(p) => {
            if parts.next().is_some() {
                return None; // IPv6 sin corchetes.
            }
            if !p.is_empty() && p.len() <= 5 && p.chars().all(|c| c.is_ascii_digit()) {
                Some(host.to_string())
            } else {
                None
            }
        }
    }
}

/// CORS por allowlist (diseño B3, evolución de P29): loopback como hoy, más
/// red privada con `lan.enabled`. Dominios no-IP jamás (el túnel se consume
/// same-origin o vía app/CLI, que no necesitan ACAO). `https` en LAN tampoco:
/// el gateway no termina TLS en v1.
pub fn origen_permitido(origin: &str, lan_enabled: bool) -> bool {
    if is_loopback_origin(origin) {
        return true;
    }
    if !lan_enabled {
        return false;
    }
    let host = match origin_host(origin) {
        Some(h) => h,
        None => return false,
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(ip) => ip_red_privada(ip),
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Interruptor LAN del proceso (Fase B2): se fija UNA vez al arrancar el gateway
// desde `[lan].enabled` y no cambia en toda la vida del proceso (cambiarlo
// exige reiniciar: el socket se liga una vez, diseño B11). Existe como estado
// global por la misma razón que `auth::CACHED_KEY`: los helpers CORS puros de
// `server.rs` (`cors_origin_header`, `json_response_for_origin`, ...) se llaman
// en ~120 sitios y no arrastran la config; la alternativa era cambiar todas
// esas firmas. Las funciones puras con parámetro explícito (`peer_permitido`,
// `origen_permitido`) siguen siendo la referencia testeable.
// ---------------------------------------------------------------------------

static LAN_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Fijar el modo LAN al arrancar. Llamar una sola vez (`HttpServer::start`).
pub fn set_lan_mode(enabled: bool) {
    LAN_MODE.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// ¿El gateway escucha fuera de loopback? Lo que se fijó al arrancar.
pub fn lan_mode() -> bool {
    LAN_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Descubrimiento LAN + pairing (Fase B3, diseño B5): la red que el dueño
// muestra en QR para emparejar el Cliente.
// ---------------------------------------------------------------------------

/// Payload de pairing para el QR (diseño B5): `omni://<ip>:<puerto>#k=<clave>`.
/// Puro: el llamador (autenticado) pone la clave vigente; la UI del Cliente
/// lo escanea o pega como URL + clave.
pub fn pairing_url(ip: &str, port: u16, key: &str) -> String {
    format!("omni://{}:{}#k={}", ip, port, key)
}

/// IPs locales aptas para pairing (diseño B5): la IP de salida hacia internet
/// (truco UDP sin tráfico: `connect` no envía nada, solo consulta la tabla de
/// rutas) filtrada a red privada + loopback excluido. Vacío = sin red o solo
/// loopback: la UI lo dice en vez de inventar. Ordenada y sin duplicados.
pub fn descubrir_ips_locales() -> Vec<String> {
    use std::net::UdpSocket;
    let mut ips = Vec::new();
    // Varios destinos por si la ruta por defecto apunta afuera de uno solo.
    for destino in ["8.8.8.8:80", "1.1.1.1:80"] {
        let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else {
            continue;
        };
        if sock.connect(destino).is_err() {
            continue;
        }
        let Ok(local) = sock.local_addr() else {
            continue;
        };
        let ip = local.ip();
        if ip.is_loopback() || !ip_red_privada(ip) {
            continue;
        }
        let s = ip.to_string();
        if !ips.contains(&s) {
            ips.push(s);
        }
    }
    ips.sort();
    ips
}

// ---------------------------------------------------------------------------
// Sello UTC `YYYYMMDD-HHMMSS` sin crates de fecha
// ---------------------------------------------------------------------------

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    // Howard Hinnant, inverso de days_from_civil (1970-01-01 = día 719468).
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `YYYYMMDD-HHMMSS` UTC para unos segundos Unix.
pub fn utc_stamp(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Sello del instante actual (para el nombre del export de logs).
pub fn utc_stamp_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    utc_stamp(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_real_de_llama_server() {
        let out = "version: 0.2.0-dev (build 10683, commit d8f26eec7)\nbuilt with MSVC 19.51.36256.0 for x64\n";
        assert_eq!(
            parse_engine_version(out),
            Some(("10683".to_string(), "d8f26eec7".to_string()))
        );
        // En producción la línea llega por STDERR (el formato une ambos).
        let joined = format!("{}\n{}", "", out);
        assert_eq!(
            parse_engine_version(&joined),
            Some(("10683".to_string(), "d8f26eec7".to_string()))
        );
        assert_eq!(parse_engine_version(""), None);
        assert_eq!(parse_engine_version("basura sin campos\n"), None);
        assert_eq!(parse_engine_version("version: 1.0\n"), None);
    }

    #[test]
    fn version_malformada_da_none() {
        assert_eq!(parse_engine_version("version: x (build 12)\n"), None);
        assert_eq!(parse_engine_version("version: x (commit abc)\n"), None);
    }

    #[test]
    fn sello_utc_conocido() {
        assert_eq!(utc_stamp(0), "19700101-000000");
        // 2026-09-25 00:00:00 UTC.
        assert_eq!(utc_stamp(1790294400), "20260925-000000");
        assert_eq!(utc_stamp(1790294400 + 3661), "20260925-010101");
    }

    #[test]
    fn origenes_loopback_aceptados() {
        assert!(is_loopback_origin("http://127.0.0.1"));
        assert!(is_loopback_origin("http://127.0.0.1:17860"));
        assert!(is_loopback_origin("http://localhost"));
        assert!(is_loopback_origin("http://localhost:3000"));
        assert!(is_loopback_origin("http://[::1]"));
        assert!(is_loopback_origin("http://[::1]:8080"));
    }

    #[test]
    fn origenes_ajenos_rechazados() {
        assert!(!is_loopback_origin("https://evil.example"));
        assert!(!is_loopback_origin("http://localhost.evil.com"));
        assert!(!is_loopback_origin("http://127.0.0.1.evil.com"));
        assert!(!is_loopback_origin("null"));
        assert!(!is_loopback_origin(""));
        assert!(!is_loopback_origin("http://evil.com"));
        assert!(!is_loopback_origin("https://localhost"));
        assert!(!is_loopback_origin("https://127.0.0.1:17860"));
        assert!(!is_loopback_origin("http://127.0.0.1/"));
        assert!(!is_loopback_origin("http://127.0.0.1:abc"));
        assert!(!is_loopback_origin("http://[::2]"));
        assert!(!is_loopback_origin("http://user@localhost"));
        assert!(!is_loopback_origin("http://::1"));
    }

    // ---- Fase B1: regla del peer y allowlist de orígenes ----

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn peer_loopback_siempre_pasa() {
        assert!(peer_permitido(ip("127.0.0.1"), false));
        assert!(peer_permitido(ip("127.0.0.1"), true));
        assert!(peer_permitido(ip("::1"), false));
        assert!(peer_permitido(ip("::1"), true));
    }

    #[test]
    fn peer_privado_solo_con_lan() {
        for h in ["192.168.1.10", "10.0.0.2", "172.16.0.1", "172.31.255.255"] {
            assert!(!peer_permitido(ip(h), false), "{}", h);
            assert!(peer_permitido(ip(h), true), "{}", h);
        }
        // Fronteras del /12 y del CGNAT: un bit afuera ya es internet.
        assert!(!peer_permitido(ip("172.15.255.255"), true));
        assert!(!peer_permitido(ip("172.32.0.1"), true));
        assert!(peer_permitido(ip("100.64.0.1"), true));
        assert!(peer_permitido(ip("100.127.255.255"), true));
        assert!(!peer_permitido(ip("100.64.0.1"), false));
        assert!(!peer_permitido(ip("100.63.255.255"), true));
        assert!(!peer_permitido(ip("100.128.0.0"), true));
        // Internet directa: jamás, en ningún modo.
        for h in ["8.8.8.8", "1.1.1.1", "203.0.113.7"] {
            assert!(!peer_permitido(ip(h), false), "{}", h);
            assert!(!peer_permitido(ip(h), true), "{}", h);
        }
    }

    #[test]
    fn allowlist_origenes_respeta_modo() {
        // Loopback: igual que hoy, en ambos modos.
        assert!(origen_permitido("http://127.0.0.1:17860", false));
        assert!(origen_permitido("http://localhost:3000", true));
        // LAN: solo con lan.enabled, solo http, solo IP privada.
        assert!(!origen_permitido("http://192.168.1.10:17860", false));
        assert!(origen_permitido("http://192.168.1.10:17860", true));
        assert!(origen_permitido("http://10.0.0.2:17860", true));
        assert!(!origen_permitido("https://192.168.1.10:17860", true));
        assert!(!origen_permitido("http://192.168.1.10:17860", false));
        assert!(!origen_permitido("http://8.8.8.8:17860", true));
        assert!(!origen_permitido("http://evil.com", true));
        assert!(!origen_permitido("null", true));
        assert!(!origen_permitido("", true));
    }

    #[test]
    fn pairing_url_con_forma_qr() {
        assert_eq!(
            pairing_url("192.168.1.10", 17860, "abc"),
            "omni://192.168.1.10:17860#k=abc"
        );
    }

    #[test]
    fn descubrir_ips_solo_privadas_y_sin_duplicar() {
        // Sin red el resultado es vacío (no inventa); con red, solo privadas.
        for ip in descubrir_ips_locales() {
            let parsed: IpAddr = ip.parse().expect("IP válida");
            assert!(!parsed.is_loopback());
            assert!(ip_red_privada(parsed), "{}", ip);
        }
        let mut v = descubrir_ips_locales();
        v.sort();
        v.dedup();
        assert_eq!(v, descubrir_ips_locales());
    }
}
