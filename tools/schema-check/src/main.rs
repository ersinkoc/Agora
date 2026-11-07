//! `schema-check BASE_SCHEMA_DIR HEAD_SCHEMA_DIR`: exit 1 if HEAD breaks anything BASE published.
//! A base directory without any schema (first introduction) passes.

use std::path::Path;
use std::process::ExitCode;

use agora_schema_check::{breaking_changes, load};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [base, head] = &args[..] else {
        eprintln!("usage: schema-check BASE_SCHEMA_DIR HEAD_SCHEMA_DIR");
        return ExitCode::from(2);
    };
    let (base, head) = match (load(Path::new(base)), load(Path::new(head))) {
        (Ok(b), Ok(h)) => (b, h),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("schema-check: {e}");
            return ExitCode::from(2);
        }
    };
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
