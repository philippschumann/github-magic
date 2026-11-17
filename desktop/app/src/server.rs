//! The socket the bridge connects to: one request frame and one reply frame per connection, each connection in
//! its own task so a slow `git fetch` never blocks status polls.

use std::fs::{self, Permissions};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;

use github_magic_core::handler::{self, PROTOCOL};
use github_magic_core::{Core, MAX_FRAME, settings, socket_path};
use serde_json::{Value, json};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

pub fn start(app: AppHandle, core: Arc<Core>) -> io::Result<()> {
    let path = socket_path();
    let _ = fs::remove_file(&path); // stale socket left by a crash
    let listener = std::os::unix::net::UnixListener::bind(&path)?;
    // The temp dir is already private to the user; 0600 keeps the socket private if it ever moves.
    fs::set_permissions(&path, Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    tauri::async_runtime::spawn(async move {
        let listener = UnixListener::from_std(listener).expect("inside the tokio runtime");
        loop {
            match listener.accept().await {
                Ok((conn, _)) => {
                    let (app, core) = (app.clone(), core.clone());
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = handle(conn, app, core).await {
                            eprintln!("github-magic: connection failed: {e}");
                        }
                    });
                }
                Err(e) => eprintln!("github-magic: accept failed: {e}"),
            }
        }
    });
    Ok(())
}

async fn handle(mut conn: UnixStream, app: AppHandle, core: Arc<Core>) -> io::Result<()> {
    let mut len = [0; 4];
    conn.read_exact(&mut len).await?;
    let len = u32::from_ne_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::other(format!("request of {len} bytes is over the 1 MB limit")));
    }
    let mut request = vec![0; len];
    conn.read_exact(&mut request).await?;
    let reply = match serde_json::from_slice::<Value>(&request) {
        Ok(msg) => answer(&app, core, msg).await,
        Err(e) => json!({ "ok": false, "error": format!("invalid request: {e}") }),
    };
    let reply = reply.to_string();
    conn.write_all(&(reply.len() as u32).to_ne_bytes()).await?;
    conn.write_all(reply.as_bytes()).await
}

/// Answers one request, from the extension or the Settings window. The folder dialogs are the app's own; everything
/// else is the core's.
pub async fn answer(app: &AppHandle, core: Arc<Core>, msg: Value) -> Value {
    let dialog = if msg["protocol"] == PROTOCOL { msg["action"].as_str() } else { None };
    match dialog {
        Some("choose_folder") => {
            // A browser's own file picker never reveals the absolute path.
            let current = msg["current"].as_str().map(|dir| core.paths.expand_user(dir)).filter(|dir| dir.is_dir());
            match pick_folder(app, "Worktree folder".into(), current).await {
                Some(path) => json!({ "ok": true, "path": path }),
                None => json!({ "ok": false, "cancelled": true }),
            }
        }
        Some("add_repo") => {
            // A repo's folder comes only from this dialog, never from a request.
            let expected = msg["repo"].as_str().map(str::to_owned);
            let title = format!("Main clone of {}", expected.as_deref().unwrap_or("the repo"));
            let Some(clone) = pick_folder(app, title, None).await else { return json!({ "ok": false, "cancelled": true }) };
            match blocking(move || settings::add_repo(&core, &clone, expected.as_deref())).await {
                Ok(repo) => json!({ "ok": true, "repo": repo }),
                Err(error) => json!({ "ok": false, "error": error }),
            }
        }
        _ => blocking(move || Ok(handler::handle(&core, &msg))).await.unwrap_or_else(|error| json!({ "ok": false, "error": error })),
    }
}

/// On a blocking thread: the tools the core runs (git fetch, …) take a while.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| e.to_string())?
}

/// The system's folder dialog. None: cancelled.
async fn pick_folder(app: &AppHandle, title: String, start: Option<PathBuf>) -> Option<PathBuf> {
    let mut dialog = app.dialog().file().set_title(title);
    if let Some(start) = start {
        dialog = dialog.set_directory(start);
    }
    let (chosen, choice) = tokio::sync::oneshot::channel();
    dialog.pick_folder(move |folder| drop(chosen.send(folder)));
    choice.await.ok().flatten().and_then(|folder| folder.into_path().ok())
}
