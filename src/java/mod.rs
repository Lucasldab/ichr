//! Java runtime management — download (Mojang + Adoptium), system detection,
//! per-instance override, and launch-time resolution.
//! See `.planning/phases/05-java-runtime-management/05-RESEARCH.md`.

pub mod adoptium;
pub mod detect;
pub mod mapping;
pub mod mojang_jre;
pub mod types;
// Populated by plan 05-06:
// pub mod service;     // 05-06
