mod common;

use std::fs;
use std::path::Path;

use common::{fixture, paths, tmp, write};
use github_magic_core::platform::{IDES, installed_ides};
use github_magic_core::settings::Config;
use github_magic_core::{Core, detect, handler};
use serde_json::json;

fn core(tmp: &Path) -> Core {
    fs::create_dir_all(tmp.join("clone")).unwrap();
    let repos = [("o/r".to_owned(), tmp.join("clone").to_str().unwrap().to_owned())].into();
    Core::new(Config { repos, ..Config::default() }, paths(tmp))
}

#[test]
fn lists_installed_ides_in_order_with_their_app() {
    let tmp = tmp();
    let core = core(&tmp);
    for app in ["home/Applications/IntelliJ IDEA CE.app", "Applications/Visual Studio Code.app", "Applications/Xcode.app"] {
        fs::create_dir_all(tmp.join(app)).unwrap();
    }
    let found = [
        ("IntelliJ IDEA", tmp.join("home/Applications/IntelliJ IDEA CE.app")),
        ("VS Code", tmp.join("Applications/Visual Studio Code.app")),
    ];
    assert_eq!(installed_ides(&core.paths, IDES), found);
    let reply = handler::handle(&core, &json!({ "action": "detect" }));
    assert_eq!(reply["ides"], json!(["IntelliJ IDEA", "VS Code"]));
    assert_eq!(reply["iosIdes"], json!(["Xcode", "VS Code"]));
}

#[test]
fn repo_with_an_xcode_project_at_its_root_is_ios() {
    let tmp = tmp();
    let core = core(&tmp);
    let clone = tmp.join("clone");
    assert_eq!(detect::detect(&core, &json!("o/r"))["platform"], "android");
    fs::create_dir_all(clone.join("App.xcodeproj")).unwrap();
    assert_eq!(detect::detect(&core, &json!("o/r"))["platform"], "ios");
    assert_eq!(detect::detect(&core, &json!("evil/repo"))["platform"], "android");
    fs::create_dir_all(clone.join("App.xcworkspace")).unwrap();
    assert_eq!(detect::xcode_project(&clone), Some(clone.join("App.xcworkspace")));
}

#[test]
fn simulators_newest_ios_first_without_other_platforms() {
    let listed = json!({ "devices": {
        "com.apple.CoreSimulator.SimRuntime.iOS-18-6": [{ "name": "iPhone 16", "udid": "A" }],
        "com.apple.CoreSimulator.SimRuntime.watchOS-11-0": [{ "name": "Apple Watch", "udid": "W" }],
        "com.apple.CoreSimulator.SimRuntime.iOS-26-3": [{ "name": "iPhone 17", "udid": "B" }, { "name": "iPad (A16)", "udid": "C" }],
        "com.apple.CoreSimulator.SimRuntime.iOS-9-3": [{ "name": "iPhone 6s", "udid": "D" }],
    } });
    let expected = [("iPhone 17 (iOS 26.3)", "B"), ("iPad (A16) (iOS 26.3)", "C"), ("iPhone 16 (iOS 18.6)", "A"), ("iPhone 6s (iOS 9.3)", "D")];
    assert_eq!(detect::parse_simulators(&listed.to_string()), expected.map(|(label, udid)| (label.to_owned(), udid.to_owned())));
    // No Xcode: the stand-in xcrun lists nothing.
    assert_eq!(detect::simulators(&paths(&tmp())), []);
}

#[test]
fn lists_emulators_by_their_ini_files() {
    let tmp = tmp();
    let core = core(&tmp);
    assert_eq!(detect::detect(&core, &json!(null))["avds"], json!([])); // no AVD folder
    for name in ["Pixel_9_API_35.ini", "Pixel_9_API_35.avd/config.ini", "Medium_Phone_API_36.1.ini", "gradle-managed/x"] {
        write(core.paths.avd_dir.join(name), "");
    }
    let reply = handler::handle(&core, &json!({ "action": "detect" }));
    assert_eq!(reply["avds"], json!(["Medium_Phone_API_36.1", "Pixel_9_API_35"]));
}

#[test]
fn adb_only_when_the_repos_sdk_has_it() {
    let tmp = tmp();
    let core = core(&tmp);
    let adb = |repo| detect::detect(&core, &repo)["adb"] == true;
    assert!(!adb(json!("o/r"))); // no local.properties
    write(tmp.join("clone/local.properties"), &format!("sdk.dir={}\n", tmp.join("sdk").display()));
    assert!(!adb(json!("o/r"))); // sdk without platform-tools
    fixture("adb", &tmp.join("sdk/platform-tools"));
    assert!(adb(json!("o/r")));
    assert!(!adb(json!("evil/repo")));
    assert!(!adb(json!(null)));
}
