//! Generates Rust types from `schema/agora/v1/*.proto` without `protoc`.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Read at run time: `env!` would bake in the folder this script was compiled in, and a build folder shared by two checkouts (or a
    // deleted worktree) would then read, and watch, the wrong `schema/`.
    let schema = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?).join("../../schema");
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
    // Keep the descriptor set so tests (and later tools) can reflect over messages.
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    std::fs::write(
        out.join("agora_descriptor.bin"),
        prost::Message::encode_to_vec(&fds),
    )?;
    prost_build::compile_fds(fds)?;
    Ok(())
}
