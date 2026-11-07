default: check

fmt:
    cargo fmt --all

check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    cargo run -q -p agora-i18n --bin i18n-lint
    sh scripts/check-docs.sh

test:
    cargo test --workspace

deny:
    cargo deny check
