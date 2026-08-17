.PHONY: fmt clippy test run

fmt:
	cargo fmt --all

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

run:
	cargo run -p runner-native
