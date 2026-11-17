//! The Claude button: a Claude desktop session in the PR's worktree.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::cmd;
use crate::platform::Paths;

/// Reopens the newest unarchived Claude desktop session in this folder, else a new session's composer there.
pub fn open_claude(paths: &Paths, path: &Path, prompt: &str) -> Result<(), String> {
    let mut sessions = vec![];
    find_sessions(&paths.sessions_dir, path, &mut sessions);
    let url = match sessions.into_iter().max_by(|a, b| a.0.total_cmp(&b.0)) {
        Some((_, session)) => format!("claude://code/continue?session={}", quote(&session)),
        None => format!("claude://code/new?folder={}&q={}", quote(&path.to_string_lossy()), quote(prompt)),
    };
    cmd::run(cmd::command(paths, "open").arg(url)).map(drop)
}

/// Collects (last activity, session ID) of the unarchived sessions in `cwd` from the `local_*.json` files under `dir`.
fn find_sessions(dir: &Path, cwd: &Path, found: &mut Vec<(f64, String)>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let (path, name) = (entry.path(), entry.file_name().to_string_lossy().into_owned());
        if path.is_dir() {
            find_sessions(&path, cwd, found);
        } else if name.starts_with("local_") && name.ends_with(".json") {
            let session = fs::read(&path).ok().and_then(|json| serde_json::from_slice::<Value>(&json).ok()).unwrap_or_default();
            let id = session["sessionId"].as_str().filter(|id| !id.is_empty());
            if let Some(id) = id.filter(|_| session["cwd"].as_str().map(Path::new) == Some(cwd) && session["isArchived"] != true) {
                found.push((session["lastActivityAt"].as_f64().unwrap_or(0.0), id.to_owned()));
            }
        }
    }
}

/// Percent-encodes everything but letters, digits and `_.-~`.
fn quote(text: &str) -> String {
    let plain = |byte: u8| byte.is_ascii_alphanumeric() || b"_.-~".contains(&byte);
    text.bytes().map(|byte| if plain(byte) { char::from(byte).to_string() } else { format!("%{byte:02X}") }).collect()
}
