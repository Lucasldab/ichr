//! Java runtime management — download (Mojang + Adoptium), system detection,
//! per-instance override, and launch-time resolution.
//! See `.planning/phases/05-java-runtime-management/05-RESEARCH.md`.

pub mod mapping;
pub mod mojang_jre;
pub mod types;
// Populated by plans 05-04..05-06:
// pub mod adoptium;    // 05-04
// pub mod detect;      // 05-05
// pub mod service;     // 05-06
