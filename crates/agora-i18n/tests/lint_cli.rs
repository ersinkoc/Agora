// i18n-lint: ignore-file
#![allow(clippy::expect_used)] // expect is allowed in tests
use std::fs;
use std::process::Command;

fn lint(root: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_i18n-lint"))
        .arg(root)
        .output()
        .expect("run lint")
}

fn fixture(name: &str, rust: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("agora-i18n-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("locales/en")).expect("mkdir");
    fs::create_dir_all(root.join("crates/x/src")).expect("mkdir");
    fs::write(root.join("locales/en/main.ftl"), "hello = Hi\n").expect("write");
    fs::write(root.join("crates/x/src/lib.rs"), rust).expect("write");
    root
}

#[test]
fn passes_when_keys_exist() {
    let root = fixture("ok", r#"const A: &str = key!("hello");"#);
    assert!(lint(&root).status.success());
}

#[test]
fn fails_on_missing_key() {
    let root = fixture("bad", r#"const A: &str = key!("missing-key");"#);
    let out = lint(&root);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("missing-key"));
}
