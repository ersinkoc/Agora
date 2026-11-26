//! `i18n-lint [repo-root]`: exits 1 on errors, prints warnings.

use std::path::PathBuf;
use std::process::ExitCode;

use agora_i18n::lint::{run, Severity};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let root = match (args.next(), args.next()) {
        (None, None) => PathBuf::from("."),
        (Some(root), None) => PathBuf::from(root),
        _ => {
            eprintln!("usage: i18n-lint [repo-root]");
            return ExitCode::from(2);
        }
    };
    match run(&root) {
        Ok(findings) => {
            let mut errors = 0;
            for f in &findings {
                let tag = match f.severity {
                    Severity::Error => {
                        errors += 1;
                        "error"
                    }
                    Severity::Warning => "warning",
                };
                eprintln!("{tag}: {}", f.message);
            }
            if errors > 0 {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("i18n-lint: {e}");
            ExitCode::from(2)
        }
    }
}
