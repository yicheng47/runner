#!/usr/bin/env node
// Build Runner's CLI sidecars for the active Tauri target triple and
// stage them at `src-tauri/binaries/<name>-<triple>` — the paths
// `tauri.conf.json`'s `bundle.externalBin` entries resolve at bundle
// time. Tauri's bundler then drops them next to the app binary, where
// `cli_install` copies them into `<app_data>/bin/` with product-facing
// names: `runner` for mission agents and `runner-mcp` for MCP clients.
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
const bins = ["runner-agent-cli", "runner-mcp"];

function resolveProfile() {
  const profileArg = process.argv.find((a) => a.startsWith("--profile="));
  if (!profileArg) {
    return "release";
  }
  const profile = profileArg.slice("--profile=".length);
  if (profile !== "debug" && profile !== "release") {
    throw new Error(
      `stage-runner-cli: --profile must be "debug" or "release", got ${profile}`,
    );
  }
  return profile;
}

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

function buildCli(triple, profile) {
  console.log(`stage-runner-cli: building CLI sidecars for ${triple} (${profile})…`);
  const args = ["build", "-p", "runner-cli", "--bins"];
  if (profile === "release") {
    args.push("--release", "--target", triple);
  }
  const result = spawnSync("cargo", args, {
    cwd: repoRoot,
    stdio: "inherit",
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function stageArtifacts(triple, profile) {
  // Workspace `Cargo.toml` sets `target/` at the repo root, so the
  // artifact lives there regardless of which crate triggered the build.
  const ext = process.platform === "win32" ? ".exe" : "";
  const sourceDir =
    profile === "release"
      ? path.join(repoRoot, "target", triple, "release")
      : path.join(repoRoot, "target", "debug");
  const destDir = path.join(repoRoot, "src-tauri", "binaries");
  mkdirSync(destDir, { recursive: true });

  for (const bin of bins) {
    const source = path.join(sourceDir, `${bin}${ext}`);
    if (!existsSync(source)) {
      throw new Error(
        `stage-runner-cli: built artifact missing at ${source}. Did the cargo build silently emit elsewhere?`,
      );
    }
    // Tauri's externalBin convention: `<declared-path>-<triple>[.exe]`.
    const dest = path.join(destDir, `${bin}-${triple}${ext}`);
    copyFileSync(source, dest);
    if (process.platform !== "win32") {
      chmodSync(dest, 0o755);
    }
    console.log(`stage-runner-cli: staged ${dest}`);
  }
}

function main() {
  const triple = resolveTargetTriple();
  const profile = resolveProfile();
  buildCli(triple, profile);
  stageArtifacts(triple, profile);
}

main();
