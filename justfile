default:
    @just --list

build:
    cargo build --all-targets
    cargo build --all-targets --all-features

check: fmt clippy test msrv doc examples

fmt:
    cargo fmt --all -- --check

clippy:
    cargo clippy --all-targets -- -D warnings
    cargo clippy --no-default-features --all-targets -- -D warnings
    cargo clippy --all-features --all-targets -- -D warnings

test:
    cargo test --all-features
    cargo test --no-default-features

msrv:
    cargo +1.89 build --all-targets --all-features

doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features

examples:
    cargo build --examples --all-features
