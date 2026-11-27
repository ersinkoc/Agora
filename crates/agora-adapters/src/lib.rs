//! Agent integration levels: L1 manifest engine, L2 ACP client, L3 adapter host. See docs/specs/agent-manifest.md and docs/specs/adapter-protocol.md.

pub mod acp;
pub mod agent_install;
pub mod agent_packages;
pub mod gate;
pub mod host;
pub mod inject;
pub mod manifest;
pub mod pacer;
pub mod package;
pub mod perm;
pub mod registry;
pub mod registry_convert;
pub mod rules;
pub mod text;
pub mod usage;

pub use gate::{Gate, Timing};
pub use manifest::{Manifest, ManifestError};
pub use pacer::Pacer;
pub use rules::{Detection, Detector, Explanation, Region, RuleCheck, RuleOutcome};
