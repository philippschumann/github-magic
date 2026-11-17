fn main() {
    // Listing the command makes it need a permission: only the Settings window's capability grants `allow-settings`.
    let app = tauri_build::AppManifest::new().commands(&["settings"]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(app)).expect("failed to run tauri-build");
}
