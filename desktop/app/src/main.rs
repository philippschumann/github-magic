//! Github Magic: a tray app the browser extension talks to through the bridge (`github-magic-bridge`).

mod server;
mod tray;

use std::io;
use std::sync::Arc;

use github_magic_core::Core;
use github_magic_core::handler::PROTOCOL;
use github_magic_core::platform::Paths;
use serde_json::Value;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

const SETTINGS: &str = "settings";

fn main() {
    let app = tauri::Builder::default()
        // A second launch (the app's icon, `open -a`) shows the settings.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show_settings(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![settings])
        .menu(app_menu)
        .setup(|app| {
            app.set_activation_policy(tauri::ActivationPolicy::Accessory); // tray only, no Dock icon
            register_hosts()?;
            let core = Arc::new(Core::new(Paths::system()));
            app.manage(core.clone());
            server::start(app.handle().clone(), core.clone())?;
            tray::start(app.handle(), core.clone())?;
            let menus = core.clone();
            app.on_menu_event(move |app, event| tray::on_click(app, &menus, event.id().as_ref()));
            open_at_login(app.handle(), core.settings.get().open_at_login);
            let app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut changes = core.settings.subscribe();
                // Lagging behind only means several changes at once: the window reloads everything anyway.
                while !matches!(changes.recv().await, Err(tokio::sync::broadcast::error::RecvError::Closed)) {
                    let _ = app.emit_to(SETTINGS, "settings-changed", ());
                    open_at_login(&app, core.settings.get().open_at_login);
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while running Github Magic");
    app.run(|_, event| {
        // Closing the Settings window leaves the app in the menu bar; it quits from the tray menu or with ⌘Q.
        if let RunEvent::ExitRequested { code: None, api, .. } = event {
            api.prevent_exit();
        }
    });
}

/// The settings page's requests. Only the Settings window may send them (capabilities/settings.json).
#[tauri::command]
async fn settings(app: AppHandle, core: State<'_, Arc<Core>>, mut msg: Value) -> Result<Value, ()> {
    if let Some(msg) = msg.as_object_mut() {
        msg.insert("protocol".into(), PROTOCOL.into()); // the page ships inside the app, so it can't be out of date
    }
    Ok(server::answer(&app, core.inner().clone(), msg).await)
}

pub fn show_settings(app: &AppHandle) {
    let window = match app.get_webview_window(SETTINGS) {
        Some(window) => Ok(window),
        None => WebviewWindowBuilder::new(app, SETTINGS, WebviewUrl::App("settings.html".into()))
            .title("Github Magic Settings")
            .inner_size(680.0, 760.0)
            .build(),
    };
    match window {
        Ok(window) => drop(window.show().and_then(|()| window.set_focus())),
        Err(e) => eprintln!("github-magic: couldn't open the settings: {e}"),
    }
}

/// The menu bar's menu. It only shows while the Settings window is in front, but it is what makes ⌘Q, ⌘W and the
/// editing shortcuts work there. Quit is the app's own item, so it asks first if a run is going.
fn app_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let quit = MenuItem::with_id(app, "quit", "Quit Github Magic", true, Some("CmdOrCtrl+Q"))?;
    let edit: [&dyn tauri::menu::IsMenuItem<tauri::Wry>; 6] = [
        &PredefinedMenuItem::undo(app, None)?,
        &PredefinedMenuItem::redo(app, None)?,
        &PredefinedMenuItem::cut(app, None)?,
        &PredefinedMenuItem::copy(app, None)?,
        &PredefinedMenuItem::paste(app, None)?,
        &PredefinedMenuItem::select_all(app, None)?,
    ];
    Menu::with_items(app, &[
        &Submenu::with_items(app, "Github Magic", true, &[&quit])?,
        &Submenu::with_items(app, "Edit", true, &edit)?,
        &Submenu::with_items(app, "Window", true, &[&PredefinedMenuItem::close_window(app, None)?])?,
    ])
}

/// Makes the login item match the setting.
fn open_at_login(app: &AppHandle, on: bool) {
    if tauri::is_dev() {
        return; // a development build is no login item
    }
    let login_item = app.autolaunch();
    let change = match (login_item.is_enabled().unwrap_or(false), on) {
        (false, true) => login_item.enable(),
        (true, false) => login_item.disable(),
        _ => Ok(()),
    };
    if let Err(e) = change {
        eprintln!("github-magic: couldn't change the login item: {e}");
    }
}

/// Points the browsers' native messaging host at the bridge next to this executable, at every start, so it
/// follows the app wherever it runs from.
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
