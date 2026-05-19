.PHONY: dev dev-web build package install lint lint-frontend lint-rust typecheck test test-rust test-ts check fmt fmt-check ci clean clean-all

# Start Tauri app (frontend + Rust backend) in dev mode
dev:
	pnpm tauri dev

# Start frontend only (browser — Rust commands unavailable)
dev-web:
	pnpm dev

# Build production app
build:
	pnpm tauri build

# Package macOS app (.app + .dmg)
package:
	pnpm tauri build --bundles app,dmg
	@echo "\nPackaged:"
	@ls -lh src-tauri/target/release/bundle/dmg/*.dmg 2>/dev/null || true
	@ls -lh src-tauri/target/release/bundle/macos/*.app 2>/dev/null || true

# Install JS deps
install:
	pnpm install

# Lint everything (frontend + backend) — matches CI's lint gates so a
# clean local run means CI won't fail on lint. Stops on the first
# failing target so the error is the last thing in the scrollback.
lint: lint-frontend lint-rust

# Frontend: typecheck + eslint. Both gates from the `frontend` CI job.
lint-frontend:
	pnpm exec tsc --noEmit
	pnpm run lint

# Backend: cargo fmt --check + clippy -D warnings. Both gates from the
# `backend` CI job (cargo check + cargo test are separate, run via
# `make check` / `make test-rust` or `make ci`).
lint-rust:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings

# TS typecheck (no emit) — kept as a separate target for IDE-style
# loops; `make lint` runs this transitively via lint-frontend.
typecheck:
	pnpm exec tsc --noEmit

# Rust unit + integration tests (whole workspace — app crate + runners-core)
test-rust:
	cargo test --workspace

# Frontend typecheck (v0 has no JS tests yet)
test-ts: typecheck

# Everything
test: test-rust test-ts

# Rust compile-only
check:
	cargo check --workspace

# Rust format (rewrites files)
fmt:
	cargo fmt --all

# Rust format check (CI parity — fails on any formatting drift)
fmt-check:
	cargo fmt --all --check

# Full pre-push battery: every gate the `frontend` + `backend` CI jobs
# run, in the same order. Heavier than `make lint` — includes
# cargo check + the full workspace test run.
ci: lint check test-rust

# Clean dev artifacts
clean:
	rm -rf dist node_modules/.vite target/debug src-tauri/target/debug

# Clean everything including release builds
clean-all: clean
	rm -rf target/release src-tauri/target/release
