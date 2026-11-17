//! The socket the bridge connects to: one request frame and one reply frame per connection, each connection in
//! its own task so a slow `git fetch` never blocks status polls.

use std::fs::{self, Permissions};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::process::{Output, Stdio};

use std::sync::Arc;

use github_magic_core::{Core, MAX_FRAME, handler, socket_path};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::Command;

/// shortcut: actions not ported yet go to the Python script of this checkout; gone at the end of phase 2.
const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../native-host/open_pr_in_studio.py");

pub fn start(core: Arc<Core>) -> io::Result<()> {
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
                    let core = core.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = handle(conn, core).await {
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

async fn handle(mut conn: UnixStream, core: Arc<Core>) -> io::Result<()> {
    let mut frame = vec![0; 4];
    conn.read_exact(&mut frame).await?;
    let len = u32::from_ne_bytes(frame[..4].try_into().unwrap()) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::other(format!("request of {len} bytes is over the 1 MB limit")));
    }
    frame.resize(4 + len, 0);
    conn.read_exact(&mut frame[4..]).await?;
    let reply = match serde_json::from_slice::<Value>(&frame[4..]) {
        // On a blocking thread: the tools it runs (git fetch, …) take a while.
        Ok(msg) if handler::ported(&msg) => {
            let reply = tauri::async_runtime::spawn_blocking(move || handler::handle(&core, &msg)).await;
            reply_frame(&reply.unwrap_or_else(|e| serde_json::json!({ "ok": false, "error": e.to_string() })))
        }
        _ => python_reply(&frame).await,
    };
    conn.write_all(&reply).await
}

async fn python_reply(frame: &[u8]) -> Vec<u8> {
    match python(frame).await {
        Ok(out) if !out.stdout.is_empty() => out.stdout, // already a reply frame
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            eprintln!("github-magic: python3 replied nothing: {stderr}");
            error_frame(stderr.trim().lines().last().unwrap_or("python3 exited without a reply"))
        }
        Err(e) => error_frame(&format!("couldn't run python3: {e}")),
    }
}

/// Runs the script on the request frame unchanged; its stdout is the reply frame.
async fn python(frame: &[u8]) -> io::Result<Output> {
    let mut child = Command::new("/usr/bin/python3")
        .arg(SCRIPT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().expect("piped").write_all(frame).await?;
    child.wait_with_output().await
}

fn error_frame(error: &str) -> Vec<u8> {
    reply_frame(&serde_json::json!({ "ok": false, "error": error }))
}

fn reply_frame(reply: &Value) -> Vec<u8> {
    let body = reply.to_string();
    [&(body.len() as u32).to_ne_bytes()[..], body.as_bytes()].concat()
}
