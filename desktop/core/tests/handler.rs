mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use common::{Repo, calls, head, paths, tmp, write};
use github_magic_core::settings::Config;
use github_magic_core::{Core, handler};
use serde_json::{Value, json};

/// An Android repo (`o/android`) and an iOS repo (`o/ios`) with a PR branch `feat/a` each, IntelliJ IDEA as the
/// only Android IDE, Xcode and VS Code for iOS.
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
        let root = |repo: &Repo| repo.root.to_str().unwrap().to_owned();
        let repos = [("o/android".to_owned(), root(&android)), ("o/ios".to_owned(), root(&ios))].into();
        Self { core: Arc::new(Core::new(Config { repos, app_id: "com.example.myapp".into(), ios_scheme: "MyApp".into(), ..Config::default() }, paths(&tmp))), tmp, android }
    }

    fn act(&self, action: &str, extra: Value) -> Value {
        let mut msg = json!({ "repo": "o/android", "pr": 1, "action": action, "branch": "feat/a", "ide": "IntelliJ IDEA" });
        msg.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        handler::handle(&self.core, &msg)
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
        (json!({ "repo": "evil/repo" }), "not configured"),
        (json!({ "pr": "1; rm -rf /" }), "invalid PR"),
        (json!({ "pr": true }), "invalid PR"),
        (json!({ "pr": -3 }), "invalid PR"),
        (json!({ "action": "rm" }), "unknown action"),
        (json!({ "branch": "--upload-pack=x" }), "can't check out"),
        (json!({ "branch": "../../x" }), "can't check out"),
        (json!({ "branch": "someone:feat/a" }), "forks"),
        (json!({ "branch": null }), "can't check out"),
        (json!({ "worktreeDir": "/does/not/exist" }), "does not exist"),
        (json!({ "worktreeDir": "relative/dir" }), "does not exist"),
        (json!({ "ide": "Notepad" }), "not installed"),
        (json!({ "ide": "/Applications/Calculator.app" }), "not installed"),
    ] {
        let reply = f.act("open", bad.clone());
        assert!(error(&reply).contains(message), "{bad}: {reply}");
    }
    assert_eq!(f.opened(), [] as [&str; 0]);
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
    assert_eq!(path(&f.act("open", json!({ "worktreeDir": dir }))), dir.join("Android-feat-a"));
}

#[test]
fn ios_repo_opens_in_the_chosen_ios_ide_xcode_with_its_project() {
    let f = Fixture::new();
    let reply = f.act("open", json!({ "repo": "o/ios", "iosIde": "Xcode" }));
    let worktree = path(&reply).display().to_string();
    f.act("open", json!({ "repo": "o/ios", "iosIde": "VS Code" }));
    assert_eq!(f.opened(), [
        format!("open -a {} {worktree}/App.xcodeproj", f.app("Xcode.app")),
        format!("open -a {} {worktree}", f.app("Visual Studio Code.app")),
    ]);
    // The Android IDE setting doesn't count for iOS repos.
    for ide in [json!("IntelliJ IDEA"), json!(null)] {
        assert!(error(&f.act("open", json!({ "repo": "o/ios", "iosIde": ide }))).contains("not installed"));
    }
}

#[test]
fn deleted_branch_stops_ide_actions_before_anything_runs_but_not_claude() {
    let f = Fixture::new();
    let worktree = path(&f.act("open", json!({}))).to_owned(); // checked out while the PR was open
    f.android.delete_branch("feat/a");
    for extra in [json!({}), json!({ "branch": "feat/gone" }), json!({ "branch": "feat/gone", "repo": "o/ios", "iosIde": "Xcode" })] {
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
    assert_eq!(reply["log"], json!(f.tmp.join("logs/feat-a-phone.log")));
    // The repo has no Android SDK, so the run fails; the button's poll reports why.
    let status = || handler::handle(&f.core, &json!({ "action": "status", "log": reply["log"] }));
    while status()["state"] == "running" {
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(status()["state"], "failed");
    assert!(status()["message"].as_str().unwrap().contains("local.properties"), "{}", status());
}

#[test]
fn emulator_run_uses_the_chosen_emulator() {
    let f = Fixture::new();
    write(f.core.paths.avd_dir.join("Medium_Phone.ini"), "");
    for name in [json!("-wipe-data"), json!(""), json!(null)] {
        assert!(error(&f.act("emulator", json!({ "emulator": name }))).contains("not found"));
    }
    assert!(!f.tmp.join("Android-feat-a").exists()); // rejected before anything runs
    let reply = f.act("emulator", json!({ "emulator": "Medium_Phone" }));
    assert_eq!(reply["log"], json!(f.tmp.join("logs/feat-a-emulator.log")));
    assert_eq!(f.core.runs.latest()[0].device(), "Medium_Phone");
}

#[test]
fn status_knows_only_this_apps_runs() {
    let f = Fixture::new();
    for log in [json!("/etc/passwd"), json!(f.tmp.join("logs/feat-a-phone.log")), json!(null), json!(3)] {
        assert_eq!(error(&handler::handle(&f.core, &json!({ "action": "status", "log": log }))), "unknown run");
    }
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
    for name in [json!("iPhone 1 (iOS 26.3)"), json!(""), json!(null)] {
        assert!(error(&f.act("simulator", json!({ "repo": "o/ios", "simulator": name }))).contains("not found"));
    }
    assert!(!f.tmp.join("Ios-feat-a").exists()); // rejected before anything runs
    let reply = f.act("simulator", json!({ "repo": "o/ios", "simulator": "iPhone 17 (iOS 26.3)" }));
    assert_eq!(reply["log"], json!(f.tmp.join("logs/feat-a-simulator.log")));
    let run = f.core.runs.latest().remove(0);
    assert_eq!(run.device(), "iPhone 17 (iOS 26.3)");
    while run.is_running() {
        thread::sleep(Duration::from_millis(50));
    }
    assert!(calls(&f.tmp.join("bin")).contains(&"xcrun simctl bootstatus SIM-UDID -b".to_owned()));
}
