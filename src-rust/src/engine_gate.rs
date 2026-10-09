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
