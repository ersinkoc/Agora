//! Regenerate `manifests/agents/registry/` from a clone of the ACP registry (ADR-0092). Runs on a maintainer's machine, never on a node:
//! `cargo run -p agora-adapters --example registry_convert -- <registry clone> manifests/agents/registry <commit> <date>`
//! The clone is read as data only: each `<id>/agent.json`, `<id>/icon.svg` and the registry's `quarantine.json`. Nothing in it is run.
//! Writes `<id>.toml` and `<id>.package.json` for each converted entry, `icons/<id>.svg` for an icon that passes `check_icon`,
//! `catalog.json` (what the window's list shows), `SKIPPED` (each entry left out, with its reason code) and `SOURCE` (the commit and
//! date). The files it wrote last time are removed first, so an agent the registry dropped goes too. It prints `skipped`, `partial`
//! and `icon` lines for the review.

use std::{fs, path::Path, process::ExitCode};

use agora_adapters::registry_convert::{
    catalog_entry, check_icon, convert, quarantine_list, Reason, GENERATED,
};

fn main() -> ExitCode {
    let mut a = std::env::args().skip(1);
    let (Some(src), Some(out), Some(commit), Some(date)) = (a.next(), a.next(), a.next(), a.next())
    else {
        eprintln!(
            "usage: registry_convert <registry clone> <output dir> <registry commit> <commit date>"
        );
        return ExitCode::from(2);
    };
    let plain = |s: &str| {
        !s.is_empty()
            && s.len() <= 64
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-:+.".contains(c))
    };
    if !plain(&commit) || !plain(&date) {
        eprintln!("registry_convert: the commit and the date are plain words");
        return ExitCode::from(2);
    }
    match run(Path::new(&src), Path::new(&out), &commit, &date) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("registry_convert: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Removes what an earlier run wrote: the generated files only, never anything a person put there.
fn clear(out: &Path) -> std::io::Result<()> {
    for e in fs::read_dir(out)?.filter_map(Result::ok) {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(".toml")
            || name.ends_with(".package.json")
            || ["catalog.json", "SKIPPED", "SOURCE"].contains(&name.as_str())
        {
            fs::remove_file(e.path())?;
        }
    }
    for e in fs::read_dir(out.join("icons"))?.filter_map(Result::ok) {
        if e.file_name().to_string_lossy().ends_with(".svg") {
            fs::remove_file(e.path())?;
        }
    }
    Ok(())
}

fn run(src: &Path, out: &Path, commit: &str, date: &str) -> std::io::Result<()> {
    fs::create_dir_all(out.join("icons"))?;
    clear(out)?;
    // Required: without it the entries the registry itself holds back would be converted (#1133 review F8).
    let text = fs::read_to_string(src.join("quarantine.json")).map_err(|e| {
        std::io::Error::other(format!(
            "the registry clone has no readable quarantine.json: {e}"
        ))
    })?;
    let quarantine = quarantine_list(&text).map_err(|r| std::io::Error::other(r.code()))?;
    let mut dirs: Vec<_> = fs::read_dir(src)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    dirs.sort();
    let mut catalog = Vec::new();
    let mut skipped = Vec::new();
    let mut entries = 0;
    for dir in dirs {
        let Ok(text) = fs::read_to_string(dir.join("agent.json")) else {
            continue;
        };
        entries += 1;
        let id = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let mut skip = |code: &str| {
            println!("skipped {id} {code}");
            skipped.push(format!("{id} {code}"));
        };
        if quarantine.contains_key(&id) {
            skip(Reason::Quarantined.code());
            continue;
        }
        let c = match convert(&text) {
            Ok(c) => c,
            Err(r) => {
                skip(r.code());
                continue;
            }
        };
        // The file is named by the id the converter validated; a directory that disagrees with it is refused.
        if c.package["source"]["id"].as_str() != Some(id.as_str()) {
            skip("registry.id_mismatch");
            continue;
        }
        fs::write(out.join(format!("{id}.toml")), &c.manifest)?;
        let json = serde_json::to_string_pretty(&c.package).unwrap_or_default();
        fs::write(out.join(format!("{id}.package.json")), json + "\n")?;
        for n in &c.notes {
            println!("partial {id} {} {}", n.distribution, n.reason.code());
        }
        let icon = match fs::read(dir.join("icon.svg")).map(|b| check_icon(&b)) {
            Ok(Ok(svg)) => {
                fs::write(out.join("icons").join(format!("{id}.svg")), svg + "\n")?;
                true
            }
            Ok(Err(r)) => {
                println!("icon {id} {}", r.code());
                false
            }
            Err(_) => false,
        };
        catalog.push(catalog_entry(&c, icon));
    }
    let catalog = serde_json::json!({ "_generated": GENERATED, "registry_commit": commit, "agents": catalog });
    fs::write(
        out.join("catalog.json"),
        serde_json::to_string_pretty(&catalog).unwrap_or_default() + "\n",
    )?;
    let mut text = format!("# {GENERATED}\n# Registry entries left out, with the reason code.\n");
    for s in &skipped {
        text.push_str(s);
        text.push('\n');
    }
    fs::write(out.join("SKIPPED"), text)?;
    fs::write(
        out.join("SOURCE"),
        format!(
            "# {GENERATED}\nregistry = https://github.com/agentclientprotocol/registry\ncommit = {commit}\ndate = {date}\nentries = {entries}\nconverted = {}\nskipped = {}\n",
            entries - skipped.len(),
            skipped.len()
        ),
    )?;
    Ok(())
}
