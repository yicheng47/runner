.PHONY: build check fmt fmt-check clippy test verify run clean cache-size

# Crew builds hit load 208 on 2026-08-22 and starved the UI.
CARGO_BUILD_JOBS ?= 12
export CARGO_BUILD_JOBS

build:
	cargo build --workspace

check:
	cargo check --workspace

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings
	cargo clippy -p runner-app --features updater --all-targets -- -D warnings

test:
	cargo test --workspace

verify: check test clippy fmt-check

run:
	cargo build -p runner-cli
	cargo run -p runner-app

clean:
	cargo clean

cache-size:
	@du -sh target 2>/dev/null || echo "target/: (none)"
	@du -sh ~/.cargo/registry ~/.cargo/git 2>/dev/null
