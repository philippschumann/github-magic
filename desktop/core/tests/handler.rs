mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use common::{Repo, calls, head, tmp, write};
use github_magic_core::settings::Settings;
use github_magic_core::{Core, handler};
use serde_json::{Value, json};

/// An Android repo (`o/android`) and an iOS repo (`o/ios`) with a PR branch `feat/a` each, IntelliJ IDEA as the
/// only Android IDE and the chosen one, Xcode and VS Code for iOS.
struct Fixture {
    tmp: PathBuf,
    core: Arc<Core>,
    android: Repo,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tmp();
        let (android, ios) = (Repo::new(&tmp, "Android"), Repo::new(&tmp, "Ios"));
        fs::create_dir_all(ios.root.join("App.xcodeproj")).unwrap();
        for repo in [&android, &ios] {
            repo.push_commit("feat/a");
        }
        for app in ["IntelliJ IDEA CE.app", "Xcode.app", "Visual Studio Code.app"] {
            fs::create_dir_all(tmp.join("Applications").join(app)).unwrap();
        }
        let core = common::core(&tmp, |s| {
            let android_repo = s.repos.entry("o/android".into()).or_default();
            android_repo.path = android.root.to_str().unwrap().into();
            android_repo.app_id = "com.example.myapp".into();
            let ios_repo = s.repos.entry("o/ios".into()).or_default();
            ios_repo.path = ios.root.to_str().unwrap().into();
            ios_repo.scheme = "MyApp".into();
            s.ide = "IntelliJ IDEA".into();
        });
        Self { core, tmp, android }
    }

    fn set(&self, change: impl FnOnce(&mut Settings)) {
        self.core.settings.update(change).unwrap();
    }

    fn send(&self, msg: Value) -> Value {
        let mut request = json!({ "protocol": 1 });
        request.as_object_mut().unwrap().extend(msg.as_object().unwrap().clone());
        handler::handle(&self.core, &request)
    }

    /// A click on a button of PR #1 (branch `feat/a`) of the Android repo, unless `extra` says otherwise.
    fn act(&self, action: &str, extra: Value) -> Value {
        let mut msg = json!({ "repo": "o/android", "pr": 1, "action": action, "branch": "feat/a" });
        msg.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        self.send(msg)
    }

    /// What the stand-in `open` was called with.
    fn opened(&self) -> Vec<String> {
        calls(&self.tmp.join("bin")).into_iter().filter(|call| call.starts_with("open ")).collect()
    }

    fn app(&self, name: &str) -> String {
        self.tmp.join("Applications").join(name).display().to_string()
    }
}

fn error(reply: &Value) -> &str {
    assert_eq!(reply["ok"], false, "{reply}");
    reply["error"].as_str().unwrap()
}

fn path(reply: &Value) -> &Path {
    assert_eq!(reply["ok"], true, "{reply}");
    Path::new(reply["path"].as_str().unwrap())
}

#[test]
fn rejects_bad_input() {
    let f = Fixture::new();
    for (bad, message) in [
        (json!({ "repo": "evil/repo" }), "evil/repo isn't added yet"),
        (json!({ "repo": null }), "isn't added yet"),
        (json!({ "pr": "1; rm -rf /" }), "invalid PR"),
        (json!({ "pr": true }), "invalid PR"),
        (json!({ "pr": -3 }), "invalid PR"),
        (json!({ "action": "rm" }), "unknown action"),
        (json!({ "action": "add_repo" }), "unknown action"), // only the app itself adds repos, through its folder picker
        (json!({ "action": null }), "unknown action"),
        (json!({ "branch": "--upload-pack=x" }), "can't check out"),
        (json!({ "branch": "../../x" }), "can't check out"),
        (json!({ "branch": "someone:feat/a" }), "forks"),
        (json!({ "branch": null }), "can't check out"),
    ] {
        let reply = f.act("open", bad.clone());
        assert!(error(&reply).contains(message), "{bad}: {reply}");
    }
    assert_eq!(f.opened(), [] as [&str; 0]);
}

#[test]
fn settings_that_no_longer_hold_are_rejected() {
    let f = Fixture::new();
    f.set(|s| s.ide = "Android Studio".into()); // uninstalled since it was chosen
    assert!(error(&f.act("open", json!({}))).contains("not installed"));
    f.set(|s| s.worktree_dir = f.tmp.join("deleted").to_str().unwrap().into());
    assert!(error(&f.act("claude", json!({}))).contains("does not exist"));
    assert_eq!(f.opened(), [] as [&str; 0]);
}

#[test]
fn requests_of_another_protocol_are_told_which_side_to_update() {
    let f = Fixture::new();
    let detect = |protocol: Value| handler::handle(&f.core, &json!({ "action": "detect", "protocol": protocol }));
    assert_eq!(detect(json!(1))["ok"], true);
    for (protocol, older) in [(json!(null), "extension"), (json!(0), "extension"), (json!("1"), "extension"), (json!(2), "app")] {
        let reply = detect(protocol);
        assert_eq!(reply, json!({ "ok": false, "title": "Update needed", "error": format!("Update the Github Magic {older}") }));
    }
}

#[test]
fn open_starts_the_chosen_ide_then_updates() {
    let f = Fixture::new();
    let reply = f.act("open", json!({}));
    let worktree = path(&reply);
    assert_eq!(worktree, f.tmp.join("Android-feat-a"));
    // A later click on the same PR opens the same folder and brings it up to date.
    let pushed = f.android.push_commit("feat/a");
    assert_eq!(path(&f.act("open", json!({}))), worktree);
    assert_eq!(head(worktree), pushed);
    assert_eq!(f.opened(), vec![format!("open -a {} {}", f.app("IntelliJ IDEA CE.app"), worktree.display()); 2]);
}

#[test]
fn worktrees_go_into_the_chosen_folder() {
    let f = Fixture::new();
    let dir = f.tmp.join("worktrees");
    fs::create_dir_all(&dir).unwrap();
    f.set(|s| s.worktree_dir = dir.to_str().unwrap().into());
    assert_eq!(path(&f.act("open", json!({}))), dir.join("Android-feat-a"));
}

#[test]
fn ios_repo_opens_in_the_chosen_ios_ide_xcode_with_its_project() {
    let f = Fixture::new();
    // The Android IDE setting doesn't count for iOS repos.
    assert!(error(&f.act("open", json!({ "repo": "o/ios" }))).contains("not installed"));
    f.set(|s| s.ios_ide = "Xcode".into());
    let reply = f.act("open", json!({ "repo": "o/ios" }));
    let worktree = path(&reply).display().to_string();
    f.set(|s| s.ios_ide = "VS Code".into());
    f.act("open", json!({ "repo": "o/ios" }));
    assert_eq!(f.opened(), [
        format!("open -a {} {worktree}/App.xcodeproj", f.app("Xcode.app")),
        format!("open -a {} {worktree}", f.app("Visual Studio Code.app")),
    ]);
}

#[test]
fn deleted_branch_stops_ide_actions_before_anything_runs_but_not_claude() {
    let f = Fixture::new();
    f.set(|s| s.ios_ide = "Xcode".into());
    let worktree = path(&f.act("open", json!({}))).to_owned(); // checked out while the PR was open
    f.android.delete_branch("feat/a");
    for extra in [json!({}), json!({ "branch": "feat/gone" }), json!({ "branch": "feat/gone", "repo": "o/ios" })] {
        let reply = f.act("open", extra);
        assert!(error(&reply).contains("no longer exists on GitHub"));
        assert_eq!(reply["title"], "Branch not found");
    }
    assert_eq!(f.opened().len(), 1);
    assert!(!f.tmp.join("Android-feat-gone").exists());
    assert_eq!(path(&f.act("claude", json!({ "title": "Fix login" }))), worktree);
    let folder = worktree.to_str().unwrap().replace('/', "%2F");
    assert_eq!(f.opened()[1], format!("open claude://code/new?folder={folder}&q=Fix%20login%20%28%231%29%3A%20"));
}

#[test]
fn two_prs_of_one_repo_at_once_take_turns_with_git() {
    let f = Fixture::new();
    let branches: Vec<String> = (0..4).map(|n| format!("feat/{n}")).collect();
    for branch in &branches {
        f.android.push_commit(branch);
    }
    let f = &f;
    thread::scope(|scope| {
        let clicks: Vec<_> = branches.iter().map(|branch| scope.spawn(move || f.act("open", json!({ "branch": branch })))).collect();
        for click in clicks {
            path(&click.join().unwrap());
        }
    });
}

#[test]
fn device_run_updates_then_runs_in_the_background() {
    let f = Fixture::new();
    let worktree = path(&f.act("open", json!({}))).to_owned();
    let pushed = f.android.push_commit("feat/a");
    let reply = f.act("phone", json!({}));
    assert_eq!(path(&reply), worktree);
    assert_eq!(head(&worktree), pushed);
    assert_eq!(reply["run"], 0);
    // The repo has no Android SDK, so the run fails; the button's poll reports why and where the log is.
    let status = || f.send(json!({ "action": "status", "run": reply["run"] }));
    while status()["state"] == "running" {
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(status()["state"], "failed");
    assert!(status()["message"].as_str().unwrap().contains("local.properties"), "{}", status());
    assert_eq!(status()["log"], json!(f.tmp.join("logs/feat-a-phone.log")));
}

#[test]
fn emulator_run_uses_the_chosen_emulator() {
    let f = Fixture::new();
    write(f.core.paths.avd_dir.join("Medium_Phone.ini"), "");
    for name in ["-wipe-data", ""] {
        f.set(|s| s.emulator = name.into()); // not chosen yet, or deleted since
        assert!(error(&f.act("emulator", json!({}))).contains("not found"));
    }
    assert!(!f.tmp.join("Android-feat-a").exists()); // rejected before anything runs
    f.set(|s| s.emulator = "Medium_Phone".into());
    assert_eq!(f.act("emulator", json!({}))["run"], 0);
    assert_eq!(f.core.runs.latest()[0].device(), "Medium_Phone");
}

/// An xcrun that lists one simulator and logs its calls, like the stand-in.
const XCRUN_WITH_A_SIMULATOR: &str = r#"#!/bin/sh
echo "xcrun $*" >> "$(dirname "$0")/calls.log"
[ "$*" = "simctl list devices available -j" ] && echo '{"devices": {"com.apple.CoreSimulator.SimRuntime.iOS-26-3": [{"name": "iPhone 17", "udid": "SIM-UDID"}]}}'
exit 0
"#;

#[test]
fn simulator_run_uses_the_chosen_simulators_udid() {
    let f = Fixture::new();
    write(f.tmp.join("bin/xcrun"), XCRUN_WITH_A_SIMULATOR);
    for udid in ["iPhone 17 (iOS 26.3)", ""] {
        f.set(|s| s.simulator = udid.into()); // a label is no UDID
        assert!(error(&f.act("simulator", json!({ "repo": "o/ios" }))).contains("not found"));
    }
    assert!(!f.tmp.join("Ios-feat-a").exists()); // rejected before anything runs
    f.set(|s| s.simulator = "SIM-UDID".into());
    assert_eq!(f.act("simulator", json!({ "repo": "o/ios" }))["run"], 0);
    let run = f.core.runs.latest().remove(0);
    assert_eq!(run.device(), "iPhone 17 (iOS 26.3)");
    while run.is_running() {
        thread::sleep(Duration::from_millis(50));
    }
    assert!(calls(&f.tmp.join("bin")).contains(&"xcrun simctl bootstatus SIM-UDID -b".to_owned()));
    // It got as far as building the repo's scheme: the project folder just isn't checked in here.
    assert!(run.status()["message"].as_str().unwrap().starts_with("no Xcode project in"), "{}", run.status());
}

#[test]
fn status_knows_only_this_apps_runs() {
    let f = Fixture::new();
    for run in [json!(0), json!(99), json!("0"), json!(-1), json!(null)] {
        assert_eq!(error(&f.send(json!({ "action": "status", "run": run }))), "unknown run");
    }
}
