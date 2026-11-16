// Append to winconfine.rs in a throwaway Windows worktree, then run:
// cargo test -p agora-platform --lib review_591_regular_file -- --nocapture
#[cfg(test)]
mod review_591_regular_file {
    use super::*;

    #[test]
    fn package_directory_names_do_not_exempt_regular_files() {
        let root = std::env::temp_dir().join(format!("agora-review-591-{}", std::process::id()));
        assert!(!root.exists());
        std::fs::create_dir_all(&root).unwrap();
        let outside = root.join("outside.txt");
        std::fs::write(&outside, "keep").unwrap();
        let results: Vec<_> = LINKS_NOT_CHECKED
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let work = root.join(format!("work-{i}"));
                std::fs::create_dir_all(&work).unwrap();
                let link = work.join(name);
                std::fs::hard_link(&outside, &link).unwrap();
                assert!(link.is_file());
                assert!(has_other_names(&link).unwrap());
                let got = broad_writer_below(&work, true).unwrap();
                (link, got)
            })
            .collect();
        std::fs::remove_dir_all(&root).unwrap();
        for (link, got) in results {
            assert_eq!(got, Some((link, SandboxRefusal::WritableHasLinks)));
        }
    }
}
