//! Agent integration levels: L1 manifest engine, L2 ACP client, L3 adapter host. See docs/specs/agent-manifest.md and docs/specs/adapter-protocol.md.

pub mod acp;
pub mod host;
pub mod inject;
pub mod manifest;
pub mod pacer;
pub mod registry;
pub mod rules;

pub use manifest::{Manifest, ManifestError};
pub use pacer::Pacer;
pub use rules::{Detection, Detector};
