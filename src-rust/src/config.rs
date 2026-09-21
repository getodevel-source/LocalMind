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
            idle_timeout_secs: 1500,
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

/// Perfiles integrados universales. Sobrescribibles vía [[profiles]] en el TOML.
pub fn built_in_profiles() -> Vec<HardwareProfile> {
    vec![
        HardwareProfile {
            id: "velocidad".to_string(),
            name: "Velocidad máxima · 32K contextos cortos".to_string(),
            description: "Todo el modelo y su memoria de conversación (KV cache) viven íntegramente en la VRAM de la GPU. Es el modo más rápido con mínima latencia. Ideal para chat, preguntas y código corto (<30k palabras).".to_string(),
            context: 32768,
            cache_ram: 0,
            extra_flags: vec!["--cache-reuse".to_string(), "256".to_string()],
        },
        HardwareProfile {
            id: "multi_doc".to_string(),
            name: "Multi-documento · 64K (velocidad levemente menor)".to_string(),
            description: "El modelo reside en VRAM y parte del KV cache pasa a la memoria RAM del sistema. Cede ~5% de velocidad para sostener ~60k palabras de contexto: múltiples documentos o repositorios completos.".to_string(),
            context: 65536,
            cache_ram: 4096,
            extra_flags: vec!["--cache-reuse".to_string(), "256".to_string()],
        },
        HardwareProfile {
            id: "libros".to_string(),
            name: "Libros largos · 128K contexto extendido".to_string(),
            description: "Modelo en VRAM + KV cache ampliado en RAM del sistema: sostiene ~120k palabras (un libro entero o proyecto grande). Velocidad moderada (~15-20% menor que Velocidad máxima) al cursar tráfico por el bus de memoria.".to_string(),
            context: 131072,
            cache_ram: 6144,
            extra_flags: vec!["--cache-reuse".to_string(), "256".to_string()],
        },
        HardwareProfile {
            id: "max_contexto".to_string(),
            name: "Máximo contexto · 262K (límite físico del modelo)".to_string(),
            description: "Alcanza los 262,144 tokens que el modelo soporta nativamente (~200k palabras: bases de código masivas). Requiere memoria RAM compartida considerable; ideal para cargas analíticas profundas.".to_string(),
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
