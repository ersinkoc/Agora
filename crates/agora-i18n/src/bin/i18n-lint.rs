//! `i18n-lint [repo-root]`: exits 1 on errors, prints warnings.

use std::path::PathBuf;
use std::process::ExitCode;

use agora_i18n::lint::{run, Severity};

fn main() -> ExitCode {
    let root = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
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
