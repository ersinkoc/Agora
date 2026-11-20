//! Regenerate `manifests/agents/registry/` from a clone of the ACP registry (ADR-0092). Runs on a maintainer's machine, never on a node:
//! `cargo run -p agora-adapters --example registry_convert -- <registry clone> manifests/agents/registry`
//! Writes `<id>.toml` and `<id>.package.json` for each converted entry and prints `skipped <id> <reason code>` for the others.

use std::{fs, path::Path, process::ExitCode};

use agora_adapters::registry_convert::convert;

fn main() -> ExitCode {
    let mut a = std::env::args().skip(1);
    let (Some(src), Some(out)) = (a.next(), a.next()) else {
        eprintln!("usage: registry_convert <registry clone> <output dir>");
        return ExitCode::from(2);
    };
    match run(Path::new(&src), Path::new(&out)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("registry_convert: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(src: &Path, out: &Path) -> std::io::Result<()> {
    fs::create_dir_all(out)?;
    let mut dirs: Vec<_> = fs::read_dir(src)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    dirs.sort();
    for dir in dirs {
        let file = dir.join("agent.json");
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let id = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        match convert(&text) {
            Ok(c) => {
                // The file is named by the id the converter validated; a directory that disagrees with it is refused.
                let valid = c.package["source"]["id"].as_str().unwrap_or("");
                if valid != id {
                    println!("skipped {id} registry.id_mismatch");
                    continue;
                }
                fs::write(out.join(format!("{id}.toml")), &c.manifest)?;
                let json = serde_json::to_string_pretty(&c.package).unwrap_or_default();
                fs::write(out.join(format!("{id}.package.json")), json + "\n")?;
                for n in c.notes {
                    println!("partial {id} {} {}", n.distribution, n.reason.code());
                }
            }
            Err(r) => println!("skipped {id} {}", r.code()),
        }
    }
    Ok(())
}
