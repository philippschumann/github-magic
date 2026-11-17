//! One request from the extension or the settings page in, one reply out.

use std::sync::{Arc, PoisonError};

use serde_json::{Value, json};

use crate::platform::{IDES, IOS_IDES, installed_ides};
use crate::settings::{self, repo_name};
use crate::{Core, claude, cmd, detect, git, runs};

/// Changes only when the messages change incompatibly, not with every release.
pub const PROTOCOL: u64 = 1;

pub fn handle(core: &Arc<Core>, msg: &Value) -> Value {
    match msg["protocol"].as_u64() {
        Some(PROTOCOL) => try_handle(core, msg).unwrap_or_else(|error| json!({ "ok": false, "error": error })),
        theirs => {
            let older = if theirs.is_some_and(|theirs| theirs > PROTOCOL) { "app" } else { "extension" };
            json!({ "ok": false, "title": "Update needed", "error": format!("Update the Github Magic {older}") })
        }
    }
}

fn try_handle(core: &Arc<Core>, msg: &Value) -> Result<Value, String> {
    let paths = &core.paths;
    let ok = |()| json!({ "ok": true });
    let action = msg["action"].as_str().unwrap_or_default();
    match action {
        "detect" => return Ok(detect::detect(core, &msg["repo"])),
        "settings" => return Ok(detect::settings(core)),
        "set" => return settings::set(core, msg["key"].as_str().unwrap_or_default(), &msg["value"], &msg["repo"]).map(ok),
        "remove_repo" => return settings::remove_repo(core, msg["repo"].as_str().unwrap_or_default()).map(ok),
        "status" => {
            let run = msg["run"].as_u64().and_then(|id| core.runs.get(id));
            return run.map(|run| run.status()).ok_or_else(|| "unknown run".to_owned());
        }
        _ => {}
    }
    let settings = core.settings.get();
    let repo = msg["repo"].as_str().and_then(|repo| settings.repos.get(repo));
    let repo = repo.ok_or_else(|| format!("{} isn't added yet; add it in Github Magic's settings", repo_name(&msg["repo"])))?;
    let pr = msg["pr"].as_u64().filter(|pr| *pr > 0).ok_or_else(|| format!("invalid PR number: {}", msg["pr"]))?;
    if !["open", "claude"].contains(&action) && !runs::TARGETS.contains(&action) {
        return Err(format!("unknown action: {}", msg["action"]));
    }
    let branch = msg["branch"].as_str().filter(|branch| git::valid_branch(paths, branch));
    let branch = branch.ok_or_else(|| format!("can't check out branch {}; PRs from forks aren't supported", msg["branch"]))?;
    let root = paths.expand_user(&repo.path);
    let base_dir = settings::worktree_dir(core, &settings)?;
    let project = detect::xcode_project(&root);
    let ide = if project.is_some() { &settings.ios_ide } else { &settings.ide };
    let app = match action {
        "open" => {
            let installed = installed_ides(paths, if project.is_some() { IOS_IDES } else { IDES });
            let app = installed.into_iter().find(|(name, _)| ide == name).map(|(_, app)| app);
            Some(app.ok_or_else(|| format!("the IDE {ide:?} is not installed; choose another in Settings"))?)
        }
        _ => None,
    };
    // The emulator name goes to the emulator command as an argument, the simulator's UDID to simctl: both must be
    // ones this Mac lists. `label` is what the tray menu calls the device.
    let not_found = |kind: &str, name: &str| format!("the {kind} {name:?} was not found; choose another in Settings");
    let (device, label) = match action {
        "emulator" => {
            let avd = Some(&settings.emulator).filter(|avd| detect::avds(paths).contains(avd));
            let avd = avd.ok_or_else(|| not_found("emulator", &settings.emulator))?;
            (avd.clone(), avd.clone())
        }
        "simulator" => {
            let simulator = detect::simulators(paths).into_iter().find(|(_, udid)| *udid == settings.simulator);
            let (label, udid) = simulator.ok_or_else(|| not_found("simulator", &settings.simulator))?;
            (udid, label)
        }
        _ => (String::new(), action.to_owned()),
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
        (_, _, project) => {
            git::update(paths, &path, branch)?;
            let target = runs::TARGETS.into_iter().find(|target| *target == action).expect("checked above");
            let app = if project.is_some() { &repo.scheme } else { &repo.app_id };
            let launch = runs::Launch { target, pr, branch, path: &path, device: &device, label: &label, app };
            let run = runs::start(core, launch)?;
            return Ok(json!({ "ok": true, "path": path, "run": run.id }));
        }
    }
    Ok(json!({ "ok": true, "path": path }))
}
