//! Generates Rust types from `schema/agora/v1/*.proto` without `protoc`.

use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema");
    let mut protos: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(schema.join("agora/v1"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "proto") {
            protos.push(path);
        }
    }
    protos.sort();
    println!("cargo:rerun-if-changed={}", schema.display());
    let fds = protox::compile(&protos, [&schema])?;
    prost_build::compile_fds(fds)?;
    Ok(())
}
