mod common;

use std::fs;

use common::{Repo, core, git, tmp, write};
use github_magic_core::settings::{self, Settings, Store};
use github_magic_core::{detect, handler};
use serde_json::{Value, json};

/// An xcrun that lists one simulator.
const XCRUN_WITH_A_SIMULATOR: &str = r#"#!/bin/sh
[ "$*" = "simctl list devices available -j" ] && echo '{"devices": {"com.apple.CoreSimulator.SimRuntime.iOS-26-3": [{"name": "iPhone 17", "udid": "SIM-UDID"}]}}'
exit 0
"#;

#[test]
fn every_setting_is_checked_before_it_is_saved() {
    let tmp = tmp();
    let core = core(&tmp, |s| s.repos.entry("o/r".into()).or_default().path = "/clone".into());
    fs::create_dir_all(tmp.join("Applications/Xcode.app")).unwrap();
    fs::create_dir_all(tmp.join("Applications/Android Studio.app")).unwrap();
    write(core.paths.avd_dir.join("Pixel_9.ini"), "");
    write(tmp.join("bin/xcrun"), XCRUN_WITH_A_SIMULATOR);
    let folder = tmp.to_str().unwrap();
    let set = |key: &str, value: Value| settings::set(&core, key, &value, &json!("o/r"));

    for (key, good, bad) in [
        ("worktreeDir", vec![json!(folder), json!("")], vec![json!("/does/not/exist"), json!("relative/dir"), json!(3)]),
        ("ide", vec![json!("Android Studio"), json!("")], vec![json!("Xcode"), json!("Notepad"), json!(null)]),
        ("iosIde", vec![json!("Xcode"), json!("")], vec![json!("Android Studio"), json!("/Applications/Calculator.app")]),
        ("emulator", vec![json!("Pixel_9"), json!("")], vec![json!("-wipe-data"), json!(["Pixel_9"])]),
        ("simulator", vec![json!("SIM-UDID"), json!("")], vec![json!("iPhone 17 (iOS 26.3)"), json!("OTHER-UDID")]),
        ("phoneModels", vec![json!(["model:Pixel_10", "model:Pixel_6"]), json!([])], vec![json!(["Pixel 10"]), json!([""]), json!([3]), json!("model:Pixel_10")]),
        ("openAtLogin", vec![json!(false), json!(true)], vec![json!("yes"), json!(null)]),
        ("appId", vec![json!("com.example.my_app2")], vec![json!("myapp"), json!("com.example.-x"), json!("com..app"), json!("com.1x"), json!("")]),
        ("scheme", vec![json!("My App")], vec![json!(""), json!("  "), json!(null)]),
        ("repos", vec![], vec![json!({})]), // repos change only through add_repo and remove_repo
    ] {
        for value in good {
            assert_eq!(set(key, value.clone()), Ok(()), "{key} = {value}");
        }
        let before = core.settings.get();
        for value in bad {
            assert!(set(key, value.clone()).is_err(), "{key} = {value}");
        }
        assert_eq!(core.settings.get(), before, "{key}");
    }
    let saved = core.settings.get();
    assert_eq!((saved.repos["o/r"].app_id.as_str(), saved.repos["o/r"].scheme.as_str()), ("com.example.my_app2", "My App"));
    // An app ID or scheme belongs to one of the repos.
    for repo in [json!("evil/repo"), json!(null)] {
        assert!(settings::set(&core, "appId", &json!("com.example.app"), &repo).unwrap_err().contains("isn't one of your repos"));
    }
}

#[test]
fn each_write_is_saved_and_announced() {
    let tmp = tmp();
    let core = core(&tmp, |_| {});
    let mut changes = core.settings.subscribe();
    let send = |msg: Value| handler::handle(&core, &msg);

    assert_eq!(send(json!({ "action": "set", "key": "phoneModels", "value": ["model:Pixel_10"], "protocol": 1 })), json!({ "ok": true }));
    assert!(changes.try_recv().is_ok() && changes.try_recv().is_err()); // told once
    let reply = send(json!({ "action": "set", "key": "phoneModels", "value": "Pixel 10", "protocol": 1 }));
    assert!(reply["error"].as_str().unwrap().contains("without spaces"), "{reply}");
    assert!(changes.try_recv().is_err()); // a refused write changes nothing

    // What a restarted app reads.
    let saved = Store::load(&core.paths.config_dir).get();
    assert_eq!(saved, Settings { phone_models: vec!["model:Pixel_10".into()], ..Settings::default() });
    assert!(saved.open_at_login); // on until switched off
    let file: Value = serde_json::from_slice(&fs::read(core.paths.config_dir.join("settings.json")).unwrap()).unwrap();
    assert_eq!(file["phoneModels"], json!(["model:Pixel_10"]));
}

#[test]
fn unreadable_settings_file_is_kept_aside() {
    let tmp = tmp();
    write(tmp.join("settings.json"), "{ not json");
    assert_eq!(Store::load(&tmp).get(), Settings::default());
    assert_eq!(fs::read_to_string(tmp.join("settings.json.invalid")).unwrap(), "{ not json");
}

#[test]
fn a_repo_is_added_by_its_clones_origin() {
    let tmp = tmp();
    let core = core(&tmp, |_| {});
    let clone = Repo::new(&tmp, "App").root;
    let set_origin = |url: &str| git(&["-C", clone.to_str().unwrap(), "remote", "set-url", "origin", url]);

    // From the settings page: the repo is whatever the clone's origin says.
    for url in ["git@github.com:Owner/App.git", "https://github.com/Owner/App", "https://github.com/Owner/App.git/", "ssh://git@github.com/Owner/App.git"] {
        set_origin(url);
        assert_eq!(settings::add_repo(&core, &clone, None).unwrap(), "Owner/App", "{url}");
    }
    assert_eq!(core.settings.get().repos["Owner/App"].path, clone.to_str().unwrap());

    // From a PR page: only a clone of that page's repo.
    assert!(settings::add_repo(&core, &clone, Some("evil/repo")).unwrap_err().contains("is a clone of Owner/App, not of evil/repo"));
    assert_eq!(settings::add_repo(&core, &clone, Some("owner/app")).unwrap(), "owner/app"); // GitHub ignores case
    assert!(settings::add_repo(&core, &tmp, None).unwrap_err().contains("is not a git clone"));
    set_origin("App.git");
    assert!(settings::add_repo(&core, &clone, None).unwrap_err().contains("can't tell the repo"));

    // Adding a repo again keeps its app ID.
    settings::set(&core, "appId", &json!("com.example.app"), &json!("Owner/App")).unwrap();
    set_origin("git@github.com:Owner/App.git");
    settings::add_repo(&core, &clone, None).unwrap();
    assert_eq!(core.settings.get().repos["Owner/App"].app_id, "com.example.app");

    let send = |msg: Value| handler::handle(&core, &msg);
    assert_eq!(send(json!({ "action": "remove_repo", "repo": "Owner/App", "protocol": 1 })), json!({ "ok": true }));
    assert_eq!(core.settings.get().repos.keys().collect::<Vec<_>>(), ["owner/app"]);
}

#[test]
fn settings_page_gets_every_setting_and_the_choices() {
    let tmp = tmp();
    let ios = tmp.join("ios");
    fs::create_dir_all(ios.join("App.xcodeproj")).unwrap();
    fs::create_dir_all(tmp.join("Applications/Xcode.app")).unwrap();
    let core = core(&tmp, |s| {
        s.repos.entry("o/ios".into()).or_default().path = ios.to_str().unwrap().into();
        s.repos.entry("o/android".into()).or_default().path = "/gone".into();
        s.worktree_dir = "~/worktrees".into();
    });
    write(tmp.join("bin/xcrun"), XCRUN_WITH_A_SIMULATOR);
    let reply = detect::settings(&core);
    assert_eq!(reply["settings"], serde_json::to_value(core.settings.get()).unwrap());
    assert_eq!(reply["settings"]["worktreeDir"], "~/worktrees");
    assert_eq!(reply["platforms"], json!({ "o/android": "android", "o/ios": "ios" }));
    assert_eq!(reply["iosIdes"], json!(["Xcode"]));
    assert_eq!(reply["simulators"], json!([{ "id": "SIM-UDID", "label": "iPhone 17 (iOS 26.3)" }]));
}
