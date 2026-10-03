// Shim de compatibilidad (Fase A3, vuelta a raíces): los perfiles viven ahora
// en `config.rs`. Este módulo solo re-exporta para no romper los ~21 puntos de
// uso (`crate::profiles::...`) en `server.rs`/`process.rs` en el mismo commit.
// Próximo paso: migrar los usos a `crate::config::` y eliminar este archivo.

pub use crate::config::{get_hardware_profiles, resolve_profile, HardwareProfile, HardwareProfileDto};
