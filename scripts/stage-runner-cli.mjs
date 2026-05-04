#!/usr/bin/env node
// Build `runner-cli` for the active Tauri target triple and stage the
// artifact at `src-tauri/binaries/runner-cli-<triple>` — the path
// `tauri.conf.json`'s `bundle.externalBin: ["binaries/runner-cli"]`
// resolves at bundle time. Tauri's bundler then drops it into
// `Runner.app/Contents/MacOS/runner-cli` (renamed without the suffix),
// where `cli_install::install_runner_cli` finds it next to
// `current_exe` on first launch and copies it into
// `<app_data>/bin/runner` for child PTYs to spawn.
//
// Why a build-time stage instead of a runtime fetch:
//   - Single bundle ships everything; no first-launch download.
//   - The CLI version always matches the app version on disk.
//   - Works offline, works in CI for tagged releases.
//
// Triple resolution:
//   1. `TAURI_ENV_TARGET_TRIPLE` — set by Tauri 2 when running this as a
//      `beforeBuildCommand` (this is the canonical source during a
//      `tauri build --target <triple>` invocation).
//   2. `--target=<triple>` CLI arg — manual override for local
//      cross-compile testing.
//   3. `rustc -vV` "host:" line — fallback for plain `tauri build`
//      with no target specified (host build).

import { execSync, spawnSync } from "node:child_process";
import { copyFileSync, chmodSync, mkdirSync, existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");

function resolveTargetTriple() {
  if (process.env.TAURI_ENV_TARGET_TRIPLE) {
    return process.env.TAURI_ENV_TARGET_TRIPLE;
  }
  const cliArg = process.argv.find((a) => a.startsWith("--target="));
  if (cliArg) {
    return cliArg.slice("--target=".length);
  }
  // `rustc -vV` prints multiple lines; the `host:` line is the active
  // toolchain's default triple. Trim and split robustly.
  const out = execSync("rustc -vV", { encoding: "utf8" });
  const hostLine = out.split("\n").find((l) => l.startsWith("host:"));
  if (!hostLine) {
    throw new Error(
      `stage-runner-cli: rustc -vV output had no "host:" line:\n${out}`,
    );
  }
  return hostLine.slice("host:".length).trim();
}

function buildCli(triple) {
  console.log(`stage-runner-cli: building runner-cli for ${triple}…`);
  const args = [
    "build",
    "-p",
    "runner-cli",
    "--release",
    "--target",
    triple,
  ];
  const result = spawnSync("cargo", args, {
    cwd: repoRoot,
    stdio: "inherit",
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function stageArtifact(triple) {
  // Workspace `Cargo.toml` sets `target/` at the repo root, so the
  // artifact lives there regardless of which crate triggered the build.
  const ext = process.platform === "win32" ? ".exe" : "";
  const source = path.join(
    repoRoot,
    "target",
    triple,
    "release",
    `runner-cli${ext}`,
  );
  if (!existsSync(source)) {
    throw new Error(
      `stage-runner-cli: built artifact missing at ${source}. Did the cargo build silently emit elsewhere?`,
    );
  }
  // Tauri's externalBin convention: `<declared-path>-<triple>[.exe]`.
  // Declared path is `binaries/runner-cli` (relative to `src-tauri/`).
  const destDir = path.join(repoRoot, "src-tauri", "binaries");
  mkdirSync(destDir, { recursive: true });
  const dest = path.join(destDir, `runner-cli-${triple}${ext}`);
  copyFileSync(source, dest);
  if (process.platform !== "win32") {
    chmodSync(dest, 0o755);
  }
  console.log(`stage-runner-cli: staged ${dest}`);
}

function main() {
  const triple = resolveTargetTriple();
  buildCli(triple);
  stageArtifact(triple);
}

main();
