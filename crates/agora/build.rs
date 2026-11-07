//! Embeds the built web app (`apps/web/dist`) into the binary so `agora hub run` serves it with no extra files.
//! When the app has not been built (a plain `cargo build`, CI's Rust jobs) nothing is embedded and `--web DIR` is the
//! way to serve it. Release builds run `npm run build` in `apps/web` first.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        if path.is_dir() {
            walk(&path, root, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            let name = rel.to_string_lossy().replace('\\', "/");
            out.push((name, path));
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let dist = manifest.join("../../apps/web/dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    println!("cargo:rerun-if-env-changed=AGORA_NO_EMBED_WEB");
    let mut files = Vec::new();
    if dist.join("index.html").is_file() && std::env::var_os("AGORA_NO_EMBED_WEB").is_none() {
        walk(&dist, &dist, &mut files);
    }
    let mut src = String::from("pub static WEB_FILES: &[(&str, &[u8])] = &[\n");
    for (name, path) in &files {
        let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
        let _ = writeln!(
            src,
            "    ({name:?}, include_bytes!({:?})),",
            abs.display().to_string()
        );
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("web_files.rs");
    if let Err(e) = std::fs::write(&out, src) {
        println!("cargo:warning=could not write {}: {e}", out.display());
    }
}
