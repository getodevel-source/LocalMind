use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
pub use crate::config::HardwareProfile;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareProfileDto {
    pub id: String,
    pub name: String,
    pub description: String,
    pub context: usize,
    pub cache_ram: usize,
    pub extra_flags: Vec<String>,
}

impl From<HardwareProfile> for HardwareProfileDto {
    fn from(p: HardwareProfile) -> Self {
        Self {
            id: p.id,
            name: p.name,
            description: p.description,
            context: p.context,
            cache_ram: p.cache_ram,
            extra_flags: p.extra_flags,
        }
    }
}

/// Perfiles expuestos a la UI con datos completos desde el TOML.
pub fn get_hardware_profiles(cfg: &AppConfig) -> Vec<HardwareProfileDto> {
    cfg.profiles.iter().cloned().map(Into::into).collect()
}

/// Resolver un perfil por id; fallback al primero disponible.
pub fn resolve_profile(profiles: &[HardwareProfile], id: &str) -> HardwareProfile {
    profiles
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .unwrap_or_else(|| profiles.first().cloned().expect("no hardware profiles configured"))
}
