use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareProfile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub context: usize,
    pub cache_ram: usize,
    pub extra_flags: Vec<String>,
}

pub fn get_hardware_profiles() -> Vec<HardwareProfile> {
    vec![
        HardwareProfile {
            id: "turbo".to_string(),
            name: "Turbo VRAM (32K · Máxima Velocidad T/S)".to_string(),
            description: "100% en VRAM (RX 6800 XT). Máximo rendimiento y mínima latencia (pico de t/s).".to_string(),
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
            description: "Ventana nativa máxima (262,144 tokens) combinando 100% VRAM + RAM DDR5 segura.".to_string(),
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

pub fn find_profile(id: &str) -> HardwareProfile {
    let profiles = get_hardware_profiles();
    for p in profiles {
        if p.id == id {
            return p;
        }
    }
    get_hardware_profiles().into_iter().next().unwrap()
}
