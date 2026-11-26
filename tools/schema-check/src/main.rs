//! `schema-check BASE_SCHEMA_DIR HEAD_SCHEMA_DIR`: exit 1 if HEAD breaks anything BASE published.
//! An existing base directory without any schema (first introduction) passes; the head must contain a schema.

use std::path::Path;
use std::process::ExitCode;

use agora_schema_check::{breaking_changes, load};

fn require_directory(path: &str) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(format!("{path}: not a directory")),
        Err(error) => Err(format!("{path}: {error}")),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [base, head] = &args[..] else {
        eprintln!("usage: schema-check BASE_SCHEMA_DIR HEAD_SCHEMA_DIR");
        return ExitCode::from(2);
    };
    if let Err(error) = require_directory(base).and_then(|()| require_directory(head)) {
        eprintln!("schema-check: {error}");
        return ExitCode::from(2);
    }
    let (base, head) = match (load(Path::new(base)), load(Path::new(head))) {
        (Ok(b), Ok(h)) => (b, h),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("schema-check: {e}");
            return ExitCode::from(2);
        }
    };
    if head.file.is_empty() {
        eprintln!("schema-check: head schema has no .proto files");
        return ExitCode::from(2);
    }
    let problems = breaking_changes(&base, &head);
    for p in &problems {
        eprintln!("breaking: {p}");
    }
    if problems.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
