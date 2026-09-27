default:
    @just --list

check:
    cargo check --workspace

fmt:
    cargo fmt --all

lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    cargo test --workspace

# Fast application tests, without Telegram's networking stack.
offline:
    cargo test -p gramhive-test

verify: lint test
    nix develop -c cargo check --workspace

echo:
    cargo run -p gramhive-echo
