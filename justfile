# List recipes.
default:
    @just --list

# Install sts into ~/.cargo/bin.
install:
    cargo install --path . --locked

# Run sts from source, for example `just run doctor`.
[positional-arguments]
run *args:
    cargo run --quiet -- "$@"

# Build a release binary.
build:
    cargo build --release --locked

# Run the tests.
test:
    cargo test --locked

# Format the code.
fmt:
    cargo fmt

# Check formatting and clippy, as CI does.
lint:
    cargo fmt --check
    cargo clippy --all-targets --locked -- -D warnings

# Check dependencies against RustSec. Needs cargo-audit.
audit:
    cargo audit

# Scan the working tree for secrets. Needs gitleaks.
secrets:
    gitleaks dir . --redact --no-banner

# Run the CI check job locally.
ci: lint test
