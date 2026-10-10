//! Registro de uso JSONL (base de telemetría; espeja el gateway de 0xSero).
//!
//! - Una línea JSON por request proxyeado completado en
//!   `%APPDATA%\LocalMind\usage.jsonl`:
//!   `{ts, endpoint, model, prompt_tokens, completion_tokens, ms, stream}`.
//! - Usa el `usage` del motor cuando viene (se pide `stream_options:
//!   {"include_usage": true}` si el cliente no lo fijó); si solo hay datos
//!   parciales se registra lo disponible, jamás se inventa.
//! - Rotación: si el archivo supera 1 MiB se conservan sus últimas 5000 líneas.

use std::path::PathBuf;

/// Ruta del registro: `%APPDATA%\LocalMind\usage.jsonl`.
/// `LOCALMIND_USAGE_PATH` la redirige (los tests la fijan a un scratch para
/// no contaminar el registro real del dueño con líneas `test.tee`).
pub fn usage_path() -> PathBuf {
    if let Ok(p) = std::env::var("LOCALMIND_USAGE_PATH") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata).join("LocalMind").join("usage.jsonl")
    } else {
        std::env::temp_dir().join("LocalMind").join("usage.jsonl")
    }
}

/// Segundos Unix actuales.
pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

const ROTATE_BYTES: u64 = 1024 * 1024;
const KEEP_LINES: usize = 5000;

/// Cola acotada (máx 64 KiB) para extraer usage al final sin guardar el body entero.
pub const TAIL_MAX_BYTES: usize = 64 * 1024;

/// Guardar `chunk` en la cola acotada (conserva solo los últimos 64 KiB).
pub fn push_tail(tail: &mut Vec<u8>, chunk: &[u8]) {
    tail.extend_from_slice(chunk);
    if tail.len() > TAIL_MAX_BYTES {
        let drop_n = tail.len() - TAIL_MAX_BYTES;
        tail.drain(..drop_n);
    }
}

/// Extraer usage del último chunk SSE (o de un JSON no-streaming).
pub fn extract_usage_from_sse(raw: &str) -> (Option<u64>, Option<u64>) {
    let mut p: Option<u64> = None;
    let mut c: Option<u64> = None;
    for line in raw.lines() {
        let line = line.trim();
        let payload = if let Some(rest) = line.strip_prefix("data:") {
            rest.trim()
        } else if line.starts_with('{') {
            line
        } else {
            continue;
        };
        if payload == "[DONE]" || payload.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) {
            if let Some(u) = v.get("usage") {
                if let Some(x) = u.get("prompt_tokens").and_then(|x| x.as_u64()) {
                    p = Some(x);
                }
                if let Some(x) = u.get("completion_tokens").and_then(|x| x.as_u64()) {
                    c = Some(x);
                }
            }
        }
    }
    (p, c)
}

/// Añadir una línea JSONL + rotar si supera 1 MiB (últimas 5000 líneas).
/// Best-effort: nunca falla el request por el registro.
pub fn log_usage(
    endpoint: &str,
    model: &str,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    ms: u64,
    stream: bool,
) {
    let mut obj = serde_json::Map::new();
    obj.insert("ts".to_string(), serde_json::json!(now_ts()));
    obj.insert("endpoint".to_string(), serde_json::json!(endpoint));
    obj.insert("model".to_string(), serde_json::json!(model));
    obj.insert(
        "prompt_tokens".to_string(),
        prompt_tokens
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null),
    );
    obj.insert(
        "completion_tokens".to_string(),
        completion_tokens
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null),
    );
    obj.insert("ms".to_string(), serde_json::json!(ms));
    obj.insert("stream".to_string(), serde_json::json!(stream));
    let line_str = serde_json::Value::Object(obj).to_string();

    let path = usage_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Rotación antes de añadir: si supera 1 MiB, quedarnos con 5000 líneas.
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > ROTATE_BYTES {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let lines: Vec<&str> = content.lines().collect();
                if lines.len() > KEEP_LINES {
                    let tail = lines[lines.len() - KEEP_LINES..].join("\n");
                    let _ = std::fs::write(&path, tail + "\n");
                }
            }
        }
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{}", line_str);
    }
}

/// Recortar contenido a las últimas N líneas (para tests de rotación).
#[allow(dead_code)]
pub fn tail_lines(content: &str, n: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= n {
        return content.to_string();
    }
    lines[lines.len() - n..].join("\n") + "\n"
}

/// Ruta del resumen cacheado: `%APPDATA%\LocalMind\usage-summary.json`.
pub fn summary_path() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata)
            .join("LocalMind")
            .join("usage-summary.json")
    } else {
        std::env::temp_dir()
            .join("LocalMind")
            .join("usage-summary.json")
    }
}

/// Resumen incremental del log: `{totals, by_model, by_day, first_ts, last_ts}`.
/// Solo parsea las líneas nuevas desde el offset guardado; si el log se
/// encogió/rotó se reconstruye desde cero. Jamás inventa: lo desconocido es 0.
pub fn usage_summary() -> String {
    usage_summary_at(&usage_path(), &summary_path())
}

/// Día `YYYY-MM-DD` (UTC) desde segundos Unix, sin crates de fecha.
pub fn day_of(ts: i64) -> String {
    if ts <= 0 {
        return "1970-01-01".to_string();
    }
    let days = ts.div_euclid(86400);
    // Howard Hinnant, days_from_civil inverso (1970-01-01 = día 719468).
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 }.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", y + if m <= 2 { 1 } else { 0 }, m, d)
}

fn summary_from_scratch() -> serde_json::Value {
    serde_json::json!({
        "offset": 0,
        "totals": {"requests": 0, "prompt_tokens": 0, "completion_tokens": 0},
        "by_model": {},
        "by_day": {},
        "first_ts": 0,
        "last_ts": 0,
    })
}

fn add_line(sum: &mut serde_json::Value, v: &serde_json::Value) {
    let req_model = v
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let pt = v.get("prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
    let ct = v
        .get("completion_tokens")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let ts = v.get("ts").and_then(|x| x.as_i64()).unwrap_or(0);
    let model_id = if req_model.is_empty() {
        "(sin modelo)".to_string()
    } else {
        req_model
    };
    if let Some(t) = sum.get_mut("totals") {
        t["requests"] =
            serde_json::Value::from(t.get("requests").and_then(|x| x.as_u64()).unwrap_or(0) + 1);
        t["prompt_tokens"] = serde_json::Value::from(
            t.get("prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(0) + pt,
        );
        t["completion_tokens"] = serde_json::Value::from(
            t.get("completion_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0)
                + ct,
        );
    }
    if sum.get("by_model").and_then(|m| m.get(&model_id)).is_none() {
        if let Some(m) = sum.get_mut("by_model") {
            if let Some(o) = m.as_object_mut() {
                o.insert(
                    model_id.clone(),
                    serde_json::json!({"requests": 0, "prompt_tokens": 0, "completion_tokens": 0}),
                );
            }
        }
    }
    if let Some(e) = sum.get_mut("by_model").and_then(|m| m.get_mut(&model_id)) {
        e["requests"] =
            serde_json::Value::from(e.get("requests").and_then(|x| x.as_u64()).unwrap_or(0) + 1);
        e["prompt_tokens"] = serde_json::Value::from(
            e.get("prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(0) + pt,
        );
        e["completion_tokens"] = serde_json::Value::from(
            e.get("completion_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0)
                + ct,
        );
    }
    let day = day_of(ts);
    if sum.get("by_day").and_then(|m| m.get(&day)).is_none() {
        if let Some(m) = sum.get_mut("by_day") {
            if let Some(o) = m.as_object_mut() {
                o.insert(
                    day.clone(),
                    serde_json::json!({"requests": 0, "prompt_tokens": 0, "completion_tokens": 0}),
                );
            }
        }
    }
    if let Some(e) = sum.get_mut("by_day").and_then(|m| m.get_mut(&day)) {
        e["requests"] =
            serde_json::Value::from(e.get("requests").and_then(|x| x.as_u64()).unwrap_or(0) + 1);
        e["prompt_tokens"] = serde_json::Value::from(
            e.get("prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(0) + pt,
        );
        e["completion_tokens"] = serde_json::Value::from(
            e.get("completion_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0)
                + ct,
        );
    }
    if ts > 0 {
        let first = sum.get("first_ts").and_then(|x| x.as_i64()).unwrap_or(0);
        if first == 0 || ts < first {
            sum["first_ts"] = serde_json::Value::from(ts);
        }
        let last = sum.get("last_ts").and_then(|x| x.as_i64()).unwrap_or(0);
        if ts > last {
            sum["last_ts"] = serde_json::Value::from(ts);
        }
    }
}

/// Núcleo testeable: resumen incremental sobre rutas explícitas.
pub fn usage_summary_at(log: &std::path::Path, cache: &std::path::Path) -> String {
    let mut sum = std::fs::read_to_string(cache)
        .ok()
        .and_then(|r| serde_json::from_str::<serde_json::Value>(&r).ok())
        .filter(|v| v.get("totals").is_some() && v.get("offset").and_then(|o| o.as_u64()).is_some())
        .unwrap_or_else(summary_from_scratch);
    let meta_len = std::fs::metadata(log).map(|m| m.len()).unwrap_or(0);
    let mut offset = sum.get("offset").and_then(|o| o.as_u64()).unwrap_or(0);
    if offset > meta_len {
        // El log se encogió/rotó: reconstruir desde cero.
        sum = summary_from_scratch();
        offset = 0;
    }
    if let Ok(content) = std::fs::read(log) {
        let start = (offset as usize).min(content.len());
        let tail = &content[start..];
        // Solo líneas completas (cortar en el último `\n` si el writer va a medias).
        let end = tail
            .iter()
            .rposition(|&b| b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let consumed = start + end;
        for line in tail[..end].split(|&b| b == b'\n') {
            if line.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(line) {
                add_line(&mut sum, &v);
            }
        }
        sum["offset"] = serde_json::Value::from(consumed as u64);
    }
    if let Some(parent) = cache.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(cache, sum.to_string());
    // Respuesta pública: sin el offset interno.
    let mut out = sum.clone();
    if let Some(o) = out.as_object_mut() {
        o.remove("offset");
    }
    out.to_string()
}

/// Últimas `limit` líneas del log como `{lines:[...], skipped:n}`.
/// Las líneas que no parsean se cuentan en `skipped`, no se inventan.
pub fn usage_raw(limit: usize) -> String {
    usage_raw_at(&usage_path(), limit.min(500))
}

/// Núcleo testeable del raw sobre una ruta explícita.
pub fn usage_raw_at(log: &std::path::Path, limit: usize) -> String {
    let limit = limit.min(500);
    let content = std::fs::read_to_string(log).unwrap_or_default();
    let mut skipped = 0u64;
    // Recorrer desde el final hasta juntar `limit` válidas (acotado en memoria).
    let mut valid: Vec<serde_json::Value> = Vec::new();
    for line in content.lines().rev() {
        if valid.len() >= limit {
            break;
        }
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(t) {
            Ok(v) => valid.push(v),
            Err(_) => skipped += 1,
        }
    }
    valid.reverse();
    serde_json::json!({"lines": valid, "skipped": skipped}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_del_chunk_final_y_nada_si_no_hay() {
        let raw = "data: {\"choices\":[{\"delta\":{\"content\":\"h\"}}]}\n\ndata: {\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":9}}\n\ndata: [DONE]\n\n";
        assert_eq!(extract_usage_from_sse(raw), (Some(3), Some(9)));
        assert_eq!(
            extract_usage_from_sse("data: {\"choices\":[]}\n\ndata: [DONE]\n"),
            (None, None)
        );
    }

    #[test]
    fn usage_path_respeta_override_para_tests() {
        // Higiene (auditoría): `LOCALMIND_USAGE_PATH` redirige el registro para
        // que `cargo test` no contamine el `usage.jsonl` real del dueño.
        let scratch = std::env::temp_dir().join(format!("lm-usage-path-{}", std::process::id()));
        let custom = scratch.join("custom.jsonl");
        unsafe { std::env::set_var("LOCALMIND_USAGE_PATH", &custom) };
        assert_eq!(usage_path(), custom);
        unsafe { std::env::remove_var("LOCALMIND_USAGE_PATH") };
        // Sin override vuelve a la ruta real (no se afirma su valor, solo que
        // no es el scratch).
        assert_ne!(usage_path(), custom);
    }

    #[test]
    fn recorte_de_cola() {
        let c = (0..10)
            .map(|i| format!("l{}", i))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let t = tail_lines(&c, 3);
        assert_eq!(t, "l7\nl8\nl9\n");
    }

    #[test]
    fn cola_acotada_a_64kib() {
        let mut tail = Vec::new();
        push_tail(&mut tail, &vec![b'a'; TAIL_MAX_BYTES + 100]);
        assert_eq!(tail.len(), TAIL_MAX_BYTES);
        push_tail(&mut tail, b"BC");
        assert_eq!(tail.len(), TAIL_MAX_BYTES);
        assert!(tail.ends_with(b"BC"));
        tail.clear();
        push_tail(&mut tail, b"data: {\"choices\":[{\"delta\":{}}]}\n\n");
        push_tail(
            &mut tail,
            b"data: {\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":2}}\n\ndata: [DONE]\n\n",
        );
        let s = String::from_utf8_lossy(&tail).to_string();
        assert_eq!(extract_usage_from_sse(&s), (Some(1), Some(2)));
        assert!(tail.len() <= TAIL_MAX_BYTES);
    }

    #[test]
    fn resumen_incremental_y_rebuild_tras_rotacion() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("lm-usage-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let log = dir.join("usage.jsonl");
        let cache = dir.join("usage-summary.json");
        let line = |ts: i64, m: &str, p: u64, c: u64| {
            format!("{{\"ts\":{},\"endpoint\":\"chat.completions\",\"model\":\"{}\",\"prompt_tokens\":{},\"completion_tokens\":{},\"ms\":10,\"stream\":false}}\n", ts, m, p, c)
        };
        // Día fijo: 2026-09-25 00:00 UTC = 1790294400.
        let base = 1790294400i64;
        {
            let mut f = std::fs::File::create(&log).unwrap();
            let _ = f.write_all(line(base, "m1", 10, 20).as_bytes());
            let _ = f.write_all(line(base + 60, "m1", 5, 5).as_bytes());
        }
        let s1: serde_json::Value = serde_json::from_str(&usage_summary_at(&log, &cache)).unwrap();
        assert_eq!(s1["totals"]["requests"], serde_json::json!(2));
        assert_eq!(s1["totals"]["prompt_tokens"], serde_json::json!(15));
        assert_eq!(
            s1["by_model"]["m1"]["completion_tokens"],
            serde_json::json!(25)
        );
        assert_eq!(s1["by_day"]["2026-09-25"]["requests"], serde_json::json!(2));
        assert_eq!(s1["first_ts"], serde_json::json!(base));
        // Incremental: una línea más solo suma su parte.
        {
            let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
            let _ = f.write_all(line(base + 120, "m2", 1, 2).as_bytes());
        }
        let s2: serde_json::Value = serde_json::from_str(&usage_summary_at(&log, &cache)).unwrap();
        assert_eq!(s2["totals"]["requests"], serde_json::json!(3));
        assert_eq!(s2["by_model"]["m2"]["prompt_tokens"], serde_json::json!(1));
        // Rotación (log más corto que el offset): rebuild desde cero.
        {
            let mut f = std::fs::File::create(&log).unwrap();
            let _ = f.write_all(line(base + 200, "m3", 7, 8).as_bytes());
        }
        let s3: serde_json::Value = serde_json::from_str(&usage_summary_at(&log, &cache)).unwrap();
        assert_eq!(s3["totals"]["requests"], serde_json::json!(1));
        assert_eq!(
            s3["by_model"]["m3"]["completion_tokens"],
            serde_json::json!(8)
        );
        assert!(s3.get("by_model").and_then(|m| m.get("m1")).is_none());
        assert!(!s3.to_string().contains("NaN"));
        // Raw: últimas n con skipped.
        let raw: serde_json::Value = serde_json::from_str(&usage_raw_at(&log, 500)).unwrap();
        assert_eq!(raw["lines"].as_array().map(|a| a.len()), Some(1));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
