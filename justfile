default: check

fmt:
    cargo fmt --all

check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    ./scripts/check-docs.sh

test:
    cargo test --workspace

deny:
    cargo deny check

# Run benchmark scenarios; results in target/bench.json (ADR-0022).
bench:
    cargo run -q --release -p agora-bench -- run --out target/bench.json
    cat target/bench.json
