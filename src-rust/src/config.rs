use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareProfile {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_context")]
    pub context: usize,
    #[serde(default)]
    pub cache_ram: usize,
    #[serde(default)]
    pub extra_flags: Vec<String>,
}

fn default_context() -> usize {
    32768
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeculationConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_spec_type")]
    pub ty: String,
    #[serde(default = "default_spec_n")]
    pub draft_n_max: usize,
    #[serde(default = "default_spec_p")]
    pub draft_p_split: f32,
}

impl Default for SpeculationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ty: "draft-mtp".to_string(),
            draft_n_max: 2,
            draft_p_split: 0.1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    #[serde(default = "default_device")]
    pub device: String,
    /// None → default 6 (ajustar a núcleos físicos de la CPU)
    #[serde(default)]
    pub threads: Option<usize>,
    #[serde(default)]
    pub threads_batch: Option<usize>,
    #[serde(default = "default_priority")]
    pub priority: String,
    /// Prioridad de hilos batch (--prio-batch). Desdoblada de --prio: batch en alta
    /// prioridad eleva hilos de cómputo por encima de foreground y satura la GPU
    /// (tirones); "0" = hilos batch sin elevar, --prio interactivo intacto.
    #[serde(default = "default_priority_batch")]
    pub priority_batch: String,
    #[serde(default = "default_batch")]
    pub batch: usize,
    /// Prioridad de proceso (Windows): 0=bajo, 1=abajo-normal, 2=normal. "below_normal"
    /// protege la interactividad del sistema en sesiones largas.
    #[serde(default = "default_process_priority")]
    pub process_priority: String,
    /// Nivel de polling (llama-server --poll). 0 = sin spin-wait (CPU/PSU descansan);
    /// 50 = default de llama.cpp (latencia mínima pero CPU 100% entre tokens).
    #[serde(default = "default_poll")]
    pub poll: u32,
    /// threads_batch separado: batch de prompt es el pico máximo de CPU/VRAM.
    #[serde(default = "default_true")]
    pub limit_threads_batch: bool,
    /// Auto-stop del motor tras N segundos sin actividad de generación. 0 = desactivado.
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_secs: u64,
    #[serde(default)]
    pub extra_flags: Vec<String>,
    #[serde(default = "default_aliases")]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub speculation: Option<SpeculationConfig>,
    #[serde(default = "default_true")]
    pub flash_attention: bool,
    #[serde(default = "default_true")]
    pub reasoning_preserve: bool,
    #[serde(default = "default_true")]
    pub metrics: bool,
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    /// Puerto preferido del motor (llama-server). Si está ocupado se busca el siguiente libre.
    #[serde(default = "default_llama_port")]
    pub llama_port: u16,
    /// Reutilización de prefijos de prompt en KV cache (--cache-reuse N).
    /// 0 = desactivado (default upstream). 256 = global LocalMind.
    #[serde(default = "default_cache_reuse")]
    pub cache_reuse: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            device: "Vulkan0".to_string(),
            threads: None,
            threads_batch: None,
            priority: "2".to_string(),
            priority_batch: "0".to_string(),
            batch: 1024,
            process_priority: "below_normal".to_string(),
            poll: 0,
            limit_threads_batch: true,
            idle_timeout_secs: 1500,
            extra_flags: Vec::new(),
            aliases: default_aliases(),
            speculation: None,
            flash_attention: true,
            reasoning_preserve: true,
            metrics: true,
            http_port: 17860,
            llama_port: 8080,
            cache_reuse: default_cache_reuse(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MmprojConfig {
    /// true → detectar automáticamente el mmproj más afín al modelo cargado
    #[serde(default = "default_false")]
    pub auto: bool,
    /// Lista de candidatos (orden de prioridad) en models/
    #[serde(default = "default_mmproj_files")]
    pub files: Vec<String>,
}

impl Default for MmprojConfig {
    fn default() -> Self {
        Self {
            auto: false,
            files: default_mmproj_files(),
        }
    }
}
/// Id de perfil por defecto: siempre uno real de `built_in_profiles()` (D-1:
/// el fantasma `turbo` nunca debe asomar en el estado ni en `last.profile`).
pub const DEFAULT_PROFILE_ID: &str = "velocidad";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationsConfig {
    /// Interruptor general de avisos de escritorio (P20).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Avisar cuando el motor pasa la puerta de aceptación.
    #[serde(default = "default_true")]
    pub on_ready: bool,
    /// Avisar cuando el motor falla (puerta o crash).
    #[serde(default = "default_true")]
    pub on_failure: bool,
    /// Avisar cuando el auto-stop apaga el motor.
    #[serde(default = "default_true")]
    pub on_autostop: bool,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            on_ready: true,
            on_failure: true,
            on_autostop: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationConfig {
    /// Temperatura de muestreo (0..=2). Base para exponer en la API (P16).
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    /// Top-p (0..=1).
    #[serde(default = "default_top_p")]
    pub top_p: f64,
    /// Máximo de tokens a generar (1..=32768).
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    /// Semilla (>= 0).
    #[serde(default)]
    pub seed: i64,
}

impl Default for GenerationConfig {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            top_p: 1.0,
            max_tokens: 2048,
            seed: 0,
        }
    }
}

/// Validadores de la API de ajustes (`GET/POST /api/config`, `POST
/// `/api/profiles/save|delete`). Sin pánicos: la UI envía parciales y el
/// servidor responde `400 {"error":"<mensaje español nombrando el campo>"}`.
/// (Antes `allow(dead_code)`: ahora los consume `server.rs`.)
pub fn gen_temperature_ok(v: f64) -> bool {
    v.is_finite() && (0.0..=2.0).contains(&v)
}
pub fn gen_top_p_ok(v: f64) -> bool {
    v.is_finite() && (0.0..=1.0).contains(&v)
}
pub fn gen_max_tokens_ok(v: usize) -> bool {
    (1..=32768).contains(&v)
}
pub fn gen_seed_ok(v: i64) -> bool {
    v >= 0
}
/// `idle_timeout_secs`: 0 = desactivado; si no, 60 s..7 días.
pub fn engine_idle_timeout_ok(v: u64) -> bool {
    v == 0 || (60..=604800).contains(&v)
}
/// `threads`: None = auto (núcleos físicos); si se fija, 1..=256.
pub fn engine_threads_ok(v: usize) -> bool {
    (1..=256).contains(&v)
}
/// Prioridad de hilos del motor (`--prio`): la UI ofrece `2` (alta) y `0`.
pub fn engine_priority_ok(v: &str) -> bool {
    v == "2" || v == "0"
}
/// Id de perfil: `^[a-z0-9_-]{1,32}$` (lo que la UI usa como clave).
pub fn profile_id_ok(id: &str) -> bool {
    if id.is_empty() || id.len() > 32 {
        return false;
    }
    id.bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}
/// Contexto de perfil: 1024..=1048576 en pasos de 1024.
pub fn profile_context_ok(v: usize) -> bool {
    (1024..=1048576).contains(&v) && v % 1024 == 0
}
/// `cache_ram` (MB): 0..=65536.
pub fn profile_cache_ram_ok(v: usize) -> bool {
    v <= 65536
}
/// Una flag extra: ≤64 chars, charset `[A-Za-z0-9 _.=:/-]`.
pub fn profile_flag_ok(f: &str) -> bool {
    if f.is_empty() || f.len() > 64 {
        return false;
    }
    f.bytes().all(|b| {
        b.is_ascii_alphanumeric() || matches!(b, b' ' | b'_' | b'.' | b'=' | b':' | b'/' | b'-')
    })
}
/// La lista completa de flags: ≤8 entradas.
pub fn profile_flags_ok(flags: &[String]) -> bool {
    flags.len() <= 8 && flags.iter().all(|f| profile_flag_ok(f))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LastSettings {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub context: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub engine: EngineConfig,
    #[serde(default)]
    pub mmproj: MmprojConfig,
    /// Vacío → perfiles integrados
    #[serde(default)]
    pub profiles: Vec<HardwareProfile>,
    #[serde(default)]
    pub last: LastSettings,
    /// Avisos de escritorio (P20). `default` para TOMLs viejos sin sección.
    #[serde(default)]
    pub notifications: NotificationsConfig,
    /// Parámetros de generación (P16). La puerta usa temperature 0 fijo.
    #[serde(default)]
    pub generation: GenerationConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            engine: EngineConfig::default(),
            mmproj: MmprojConfig::default(),
            profiles: built_in_profiles(),
            last: LastSettings::default(),
            notifications: NotificationsConfig::default(),
            generation: GenerationConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Defaults helpers
// ---------------------------------------------------------------------------

fn default_true() -> bool {
    true
}
fn default_device() -> String {
    "Vulkan0".to_string()
}
fn default_priority() -> String {
    "2".to_string()
}
fn default_priority_batch() -> String {
    "0".to_string()
}
fn default_batch() -> usize {
    1024
}
fn default_process_priority() -> String {
    "below_normal".to_string()
}
fn default_poll() -> u32 {
    0
}
fn default_idle_timeout() -> u64 {
    1500 // 25 min
}
fn default_false() -> bool {
    false
}
fn default_http_port() -> u16 {
    17860
}
fn default_llama_port() -> u16 {
    8080
}
fn default_cache_reuse() -> usize {
    256
}
fn default_spec_type() -> String {
    "draft-mtp".to_string()
}
fn default_spec_n() -> usize {
    2
}
fn default_spec_p() -> f32 {
    0.1
}
fn default_temperature() -> f64 {
    0.7
}
fn default_top_p() -> f64 {
    1.0
}
fn default_max_tokens() -> usize {
    2048
}
fn default_aliases() -> Vec<String> {
    vec![
        "localmind".to_string(),
        "qwen3.8-27b".to_string(),
    ]
}
fn default_mmproj_files() -> Vec<String> {
    vec![
        "mmproj-BF16.gguf".to_string(),
        "mmproj-F16.gguf".to_string(),
    ]
}

/// Perfiles integrados universales. Sobrescribibles vía [[profiles]] en el TOML.
pub fn built_in_profiles() -> Vec<HardwareProfile> {
    vec![
        HardwareProfile {
            id: "velocidad".to_string(),
            name: "Velocidad máxima · 32K contextos cortos".to_string(),
            description: "Todo el modelo y su memoria de conversación (KV cache) viven íntegramente en la VRAM de la GPU. Es el modo más rápido con mínima latencia. Ideal para chat, preguntas y código corto (<30k palabras).".to_string(),
            context: 32768,
            cache_ram: 0,
            extra_flags: Vec::new(),
        },
        HardwareProfile {
            id: "multi_doc".to_string(),
            name: "Multi-documento · 64K (velocidad levemente menor)".to_string(),
            description: "El modelo reside en VRAM y parte del KV cache pasa a la memoria RAM del sistema. Cede ~5% de velocidad para sostener ~60k palabras de contexto: múltiples documentos o repositorios completos.".to_string(),
            context: 65536,
            cache_ram: 4096,
            extra_flags: Vec::new(),
        },
        HardwareProfile {
            id: "libros".to_string(),
            name: "Libros largos · 128K contexto extendido".to_string(),
            description: "Modelo en VRAM + KV cache ampliado en RAM del sistema: sostiene ~120k palabras (un libro entero o proyecto grande). Velocidad moderada (~15-20% menor que Velocidad máxima) al cursar tráfico por el bus de memoria.".to_string(),
            context: 131072,
            cache_ram: 6144,
            extra_flags: Vec::new(),
        },
        HardwareProfile {
            id: "max_contexto".to_string(),
            name: "Máximo contexto · 262K (límite físico del modelo)".to_string(),
            description: "Alcanza los 262,144 tokens que el modelo soporta nativamente (~200k palabras: bases de código masivas). Requiere memoria RAM compartida considerable; ideal para cargas analíticas profundas.".to_string(),
            context: 262144,
            // Barrido --cache-ram medido 2026-09-25 (bench.mjs, 262k/p512/m256,
            // Qwen3.8-27B IQ4_XS en RX 6800 XT 16 GB; gen_tps de /api/metrics):
            //   0 → 14.10 t/s · 4096 → 14.18 t/s · 6144 → 14.16 t/s · 8192 → 14.20 t/s
            // Sin -kvu (cache_ram 6144) → 11.48 t/s: -kvu SÍ ayuda, se conserva.
            // Conclusión: --cache-ram no mueve el cuello a 262K (plano ±0.1 t/s);
            // se mantiene 6144. No tocar -ub/-b/hilos/spec en este perfil (PSU).
            cache_ram: 6144,
            extra_flags: vec!["-kvu".to_string()],
        },
    ]
}

// ---------------------------------------------------------------------------
// ConfigStore: carga/escritura del TOML en %APPDATA%\LocalMind\localmind.toml
// ---------------------------------------------------------------------------

pub struct ConfigStore {
    path: PathBuf,
    inner: Arc<RwLock<AppConfig>>,
}

impl ConfigStore {
    pub fn load(base_dir: &Path) -> Self {
        let path = Self::config_path(base_dir);
        let raw = std::fs::read_to_string(&path).ok();
        let mut cfg = raw
            .as_ref()
            .and_then(|r| toml::from_str::<AppConfig>(r).ok())
            .unwrap_or_default();
        // Migración: ids de perfiles de la era anterior → nombres actuales.
        let legacy = ["turbo", "balanced", "deep", "ultra"];
        if cfg.profiles.iter().any(|p| legacy.contains(&p.id.as_str())) {
            let last = cfg.last.clone();
            cfg = AppConfig::default();
            cfg.last = last;
        }
        // D-1: si no hay perfiles, reponer los integrados; si `last.profile`
        // apunta a un id inexistente (p. ej. `turbo` heredado), caer al default
        // real para que el estado nunca exponga un perfil fantasma.
        if cfg.profiles.is_empty() {
            cfg.profiles = built_in_profiles();
        }
        let known = |id: &str| cfg.profiles.iter().any(|p| p.id == id);
        if cfg.last.profile.as_deref().is_some_and(|id| !known(id)) {
            cfg.last.profile = Some(DEFAULT_PROFILE_ID.to_string());
            cfg.last.context = None;
        }
        {
            let migrated = raw.is_some() && cfg.profiles.iter().all(|p| !["turbo","balanced","deep","ultra"].contains(&p.id.as_str()));
            if raw.is_none() || migrated {
                // Primera ejecución o migración de perfiles: materializar plantilla
                if let Ok(text) = toml::to_string_pretty(&cfg) {
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&path, text);
                }
            }
        }
        Self {
            path,
            inner: Arc::new(RwLock::new(cfg)),
        }
    }

    fn config_path(base_dir: &Path) -> PathBuf {
        std::env::var("APPDATA")
            .map(|p| Path::new(&p).join("LocalMind").join("localmind.toml"))
            .unwrap_or_else(|_| base_dir.join("localmind.toml"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self) -> AppConfig {
        self.inner.read().clone()
    }

    pub fn update(&self, f: impl FnOnce(&mut AppConfig)) {
        f(&mut self.inner.write());
    }

    /// Persistir en disco (best-effort).
    pub fn save(&self) -> Result<(), String> {
        let snapshot = self.inner.read().clone();
        let text = toml::to_string_pretty(&snapshot).map_err(|e| e.to_string())?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&self.path, text).map_err(|e| e.to_string())
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validador_id_perfil() {
        assert!(profile_id_ok("perfil_1"));
        assert!(profile_id_ok("a-b-c_123"));
        assert!(profile_id_ok("velocidad"));
        assert!(profile_id_ok("x"));
        // Longitud máxima 32 chars
        let exact32 = "a".repeat(32);
        assert!(profile_id_ok(&exact32));
        let too_long = "a".repeat(33);
        assert!(!profile_id_ok(&too_long));
        // Caracteres inválidos
        assert!(!profile_id_ok(""));
        assert!(!profile_id_ok("Perfil")); // mayúsculas
        assert!(!profile_id_ok("perfil con espacio"));
        assert!(!profile_id_ok("perfil/sub"));
        assert!(!profile_id_ok("perfil.punto"));
        assert!(!profile_id_ok("perfil;cmd"));
    }

    #[test]
    fn validador_contexto_y_cache_ram() {
        assert!(profile_context_ok(1024));
        assert!(profile_context_ok(32768));
        assert!(profile_context_ok(1048576));
        assert!(!profile_context_ok(0));
        assert!(!profile_context_ok(1023));
        assert!(!profile_context_ok(1025)); // no es múltiplo de 1024
        assert!(!profile_context_ok(1048576 + 1024)); // fuera de rango

        assert!(profile_cache_ram_ok(0));
        assert!(profile_cache_ram_ok(4096));
        assert!(profile_cache_ram_ok(65536));
        assert!(!profile_cache_ram_ok(65537));
    }

    #[test]
    fn validador_extra_flags() {
        let ok_flags = vec!["-kvu".to_string(), "--flash-attn".to_string(), "foo=bar:1/2".to_string()];
        assert!(profile_flags_ok(&ok_flags));
        assert!(profile_flags_ok(&[]));
        // Máximo 8 flags
        let nine = (0..9).map(|i| format!("-f{}", i)).collect::<Vec<_>>();
        assert!(!profile_flags_ok(&nine));
        // Flag vacía o >64 chars
        assert!(!profile_flags_ok(&["".to_string()]));
        let long_flag = "a".repeat(65);
        assert!(!profile_flags_ok(&[long_flag]));
        // Caracteres prohibidos (inyección cmd / shell)
        assert!(!profile_flags_ok(&["-flag & calc.exe".to_string()]));
        assert!(!profile_flags_ok(&["-flag; reboot".to_string()]));
        assert!(!profile_flags_ok(&["-flag | echo".to_string()]));
        assert!(!profile_flags_ok(&["-flag\"quote".to_string()]));
    }

    #[test]
    fn validador_motor_y_generacion() {
        assert!(engine_idle_timeout_ok(0));
        assert!(engine_idle_timeout_ok(60));
        assert!(engine_idle_timeout_ok(1500));
        assert!(engine_idle_timeout_ok(604800));
        assert!(!engine_idle_timeout_ok(59)); // <60 excepto 0
        assert!(!engine_idle_timeout_ok(604801));

        assert!(engine_threads_ok(1));
        assert!(engine_threads_ok(6));
        assert!(engine_threads_ok(256));
        assert!(!engine_threads_ok(0));
        assert!(!engine_threads_ok(257));

        assert!(engine_priority_ok("2"));
        assert!(engine_priority_ok("0"));
        assert!(!engine_priority_ok("1"));
        assert!(!engine_priority_ok("high"));

        assert!(gen_temperature_ok(0.0));
        assert!(gen_temperature_ok(0.7));
        assert!(gen_temperature_ok(2.0));
        assert!(!gen_temperature_ok(-0.1));
        assert!(!gen_temperature_ok(2.1));
        assert!(!gen_temperature_ok(f64::NAN));

        assert!(gen_top_p_ok(0.0));
        assert!(gen_top_p_ok(1.0));
        assert!(!gen_top_p_ok(1.01));

        assert!(gen_max_tokens_ok(1));
        assert!(gen_max_tokens_ok(32768));
        assert!(!gen_max_tokens_ok(0));
        assert!(!gen_max_tokens_ok(32769));

        assert!(gen_seed_ok(0));
        assert!(gen_seed_ok(42));
        assert!(!gen_seed_ok(-1));
    }
}
