//! Puerta de aceptación y decisiones puras del ciclo del motor.
//!
//! Extraído de `process.rs`: `gate_is_slow`, `should_auto_stop`, `engine_lost`,
//! `eta_secs`, `load_key`, `parse_adapter_ram` (+ `vram_used_mb`/`vram_total_mb`
//! con efecto comando, best-effort). Todo lo testeable vive aquí sin `Path`,
//! sin `ModelInfo`, sin red: el poller (`process.rs`) solo las llama.

/// `engine_slow` (puro): true si la mediana queda bajo el umbral configurable.
/// Solo informativo — el llamador nunca reintenta por esto (LM-NF-3).
pub(crate) fn gate_is_slow(median_tps: f64, slow_at: f64) -> bool {
    median_tps < slow_at
}

/// Umbral efectivo de puerta lenta según el tamaño del modelo (portabilidad).
/// El fijo 20 t/s se calibró para 27B en 16 GB: un 2B en integrada lo pasa
/// sobrado (falso "rápido" si va mal) y un 70B falla siempre (falso "lento").
/// Escalón por GB del .gguf (decode Vulkan medido: 27B ~26-42 t/s, 2.6B
/// ~187 t/s): ≤9 GB → 25, ≤17 GB → 20, >17 GB → 12. Desconocido (0) → cfg.
/// El cfg es TECHO: si el dueño lo baja, manda el suyo (más estricto gana).
pub(crate) fn slow_threshold(cfg_base: f64, size_gb: f64) -> f64 {
    let by_size = if size_gb <= 0.0 {
        cfg_base
    } else if size_gb < 9.0 {
        25.0
    } else if size_gb < 17.0 {
        20.0
    } else {
        12.0
    };
    cfg_base.min(by_size)
}

/// ETA en segundos: duración guardada para la misma clave; en frío,
/// `6 s × tamaño del modelo en GB` (0 si se desconoce el tamaño).
pub(crate) fn eta_secs(stored: Option<u64>, size_gb: f64) -> u64 {
    if let Some(s) = stored {
        return s;
    }
    if size_gb > 0.0 {
        (size_gb * 6.0).round() as u64
    } else {
        0
    }
}

/// Clave del registro de duraciones: nombre del modelo + contexto pedido (D2:
/// nunca se reduce el contexto a espaldas del usuario, así que la clave es exacta).
pub(crate) fn load_key(model: &str, context: usize) -> String {
    format!("{}|{}", model, context)
}

/// Decisión de auto-stop por inactividad (pura y testeable, Fase A5): el
/// llamador ya filtró `status == "running"`, timeout > 0 y motor no ocupado
/// (ocupado refresca la marca, nunca apaga). Sin marca previa (`last == 0`)
/// no se apaga: solo el paso del tiempo real dispara.
pub(crate) fn should_auto_stop(timeout_secs: u64, last_activity: u64, now_secs: u64) -> bool {
    timeout_secs > 0 && last_activity > 0 && now_secs.saturating_sub(last_activity) >= timeout_secs
}

/// Decisión de "motor perdido" (pura y testeable, Fase A5): 10 fallos seguidos
/// de `/health` (~10 s) con status `running` → el hijo murió sin exit visible.
/// Fuera de `running` no aplica (el arranque/cierre tienen su propio ciclo).
pub(crate) fn engine_lost(consecutive_failures: u32, status_running: bool) -> bool {
    consecutive_failures >= 10 && status_running
}

/// Puro: máximo `AdapterRAM` en MB desde salida `wmic ... /value`.
/// `AdapterRAM` es uint32 y muchos drivers AMD lo saturan: el valor exacto
/// `4294967295` (4 GB-1) es el caso conocido, pero hay drivers que reportan
/// otros techos falsos (p. ej. `4293918720` ≈ 4095 MB en una RX 6800 XT de
/// 16 GB). Regla: se acepta solo `n >= 8 GB` (descarta saturados parciales e
/// integradas sin dedicada); por debajo, `None` aunque haya dígitos.
/// `None` = sin adaptador válido.
pub(crate) fn parse_adapter_ram(text: &str) -> Option<u64> {
    parse_adapter_ram_min(text, 8 * 1024)
}

/// Núcleo testeable con umbral explícito en MB.
pub(crate) fn parse_adapter_ram_min(text: &str, min_mb: u64) -> Option<u64> {
    let mut best: Option<u64> = None;
    for line in text.lines() {
        let v = line.split('=').nth(1).unwrap_or("").trim();
        let digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<u64>() {
            if n == 4294967295 {
                continue;
            }
            let mb = n / (1024 * 1024);
            if mb < min_mb {
                continue;
            }
            best = Some(best.map_or(mb, |b: u64| b.max(mb)));
        }
    }
    best
}

/// VRAM en uso en MB (Windows, D-21, best-effort): `nvidia-smi` si existe
/// (parsea `memory.used`); si no, se devuelve `None` (el texto del motor queda
/// intacto: no hay contador barato y estable para AMD/Intel desde aquí, y no
/// se inventa). `None` = sin dato, nunca un error visible.
pub(crate) fn vram_used_mb() -> Option<u64> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    if let Ok(out) = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.used", "--format=csv,noheader,nounits"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        if out.status.success() {
            let digits: String = String::from_utf8_lossy(&out.stdout)
                .chars()
                .filter(|c| c.is_ascii_digit())
                .take(12)
                .collect();
            if let Ok(n) = digits.parse::<u64>() {
                return Some(n);
            }
        }
    }
    None
}

/// VRAM TOTAL instalada en MB (Windows, D-21, best-effort): WMI
/// `Win32_VideoController.AdapterRAM` (máximo entre adaptadores). No es USO, es
/// techo instalado: el llamador lo etiqueta como tal. `None` = sin dato.
pub(crate) fn vram_total_mb() -> Option<u64> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let out = std::process::Command::new("wmic")
        .args([
            "path",
            "Win32_VideoController",
            "get",
            "AdapterRAM",
            "/value",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_adapter_ram(&String::from_utf8_lossy(&out.stdout))
}

/// ¿Cabe `modelo + KV estimado` en la VRAM? (puro salvo lectura del .gguf).
/// `kv_mb` ≈ `contexto × 32 B/tok` (techo conservador: q4_0 ≈ 0.5 B por token
/// y capa × ~60 capas en 27B), menos `cache_ram` (MB que desbordan a RAM y NO
/// ocupan VRAM). Margen 0.9: la VRAM no se llena al 100% (driver +
/// framebuffer). `None` en VRAM total = sin veredicto (`Ok`, no se bloquea
/// sin dato). `Err` en español con GB concretos.
pub(crate) fn check_vram_fit(
    model_path: &std::path::Path,
    context: usize,
    cache_ram_mb: usize,
    vram_total: Option<u64>,
) -> Result<(), String> {
    let total = match vram_total {
        Some(t) => t,
        None => return Ok(()),
    };
    let model_mb = std::fs::metadata(model_path)
        .map(|m| m.len() / (1024 * 1024))
        .unwrap_or(0);
    let kv_mb =
        ((context as u64).saturating_mul(32) / (1024 * 1024)).saturating_sub(cache_ram_mb as u64);
    let need_mb = model_mb + kv_mb;
    let budget_mb = total * 9 / 10;
    if need_mb <= budget_mb {
        return Ok(());
    }
    Err(format!(
        "El modelo no cabe en la VRAM: necesita ~{} MB (pesos) + ~{} MB (KV a {}K) = ~{:.1} GB frente a {:.1} GB instalados (útil ~{:.1} GB). Usa un .gguf más pequeño o un perfil de menor contexto.",
        model_mb,
        kv_mb,
        context / 1024,
        need_mb as f64 / 1024.0,
        total as f64 / 1024.0,
        budget_mb as f64 / 1024.0,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D-21: `AdapterRAM` dice el TECHO instalado, no el uso. 16 GB =
    /// 17179869184 B → 16384 MB; gana el máximo entre adaptadores.
    /// Saturados (uint32 exacto o techos falsos <8 GB como los 4095 MB de una
    /// RX 6800 XT de 16 GB) y ruido <8 GB → `None`: mejor sin dato que con
    /// dato falso junto al real del motor.
    #[test]
    fn adapter_ram_da_techo_maximo_y_filtra_basura() {
        assert_eq!(
            parse_adapter_ram("AdapterRAM=17179869184\r\nAdapterRAM=17179869184\r\n"),
            Some(16384)
        );
        // Caso real medido 2026-10-09 (RX 6800 XT 16 GB): driver saturado.
        assert_eq!(parse_adapter_ram("AdapterRAM=4293918720\r\n"), None);
        assert_eq!(parse_adapter_ram("AdapterRAM=4294967295\r\n"), None);
        assert_eq!(parse_adapter_ram("AdapterRAM=2147483648\r\n"), None);
        assert_eq!(parse_adapter_ram("AdapterRAM=134217728\r\n"), None);
        assert_eq!(parse_adapter_ram(""), None);
        // Umbral explícito: con min 1 GB, 2 GB sí pasa (integradas honestas).
        assert_eq!(
            parse_adapter_ram_min("AdapterRAM=2147483648\r\n", 1024),
            Some(2048)
        );
    }
    /// Guard VRAM: 27B (~14 GB) + KV 32K con cache_ram del perfil en 16 GB
    /// pasa; en 8 GB falla con GB concretos; sin dato nunca bloquea.
    #[test]
    fn vram_fit_avisa_con_gb_y_no_bloquea_sin_dato() {
        let dir = std::env::temp_dir().join(format!("lm-vram-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let big = dir.join("grande.gguf");
        // 14 GB disperso: metadatos, no 14 GB reales en disco.
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(14 * 1024 * 1024 * 1024).unwrap();
        // KV 32K = 32768*32/1MiB = 1024 MB; con cache_ram 6144 del perfil:
        // 14336+1024-6144 = 9216 <= 14745 (16 GB*0.9) OK.
        assert!(check_vram_fit(&big, 32768, 6144, Some(16384)).is_ok());
        let err = check_vram_fit(&big, 32768, 0, Some(8192)).unwrap_err();
        assert!(err.contains("no cabe en la VRAM"), "{}", err);
        assert!(err.contains("GB"), "{}", err);
        assert!(check_vram_fit(&big, 32768, 0, None).is_ok());
        assert!(check_vram_fit(&big, 32768, 0, Some(32768)).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Umbral relativo: 2B→25, 27B→20, 70B→12, desconocido→cfg; el cfg como
    /// techo (dueño más estricto gana).
    #[test]
    fn slow_threshold_escala_con_tamano_y_cfg_es_techo() {
        assert_eq!(slow_threshold(20.0, 0.0), 20.0);
        assert_eq!(slow_threshold(20.0, 2.0), 20.0);
        assert_eq!(slow_threshold(99.0, 2.0), 25.0);
        assert_eq!(slow_threshold(99.0, 14.0), 20.0);
        assert_eq!(slow_threshold(99.0, 40.0), 12.0);
        assert_eq!(slow_threshold(10.0, 2.0), 10.0);
    }

    #[test]
    fn eta_stored_and_cold() {
        assert_eq!(eta_secs(Some(95), 13.0), 95);
        // Frío: 6 s × 13 GB = 78 s.
        assert_eq!(eta_secs(None, 13.0), 78);
        assert_eq!(eta_secs(None, 0.0), 0);
    }

    #[test]
    fn gate_lento_bajo_umbral() {
        // Umbral default 20: bajo → true; igual o más → false.
        assert!(gate_is_slow(19.9, 20.0));
        assert!(!gate_is_slow(20.0, 20.0));
        assert!(!gate_is_slow(34.0, 20.0));
    }

    #[test]
    fn auto_stop_borde_y_guardas() {
        // timeout 5400, última actividad hace 5400 → apaga (borde incluido).
        assert!(should_auto_stop(5400, 1000, 6400));
        assert!(should_auto_stop(5400, 1000, 9999));
        // Un segundo antes → no apaga.
        assert!(!should_auto_stop(5400, 1000, 6399));
        // Sin timeout o sin marca previa → nunca apaga.
        assert!(!should_auto_stop(0, 1000, 99999));
        assert!(!should_auto_stop(5400, 0, 99999));
    }

    #[test]
    fn motor_perdido_a_los_10_fallos_en_running() {
        assert!(!engine_lost(9, true));
        assert!(engine_lost(10, true));
        assert!(engine_lost(25, true));
        // Fuera de running no aplica (arranque/cierre tienen su ciclo).
        assert!(!engine_lost(10, false));
        assert!(!engine_lost(99, false));
    }
}
