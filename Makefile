.PHONY: fmt clippy test run-native

fmt:
	cargo fmt --all

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

run-native:
	cargo run -p runner-native --release
