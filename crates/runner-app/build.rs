fn main() {
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=../../assets/icon.ico");
        println!("cargo:rerun-if-changed=resources/windows/runner.manifest.xml");
        winresource::WindowsResource::new()
            .set_icon("../../assets/icon.ico")
            .set_manifest_file("resources/windows/runner.manifest.xml")
            .compile()
            .expect("failed to embed Runner's Windows resources");
    }
}
