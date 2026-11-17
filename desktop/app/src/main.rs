//! Github Magic: a tray app the browser extension talks to through the bridge (`github-magic-bridge`).

mod server;

use std::io;
use std::sync::Arc;

use github_magic_core::Core;
use github_magic_core::platform::Paths;
use github_magic_core::settings::Config;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_, _, _| {}))
        .setup(|app| {
            app.set_activation_policy(tauri::ActivationPolicy::Accessory); // tray only, no Dock icon
            register_hosts()?;
            let paths = Paths::system();
            server::start(Arc::new(Core::new(Config::load(&paths), paths)))?;
            let quit = MenuItem::with_id(app, "quit", "Quit Github Magic", true, None::<&str>)?;
            TrayIconBuilder::new()
                .icon(tauri::include_image!("icons/tray.png"))
                .icon_as_template(true)
                .menu(&Menu::with_items(app, &[&quit])?)
                .on_menu_event(|app, event| {
                    if event.id() == "quit" {
                        app.exit(0);
                    }
                })
                .build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Github Magic");
}

/// Points the browsers' native messaging host at the bridge next to this executable, at every start, so it
/// follows the app wherever it runs from. Replaces install.sh.
fn register_hosts() -> io::Result<()> {
    let bridge = std::env::current_exe()?.with_file_name("github-magic-bridge");
    let home = std::env::home_dir().ok_or_else(|| io::Error::other("no home directory"))?;
    for (browser, key, allowed) in [
        // The Chrome ID is fixed by "key" in manifest.json, the Firefox one by browser_specific_settings.gecko.id.
        ("Google/Chrome", "allowed_origins", "chrome-extension://nbiobejjfpehmbkkbddjhplmdeobhcjb/"),
        ("Mozilla", "allowed_extensions", "github-magic@philippschumann"),
    ] {
        let dir = home.join("Library/Application Support").join(browser).join("NativeMessagingHosts");
        std::fs::create_dir_all(&dir)?;
        let manifest = serde_json::json!({
            "name": "github_magic",
            "description": "Open GitHub PRs in an IDE or on a device",
            "path": bridge,
            "type": "stdio",
            key: [allowed],
        });
        std::fs::write(dir.join("github_magic.json"), serde_json::to_string_pretty(&manifest)?)?;
    }
    Ok(())
}
