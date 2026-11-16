// Append to macos.rs in a disposable macOS worktree, then run:
// cargo test -p agora-platform --lib review_592_sibling -- --nocapture
#[cfg(test)]
mod review_592_sibling {
    #[test]
    fn atomic_save_sibling_cannot_write_an_unlisted_file() {
        let root = std::env::temp_dir().join(format!("agora-review-592-{}", std::process::id()));
        assert!(!root.exists());
        std::fs::create_dir_all(root.join("private")).unwrap();
        let config = root.join("state.json");
        let outside = root.join("private/zshrc");
        let sibling = root.join("state.json.tmp");
        std::fs::write(&config, "{}").unwrap();
        std::fs::write(&outside, "keep\n").unwrap();
        let sb = crate::Sandbox {
            write: vec![config, "/dev".into()],
            ..Default::default()
        };
        assert!(super::profile(&sb).is_ok());
        std::fs::hard_link(&outside, &sibling).unwrap();
        let denied = match super::profile(&sb) {
            Err(e) => {
                assert_eq!(
                    crate::sandbox_refusal(&e),
                    Some(crate::SandboxRefusal::WritableHasLinks)
                );
                true
            }
            Ok(policy) => {
                let out = std::process::Command::new(super::SANDBOX_EXEC)
                    .args([
                        "-p",
                        &policy,
                        "/bin/sh",
                        "-c",
                        "printf REVIEW-WRITE >> \"$AGORA_REVIEW_SIBLING\"",
                    ])
                    .env("AGORA_REVIEW_SIBLING", &sibling)
                    .output()
                    .unwrap();
                let unchanged = std::fs::read_to_string(&outside).unwrap() == "keep\n";
                println!(
                    "native_exit={:?} outside_unchanged={unchanged}",
                    out.status.code()
                );
                unchanged
            }
        };
        std::fs::remove_dir_all(root).unwrap();
        assert!(
            denied,
            "atomic-save sibling hard link changed an unlisted file"
        );
    }
}
