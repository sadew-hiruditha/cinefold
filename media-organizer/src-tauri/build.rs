fn main() {
    // Windows' resource compiler (rc.exe) mis-parses quoted ICON directives
    // whose path contains a space - it truncates at the first space even
    // though the path is quoted (reproduced directly: `32512 ICON "C:\a b\x.ico"`
    // fails with "RC2135: file not found: C:"). tauri-build hands rc.exe the
    // project's own absolute path, so a workspace folder with a space in it
    // (this one lives under "...\2026 apps\...") silently loses every icon
    // frame but whatever rc.exe manages to salvage - no build error, just a
    // broken single-size icon nobody notices until they look closely.
    //
    // Fix: stage the icon at a space-free path before handing it to Tauri.
    // %TEMP% isn't safe to assume space-free either (it's under the Windows
    // profile, which can itself contain a space for a different username),
    // so this uses a folder at the project's own drive root instead - drive
    // roots are effectively always space-free in practice.
    #[cfg(windows)]
    {
        let source = std::path::Path::new("icons/icon.ico");
        let manifest_dir =
            std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
        let drive_root = manifest_dir
            .ancestors()
            .last()
            .expect("a path always has a root")
            .to_path_buf();

        let staging_dir = drive_root.join(".cinefold-build-icon");
        std::fs::create_dir_all(&staging_dir).expect("create icon staging dir");
        let staged_icon = staging_dir.join("icon.ico");
        std::fs::copy(source, &staged_icon).expect("stage icon.ico to a space-free path");

        assert!(
            !staged_icon.to_string_lossy().contains(' '),
            "icon staging path still contains a space, rc.exe will mis-embed it: {}",
            staged_icon.display()
        );

        println!("cargo:rerun-if-changed={}", source.display());

        let attrs = tauri_build::Attributes::new().windows_attributes(
            tauri_build::WindowsAttributes::new().window_icon_path(&staged_icon),
        );
        tauri_build::try_build(attrs).expect("tauri_build failed");
    }

    #[cfg(not(windows))]
    tauri_build::build();
}
