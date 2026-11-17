//! The socket the bridge connects to: one request frame and one reply frame per connection, each connection in
//! its own task so a slow `git fetch` never blocks status polls.

use std::fs::{self, Permissions};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use github_magic_core::{Core, MAX_FRAME, handler, socket_path};
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
        Ok(msg) if msg["action"] == "choose_folder" => choose_folder(&app, &core, &msg["current"]).await,
        // On a blocking thread: the tools it runs (git fetch, …) take a while.
        Ok(msg) => tauri::async_runtime::spawn_blocking(move || handler::handle(&core, &msg))
            .await
            .unwrap_or_else(|e| json!({ "ok": false, "error": e.to_string() })),
        Err(e) => json!({ "ok": false, "error": format!("invalid request: {e}") }),
    };
    let reply = reply.to_string();
    conn.write_all(&(reply.len() as u32).to_ne_bytes()).await?;
    conn.write_all(reply.as_bytes()).await
}

/// The native folder dialog: a browser's own file picker never reveals the absolute path.
async fn choose_folder(app: &AppHandle, core: &Core, current: &Value) -> Value {
    let mut dialog = app.dialog().file().set_title("Worktree folder");
    if let Some(dir) = current.as_str().map(|dir| core.paths.expand_user(dir)).filter(|dir| dir.is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    let (chosen, choice) = tokio::sync::oneshot::channel();
    dialog.pick_folder(move |folder| drop(chosen.send(folder)));
    match choice.await.ok().flatten().and_then(|folder| folder.into_path().ok()) {
        Some(path) => json!({ "ok": true, "path": path }),
        None => json!({ "ok": false, "cancelled": true }),
    }
}
