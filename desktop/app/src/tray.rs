//! The tray menu (one line per target that has had a run, then Update, Settings… and Quit), the notification when
//! a run ends, and the updater behind the Update item.

use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use github_magic_core::Core;
use github_magic_core::runs::{Run, State};
use mac_notification_sys::{MainButton, Notification, NotificationResponse};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Notify;
use tokio::sync::broadcast::error::RecvError;

const TRAY: &str = "tray";
/// A run's line and the menu item that shows it.
type Lines = Vec<(Arc<Run>, Submenu<Wry>)>;

/// A newer release that was found. It is installed only when its menu item is clicked.
#[derive(Default)]
struct Pending {
    update: Mutex<Option<Update>>,
    /// Tells the tray menu to show or drop the item.
    changed: Notify,
}

pub fn start(app: &AppHandle, core: Arc<Core>) -> tauri::Result<()> {
    // Notifications then carry this app's name and icon. An unbundled development build has neither.
    let _ = mac_notification_sys::set_application(&app.config().identifier);
    TrayIconBuilder::with_id(TRAY)
        .icon(tauri::include_image!("icons/tray.png"))
        .icon_as_template(true)
        .menu(&menu(app, &core)?.0)
        .build(app)?;
    app.manage(Pending::default());
    watch_updates(app.clone());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let pending = app.state::<Pending>();
        let mut events = core.runs.subscribe();
        let mut second = tokio::time::interval(Duration::from_secs(1));
        let mut lines = Lines::new();
        loop {
            tokio::select! {
                event = events.recv() => {
                    match &event {
                        Ok(run) => notify(run),
                        Err(RecvError::Closed) => break,
                        Err(RecvError::Lagged(_)) => {}
                    }
                    lines = rebuild(&app, &core).unwrap_or(lines);
                }
                _ = pending.changed.notified() => lines = rebuild(&app, &core).unwrap_or(lines),
                // The elapsed time of the runs still going.
                _ = second.tick() => for (run, line) in &lines {
                    if run.is_running() {
                        let _ = line.set_text(text(run));
                    }
                }
            }
        }
    });
    Ok(())
}

/// Shows the menu as it is now. None: it couldn't be built, so the old one stays.
fn rebuild(app: &AppHandle, core: &Core) -> Option<Lines> {
    match menu(app, core) {
        Ok((menu, lines)) => {
            let _ = app.tray_by_id(TRAY).map(|tray| tray.set_menu(Some(menu)));
            Some(lines)
        }
        Err(e) => {
            eprintln!("github-magic: couldn't build the tray menu: {e}");
            None
        }
    }
}

fn menu(app: &AppHandle, core: &Core) -> tauri::Result<(Menu<Wry>, Lines)> {
    let menu = Menu::new(app)?;
    let mut lines = Lines::new();
    for run in core.runs.latest() {
        let line = Submenu::new(app, text(&run), true)?;
        if run.is_running() {
            line.append(&MenuItem::with_id(app, format!("cancel:{}", run.target), "Cancel", true, None::<&str>)?)?;
        }
        line.append(&MenuItem::with_id(app, format!("log:{}", run.target), "Show log", true, None::<&str>)?)?;
        menu.append(&line)?;
        lines.push((run, line));
    }
    if !lines.is_empty() {
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }
    let update = app.try_state::<Pending>().and_then(|pending| pending.update.lock().unwrap().as_ref().map(|update| update.version.clone()));
    if let Some(version) = update {
        menu.append(&MenuItem::with_id(app, "update", format!("Update to {version}"), true, None::<&str>)?)?;
    }
    menu.append(&MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Quit Github Magic", true, None::<&str>)?)?;
    Ok((menu, lines))
}

/// "● Pixel 10 Pro — #412 feature/login — installing 2:14", or "… — started 14:02" once it has ended.
fn text(run: &Run) -> String {
    let (state, since) = run.state();
    let state = match state {
        State::Running => {
            let seconds = run.started.elapsed().unwrap_or_default().as_secs();
            format!("installing {}:{:02}", seconds / 60, seconds % 60)
        }
        State::Started(_) => format!("started {}", clock(since)),
        State::Failed(_) => format!("failed {}", clock(since)),
        State::Cancelled => format!("cancelled {}", clock(since)),
    };
    format!("{}{} — #{} {} — {state}", if run.is_running() { "● " } else { "" }, run.device(), run.pr, run.branch)
}

/// The local time of day, as "14:02".
fn clock(time: SystemTime) -> String {
    let seconds = time.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as libc::time_t;
    // SAFETY: localtime_r only writes the tm it is given; an all-zero tm is a valid one.
    let tm = unsafe {
        let mut tm = std::mem::zeroed::<libc::tm>();
        libc::localtime_r(&seconds, &mut tm);
        tm
    };
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

/// A click in the tray menu or the menu bar.
pub fn on_click(app: &AppHandle, core: &Arc<Core>, id: &str) {
    let run = |target: &str| core.runs.latest().into_iter().find(|run| run.target == target);
    match id.split_once(':') {
        Some(("cancel", target)) => drop(run(target).map(|run| run.cancel())),
        Some(("log", target)) => drop(run(target).map(|run| show_log(&run))),
        _ if id == "update" => update(app, core),
        _ if id == "settings" => crate::show_settings(app),
        _ if id == "quit" => quit(app, core),
        _ => {}
    }
}

/// Opens the log with the default app for .log files (Console).
fn show_log(run: &Run) {
    if let Err(e) = Command::new("/usr/bin/open").arg(&run.log).status() {
        eprintln!("github-magic: couldn't open {}: {e}", run.log.display());
    }
}

/// One notification per finished run, with a Show log button. Cancelled runs don't notify.
fn notify(run: &Arc<Run>) {
    let text = match run.state().0 {
        State::Started(serial) => format!("Started on {serial}"),
        State::Failed(error) => format!("Failed: {error}"),
        State::Running | State::Cancelled => return,
    };
    let run = run.clone();
    // shortcut: sending waits until the notification is clicked or cleared, so each one keeps an idle thread until
    // then; move to UNUserNotificationCenter if that ever matters.
    std::thread::spawn(move || {
        let title = format!("{} on {}", run.path.file_name().unwrap_or_default().to_string_lossy(), run.target);
        let sent = Notification::new().title(&title).message(&text).main_button(MainButton::SingleAction("Show log")).send();
        match sent {
            Ok(NotificationResponse::ActionButton(_)) => show_log(&run),
            Ok(_) => {}
            Err(e) => eprintln!("github-magic: couldn't notify: {e}"),
        }
    });
}

/// Does `then` once the app may stop: at once, or after the user agreed to cancel the runs still going. Emulators
/// and simulators keep running.
fn before_stopping(app: &AppHandle, core: &Arc<Core>, verb: &'static str, then: impl FnOnce() + Send + 'static) {
    let question = match core.runs.latest().iter().filter(|run| run.is_running()).count() {
        0 => return then(),
        1 => format!("1 run in progress. {verb} and cancel it?"),
        runs => format!("{runs} runs in progress. {verb} and cancel them?"),
    };
    let core = core.clone();
    let buttons = MessageDialogButtons::OkCancelCustom(verb.into(), "Cancel".into());
    app.dialog().message(question).title(format!("{verb} Github Magic")).buttons(buttons).show(move |agreed| {
        if agreed {
            core.runs.cancel_all();
            then();
        }
    });
}

pub fn quit(app: &AppHandle, core: &Arc<Core>) {
    let quitting = app.clone();
    before_stopping(app, core, "Quit", move || quitting.exit(0));
}

/// Looks for a newer release at launch and then every 24 h.
fn watch_updates(app: AppHandle) {
    if tauri::is_dev() {
        return; // a development build isn't replaced by a release
    }
    tauri::async_runtime::spawn(async move {
        loop {
            match app.updater() {
                Ok(updater) => match updater.check().await {
                    Ok(Some(update)) => {
                        let pending = app.state::<Pending>();
                        *pending.update.lock().unwrap() = Some(update);
                        pending.changed.notify_one();
                    }
                    Ok(None) => {}
                    Err(e) => eprintln!("github-magic: couldn't look for updates: {e}"),
                },
                Err(e) => eprintln!("github-magic: couldn't look for updates: {e}"),
            }
            tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        }
    });
}

/// Downloads the found release, checks its signature, replaces the app and restarts it.
fn update(app: &AppHandle, core: &Arc<Core>) {
    let updating = app.clone();
    before_stopping(app, core, "Update", move || {
        tauri::async_runtime::spawn(async move {
            let pending = updating.state::<Pending>();
            let Some(update) = pending.update.lock().unwrap().take() else { return };
            match update.download_and_install(|_, _| {}, || {}).await {
                Ok(()) => updating.restart(),
                Err(e) => {
                    updating.dialog().message(format!("The update to {} failed: {e}", update.version)).title("Github Magic").show(|_| {});
                    *pending.update.lock().unwrap() = Some(update); // still on offer
                }
            }
        });
    });
}
