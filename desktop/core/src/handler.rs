//! One request from the extension in, one reply out.

use std::path::Path;
use std::sync::{Arc, PoisonError};

use serde_json::{Value, json};

use crate::platform::{IDES, IOS_IDES, installed_ides};
use crate::settings::{self, CONFIG};
use crate::{Core, claude, cmd, detect, git, runs};

/// shortcut: actions not ported yet still go to the Python script; gone at the end of phase 2.
pub fn ported(msg: &Value) -> bool {
    matches!(msg.get("action").map_or(Some("open"), Value::as_str), Some("detect" | "open" | "claude" | "status" | "phone" | "emulator"))
}

pub fn handle(core: &Arc<Core>, msg: &Value) -> Value {
    try_handle(core, msg).unwrap_or_else(|error| json!({ "ok": false, "error": error }))
}

fn try_handle(core: &Arc<Core>, msg: &Value) -> Result<Value, String> {
    let paths = &core.paths;
    let action = msg.get("action").map_or(Some("open"), Value::as_str);
    if action == Some("detect") {
        return Ok(detect::detect(core, &msg["repo"]));
    }
    if action == Some("status") {
        // Only runs this app started are known; the path is never opened.
        let run = msg["log"].as_str().and_then(|log| core.runs.by_log(Path::new(log)));
        return run.map(|run| run.status()).ok_or_else(|| "unknown run".to_owned());
    }
    let repo = &msg["repo"];
    let root = repo.as_str().and_then(|repo| core.config.repos.get(repo));
    let root = root.ok_or_else(|| format!("{} is not configured in {CONFIG}", repo.as_str().unwrap_or(&repo.to_string())))?;
    let pr = msg["pr"].as_u64().filter(|pr| *pr > 0).ok_or_else(|| format!("invalid PR number: {}", msg["pr"]))?;
    let action = action.filter(|action| ["open", "claude"].contains(action) || runs::TARGETS.contains(action));
    let action = action.ok_or_else(|| format!("unknown action: {}", msg["action"]))?;
    let branch = msg["branch"].as_str().filter(|branch| git::valid_branch(paths, branch));
    let branch = branch.ok_or_else(|| format!("can't check out branch {}; PRs from forks aren't supported", msg["branch"]))?;
    let root = paths.expand_user(root);
    let base_dir = settings::worktree_dir(paths, &msg["worktreeDir"])?;
    let project = detect::xcode_project(&root);
    let ide = &msg[if project.is_some() { "iosIde" } else { "ide" }];
    let app = match action {
        "open" => {
            let installed = installed_ides(paths, if project.is_some() { IOS_IDES } else { IDES });
            let app = installed.into_iter().find(|(name, _)| ide == name).map(|(_, app)| app);
            Some(app.ok_or_else(|| format!("IDE {ide} is not installed; choose another in the extension's settings"))?)
        }
        _ => None,
    };
    // The emulator name goes to the emulator command as an argument.
    let device = match action {
        "emulator" => {
            let avd = msg["emulator"].as_str().filter(|avd| detect::avds(paths).iter().any(|known| known == avd));
            avd.ok_or_else(|| format!("emulator {} not found; choose another in the extension's settings", msg["emulator"]))?
        }
        _ => "",
    };

    // Checked before anything opens: an existing worktree would open, then fail to update. The Claude button skips
    // it, so a merged PR's session can still be reopened.
    if action != "claude" && git::branch_deleted(paths, &root, branch) {
        let error = format!("The branch {branch} no longer exists on GitHub. It was probably deleted after the PR was merged or closed.");
        return Ok(json!({ "ok": false, "title": "Branch not found", "error": error }));
    }
    let lock = core.repo_lock(&root);
    let _git_steps = lock.lock().unwrap_or_else(PoisonError::into_inner);
    let path = git::ensure_worktree(paths, &root, branch, base_dir.as_deref())?;
    match (action, app, project) {
        ("open", Some(app), project) => {
            // Xcode gets the project file: a plain folder would open as a Swift package.
            let target = match project.as_deref().and_then(|project| project.file_name()) {
                Some(project) if ide == "Xcode" => path.join(project),
                _ => path.clone(),
            };
            cmd::run(cmd::command(paths, "open").arg("-a").arg(app).arg(target))?;
            git::update(paths, &path, branch)?; // `open` returns at once, so the IDE loads the project while the update runs
        }
        ("claude", ..) => claude::open_claude(paths, &path, &format!("{} (#{pr}): ", msg["title"].as_str().unwrap_or_default()))?,
        _ => {
            git::update(paths, &path, branch)?;
            let target = runs::TARGETS.into_iter().find(|target| *target == action).expect("checked above");
            let label = if device.is_empty() { target } else { device };
            let run = runs::start(core, target, pr, branch, &path, device, label)?;
            return Ok(json!({ "ok": true, "path": path, "log": run.log }));
        }
    }
    Ok(json!({ "ok": true, "path": path }))
}
