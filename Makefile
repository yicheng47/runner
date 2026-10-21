.PHONY: build check fmt fmt-check clippy test test-nightly verify run clean cache-size guide-cli guide-cli-check

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
	@if [ "$$(uname -s)" != Linux ]; then cargo clippy -p runner-app --features updater --all-targets -- -D warnings; fi

test:
	cargo test --workspace

test-nightly:
	uv run --no-project --with pyyaml python script/test-nightly.py -v

guide-cli:
	cargo build --locked -p runner-cli --profile ci
	python3 script/generate-cli-reference.py --runner target/ci/runner-agent-cli

guide-cli-check:
	cargo build --locked -p runner-cli --profile ci
	python3 script/generate-cli-reference.py --runner target/ci/runner-agent-cli --check

verify: check test clippy fmt-check

run:
	cargo build -p runner-cli
	@target/debug/runner-agent-cli daemon stop || [ $$? -eq 3 ]
	cargo run -p runner-app

clean:
	cargo clean

cache-size:
	@du -sh target 2>/dev/null || echo "target/: (none)"
	@du -sh ~/.cargo/registry ~/.cargo/git 2>/dev/null
