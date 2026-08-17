.PHONY: build check fmt fmt-check clippy test verify run

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

test:
	cargo test --workspace

verify: check test clippy fmt-check

run:
	cargo run -p runner-native
