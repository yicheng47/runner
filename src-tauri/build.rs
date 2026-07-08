fn main() {
    // The Windows build embeds the Linux agent CLI (`include_bytes!` in
    // `session/wsl/install.rs`). Check up front so a missing stage fails
    // with the fix spelled out instead of a bare include_bytes! error.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        if !std::path::Path::new("binaries/runner-agent-cli-linux-x86_64").exists() {
            panic!(
                "missing src-tauri/binaries/runner-agent-cli-linux-x86_64 — the Linux agent CLI \
                 embedded into the Windows build. Stage it with: node scripts/stage-runner-cli.mjs \
                 (needs `rustup target add x86_64-unknown-linux-musl`)"
            );
        }
        // Embed the app manifest via the linker for *every* executable —
        // the app and cargo's test binaries alike (see
        // windows-app.manifest for the full story). tauri_build's own
        // resource-based embedding covers bin targets only, leaving the
        // lib-test exe to die at load with STATUS_ENTRYPOINT_NOT_FOUND;
        // it is disabled below in favor of this.
        let manifest = std::env::current_dir()
            .unwrap()
            .join("windows-app.manifest");
        println!("cargo:rerun-if-changed=windows-app.manifest");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        tauri_build::try_build(
            tauri_build::Attributes::new()
                .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
        )
        .expect("failed to run tauri-build");
        return;
    }
    tauri_build::build()
}
