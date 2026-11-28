//! Build `vendor/libghostty-vt` statically with Zig 0.17.0 and generate C ABI bindings.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_MACOS_DEPLOYMENT_TARGET: &str = "12.0";

fn macos_deployment_target() -> String {
    let raw = env::var("MACOSX_DEPLOYMENT_TARGET")
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_MACOS_DEPLOYMENT_TARGET.to_owned());
    if raw.contains('.') {
        raw
    } else {
        format!("{raw}.0")
    }
}

fn zig_target(target: &str) -> String {
    match target {
        "aarch64-apple-darwin" => format!("aarch64-macos.{}", macos_deployment_target()),
        "x86_64-apple-darwin" => format!("x86_64-macos.{}", macos_deployment_target()),
        "x86_64-pc-windows-msvc" => "x86_64-windows-msvc".to_owned(),
        "aarch64-pc-windows-msvc" => "aarch64-windows-msvc".to_owned(),
        "x86_64-unknown-linux-gnu" => "x86_64-linux-gnu".to_owned(),
        "aarch64-unknown-linux-gnu" => "aarch64-linux-gnu".to_owned(),
        other => panic!("ghostty-vt-sys: unsupported Rust target for libghostty-vt: {other}"),
    }
}

fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    let raw = path.to_string_lossy();
    match raw.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => path,
    }
}

fn parse_json_string_field(content: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let (_, after_key) = content.split_once(&needle)?;
    let (_, after_colon) = after_key.split_once(':')?;
    let trimmed = after_colon.trim_start();
    let rest = trimmed.strip_prefix('"')?;
    let (value, _) = rest.split_once('"')?;
    Some(value.to_owned())
}

fn parse_zon_version(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(".version") {
            let (_, after_eq) = rest.split_once('=')?;
            let after_quote = after_eq.trim_start().strip_prefix('"')?;
            let (value, _) = after_quote.split_once('"')?;
            return Some(value.to_owned());
        }
    }
    None
}

fn default_system_deps_dir(version: &str, deps_sha256: &str) -> Option<PathBuf> {
    let sha_prefix = deps_sha256.get(..8)?;
    let slug = format!("{version}-{sha_prefix}");
    let base = if cfg!(windows) {
        env::var_os("LOCALAPPDATA").map(PathBuf::from).or_else(|| {
            env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("AppData/Local"))
        })?
    } else {
        env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?
    };
    Some(base.join("runner").join("libghostty-vt-deps").join(slug))
}

fn resolve_system_deps_dir(vendor_meta_path: &Path) -> Option<PathBuf> {
    if let Ok(explicit) = env::var("LIBGHOSTTY_VT_ZIG_SYSTEM_DIR") {
        if !explicit.trim().is_empty() {
            let path = PathBuf::from(explicit);
            assert!(
                path.is_dir(),
                "LIBGHOSTTY_VT_ZIG_SYSTEM_DIR does not point to an existing directory: {}",
                path.display()
            );
            return Some(path);
        }
    }

    let meta = fs::read_to_string(vendor_meta_path).unwrap_or_else(|e| {
        panic!("failed to read {}: {e}", vendor_meta_path.display());
    });
    let version =
        parse_json_string_field(&meta, "version").expect("vendor metadata missing version");
    let deps_sha256 =
        parse_json_string_field(&meta, "deps_sha256").expect("vendor metadata missing deps_sha256");

    let candidate = default_system_deps_dir(&version, &deps_sha256)?;
    if candidate.join(".complete").is_file() {
        return Some(candidate);
    }

    println!(
        "cargo:warning=libghostty-vt offline Zig deps cache missing at {}; run `python3 script/fetch-libghostty-vt-deps.py` (or `make ghostty-deps`) for offline builds",
        candidate.display()
    );
    None
}

fn zig_lib_dir(zig: &str) -> String {
    let output = Command::new(zig)
        .arg("env")
        .output()
        .unwrap_or_else(|e| panic!("failed to run `{zig} env`: {e}"));
    assert!(
        output.status.success(),
        "`{zig} env` failed with status {}",
        output.status
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix(".lib_dir = \"")
                .and_then(|rest| rest.strip_suffix("\","))
        })
        .expect("`zig env` output did not contain `.lib_dir`")
        .to_owned()
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let vendor_dir = manifest_dir.join("../../vendor");
    let vendor_meta = vendor_dir.join("libghostty-vt.vendor.json");
    let src_dir = vendor_dir
        .join("libghostty-vt")
        .canonicalize()
        .expect("vendor/libghostty-vt missing");
    let src_dir = strip_windows_verbatim_prefix(src_dir);
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", vendor_meta.display());
    for rel in [
        "build.zig",
        "build.zig.zon",
        "LIB_VERSION",
        "include",
        "pkg",
        "src",
    ] {
        println!("cargo:rerun-if-changed={}", src_dir.join(rel).display());
    }
    for var in [
        "ZIG",
        "MACOSX_DEPLOYMENT_TARGET",
        "LIBGHOSTTY_VT_ZIG_SYSTEM_DIR",
        "LIBGHOSTTY_VT_ZIG_GLOBAL_CACHE_DIR",
        "LIBGHOSTTY_VT_WINDOWS_LIBC",
        "LIBCLANG_PATH",
        "XDG_CACHE_HOME",
        "HOME",
        "LOCALAPPDATA",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }

    let target = env::var("TARGET").expect("TARGET");
    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS");
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let zig = env::var("ZIG").unwrap_or_else(|_| "zig".to_owned());

    let app_zon = fs::read_to_string(src_dir.join("build.zig.zon"))
        .expect("failed to read vendor/libghostty-vt/build.zig.zon");
    let app_version =
        parse_zon_version(&app_zon).expect("vendor/libghostty-vt/build.zig.zon missing .version");

    let zig_prefix = out_dir.join("zig-out");
    let shared_profile_cache = out_dir
        .ancestors()
        .nth(2)
        .filter(|dir| dir.file_name().and_then(|n| n.to_str()) == Some("build"))
        .and_then(Path::parent)
        .map(|profile_dir| profile_dir.join("ghostty-vt-zig-cache"));
    let zig_cache = shared_profile_cache
        .as_ref()
        .map(|base| base.join("local"))
        .unwrap_or_else(|| out_dir.join("zig-cache"));
    let zig_global_cache = match env::var("LIBGHOSTTY_VT_ZIG_GLOBAL_CACHE_DIR") {
        Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        // Zig 0.17 crashes if `--system` shares a global cache holding `.tar.gz` archives.
        // Keeping the cache under this worktree's target/<profile> stays archive-free,
        // avoids cross-worktree races, and is reused across build/clippy/test units.
        _ => shared_profile_cache
            .as_ref()
            .map(|base| base.join("global"))
            .unwrap_or_else(|| out_dir.join("zig-global-cache")),
    };
    fs::create_dir_all(&zig_cache).expect("failed to create Zig local cache dir");
    fs::create_dir_all(&zig_global_cache).expect("failed to create Zig global cache dir");

    let mut cmd = Command::new(&zig);
    cmd.current_dir(&src_dir)
        .env("ZIG_GLOBAL_CACHE_DIR", &zig_global_cache)
        .args([
            "build",
            "-Demit-lib-vt",
            "-Demit-xcframework=false",
            "-Doptimize=ReleaseFast",
        ])
        .arg(format!("-Dtarget={}", zig_target(&target)))
        // Skip Ghostty's `git describe` probe, which would otherwise inspect Runner's own git repo.
        .arg(format!("-Dversion-string={app_version}"))
        .arg("--prefix")
        .arg(&zig_prefix)
        .arg("--cache-dir")
        .arg(&zig_cache);

    if let Some(system_dir) = resolve_system_deps_dir(&vendor_meta) {
        cmd.arg("--system").arg(system_dir);
    }
    if target.ends_with("windows-msvc") {
        if let Some(libc_file) = env::var_os("LIBGHOSTTY_VT_WINDOWS_LIBC") {
            let libc_path = PathBuf::from(&libc_file);
            println!("cargo:rerun-if-changed={}", libc_path.display());
            cmd.arg("--libc").arg(libc_path);
        }
    }

    let status = cmd.status().unwrap_or_else(|err| {
        panic!(
            "failed to run Zig ({zig:?}) to build libghostty-vt (Zig 0.17.0 required; set ZIG to override): {err}"
        );
    });
    assert!(
        status.success(),
        "zig build of libghostty-vt failed with status {status} (Zig 0.17.0 required)"
    );

    // Expose only the static archive in the search directory so linkers never pick a sibling shared library.
    let static_dir = out_dir.join("lib");
    fs::create_dir_all(&static_dir).expect("failed to create static library link dir");
    if target_env == "msvc" {
        fs::copy(
            zig_prefix.join("lib/ghostty-vt-static.lib"),
            static_dir.join("ghostty-vt.lib"),
        )
        .expect("failed to copy ghostty-vt-static.lib");
    } else {
        fs::copy(
            zig_prefix.join("lib/libghostty-vt.a"),
            static_dir.join("libghostty-vt.a"),
        )
        .expect("failed to copy libghostty-vt.a");
    }
    println!("cargo:rustc-link-search=native={}", static_dir.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt");
    if target_os == "windows" {
        println!("cargo:rustc-link-lib=ntdll");
        println!("cargo:rustc-link-lib=kernel32");
    }

    let include_dir = src_dir.join("include");
    let mut builder = bindgen::Builder::default()
        .header(include_dir.join("ghostty/vt.h").to_string_lossy())
        .clang_arg(format!("-I{}", include_dir.display()))
        // Match static linking so Windows headers do not mark functions `__declspec(dllimport)`.
        .clang_arg("-DGHOSTTY_STATIC");

    if target_os == "macos" {
        if let Ok(sdk) = Command::new("xcrun").args(["--show-sdk-path"]).output() {
            if sdk.status.success() {
                let sdk_path = String::from_utf8_lossy(&sdk.stdout).trim().to_owned();
                if !sdk_path.is_empty() {
                    builder = builder.clang_arg(format!("-isysroot{sdk_path}"));
                }
            }
        }
    } else if Command::new("clang").arg("--version").output().is_err() {
        let zig_lib = zig_lib_dir(&zig);
        builder = builder.clang_arg(format!("-isystem{zig_lib}/include"));
    }

    builder
        .allowlist_function("ghostty_.*")
        .allowlist_type("Ghostty.*")
        .allowlist_var("GHOSTTY_.*")
        .prepend_enum_name(false)
        .derive_default(true)
        .layout_tests(false)
        .generate()
        .expect("bindgen failed on ghostty/vt.h")
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("failed to write generated bindings.rs");
}
