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

// The Windows app additionally embeds a *Linux* agent CLI
// (`include_bytes!` in `src-tauri/src/session/wsl/install.rs`) and
// streams it into the WSL distro at launch, so mission agents running
// inside WSL can emit onto the event bus. Cross-build it for
// x86_64-unknown-linux-musl: musl yields a static ELF that runs in any
// distro regardless of glibc, and rust-lld (bundled with rustup) links
// it without needing a Linux toolchain on the Windows host. Always
// release — the bytes ship inside the app binary either way, and a
// debug ELF is ~13× larger for no benefit.
const LINUX_AGENT_TRIPLE = "x86_64-unknown-linux-musl";
const LINUX_AGENT_ARTIFACT = "runner-agent-cli-linux-x86_64";

function stageLinuxAgentCli() {
  const installed = execSync("rustup target list --installed", {
    encoding: "utf8",
  });
  if (!installed.split("\n").some((l) => l.trim() === LINUX_AGENT_TRIPLE)) {
    throw new Error(
      `stage-runner-cli: the Windows build embeds a Linux agent CLI and needs the ${LINUX_AGENT_TRIPLE} target.\n` +
        `Run: rustup target add ${LINUX_AGENT_TRIPLE}`,
    );
  }
  console.log(
    `stage-runner-cli: cross-building Linux agent CLI for ${LINUX_AGENT_TRIPLE} (release)…`,
  );
  const result = spawnSync(
    "cargo",
    [
      "build",
      "-p",
      "runner-cli",
      "--bin",
      "runner-agent-cli",
      "--release",
      "--target",
      LINUX_AGENT_TRIPLE,
    ],
    {
      cwd: repoRoot,
      stdio: "inherit",
      env: {
        CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER: "rust-lld",
        ...process.env,
      },
    },
  );
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
  const source = path.join(
    repoRoot,
    "target",
    LINUX_AGENT_TRIPLE,
    "release",
    "runner-agent-cli",
  );
  const destDir = path.join(repoRoot, "src-tauri", "binaries");
  mkdirSync(destDir, { recursive: true });
  const dest = path.join(destDir, LINUX_AGENT_ARTIFACT);
  copyFileSync(source, dest);
  console.log(`stage-runner-cli: staged ${dest}`);
}

function main() {
  const triple = resolveTargetTriple();
  const profile = resolveProfile();
  buildCli(triple, profile);
  stageArtifacts(triple, profile);
  if (triple.includes("-windows-")) {
    stageLinuxAgentCli();
  }
}

main();
