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
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            device: "Vulkan0".to_string(),
            threads: None,
            threads_batch: None,
            priority: "2".to_string(),
            batch: 2048,
            process_priority: "below_normal".to_string(),
            poll: 0,
            limit_threads_batch: true,
            extra_flags: Vec::new(),
            aliases: default_aliases(),
            speculation: Some(SpeculationConfig::default()),
            flash_attention: true,
            reasoning_preserve: true,
            metrics: true,
            http_port: 17860,
            llama_port: 8080,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MmprojConfig {
    /// true → detectar automáticamente el mmproj más afín al modelo cargado
    #[serde(default = "default_true")]
    pub auto: bool,
    /// Lista de candidatos (orden de prioridad) en models/
    #[serde(default = "default_mmproj_files")]
    pub files: Vec<String>,
}

impl Default for MmprojConfig {
    fn default() -> Self {
        Self {
            auto: true,
            files: default_mmproj_files(),
        }
    }
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
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            engine: EngineConfig::default(),
            mmproj: MmprojConfig::default(),
            profiles: built_in_profiles(),
            last: LastSettings::default(),
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
fn default_batch() -> usize {
    2048
}
fn default_process_priority() -> String {
    "below_normal".to_string()
}
fn default_poll() -> u32 {
    0
}
fn default_http_port() -> u16 {
    17860
}
fn default_llama_port() -> u16 {
    8080
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
fn default_aliases() -> Vec<String> {
    vec![
        "localmind".to_string(),
        "qwen3.8-27b".to_string(),
        "bonsai-2-27b".to_string(),
    ]
}
fn default_mmproj_files() -> Vec<String> {
    vec![
        "mmproj-BF16.gguf".to_string(),
        "mmproj-F16.gguf".to_string(),
        "Ternary-Bonsai-2-27B-mmproj-Q8_0.gguf".to_string(),
    ]
}

/// Perfiles integrados (ajustados para RX 6800 XT 16GB). Sobrescribibles vía [[profiles]] en el TOML.
pub fn built_in_profiles() -> Vec<HardwareProfile> {
    vec![
        HardwareProfile {
            id: "turbo".to_string(),
            name: "Turbo VRAM (32K · Máxima Velocidad T/S)".to_string(),
            description: "100% en VRAM (RX 6800 XT). Máximo rendimiento y mínima latencia (pico de t/s)."
                .to_string(),
            context: 32768,
            cache_ram: 0,
            extra_flags: vec!["--cache-reuse".to_string(), "256".to_string()],
        },
        HardwareProfile {
            id: "balanced".to_string(),
            name: "Equilibrado Pro (64K · Documentos y Código)".to_string(),
            description: "Pesos en VRAM + KV Cache extendido en VRAM/RAM DDR5.".to_string(),
            context: 65536,
            cache_ram: 4096,
            extra_flags: vec!["--cache-reuse".to_string(), "256".to_string()],
        },
        HardwareProfile {
            id: "deep".to_string(),
            name: "Extendido 128K (Libros y Repositorios)".to_string(),
            description: "128K tokens usando VRAM + RAM compartida optimizada.".to_string(),
            context: 131072,
            cache_ram: 6144,
            extra_flags: vec!["--cache-reuse".to_string(), "256".to_string()],
        },
        HardwareProfile {
            id: "ultra".to_string(),
            name: "Límite Hardware 262K (Máximo Contexto Físico)".to_string(),
            description: "Ventana nativa máxima (262,144 tokens) combinando 100% VRAM + RAM DDR5 segura."
                .to_string(),
            context: 262144,
            cache_ram: 6144,
            extra_flags: vec![
                "-kvu".to_string(),
                "--cache-reuse".to_string(),
                "512".to_string(),
            ],
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
        let cfg = raw
            .as_ref()
            .and_then(|r| toml::from_str::<AppConfig>(r).ok())
            .unwrap_or_default();
        if raw.is_none() {
            // Primera ejecución: materializar plantilla de defaults
            if let Ok(text) = toml::to_string_pretty(&cfg) {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&path, text);
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
