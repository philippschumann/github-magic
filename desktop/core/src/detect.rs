//! What is installed: the repo's platform, IDEs, emulators, simulators and adb.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::platform::{IDES, IOS_IDES, Paths, installed_ides, is_executable};
use crate::{Core, android, cmd};

/// The Xcode workspace or project at the folder's root (a workspace wins: it includes the project).
pub fn xcode_project(path: &Path) -> Option<PathBuf> {
    let names = fs::read_dir(path).ok()?.filter_map(|entry| entry.ok()?.file_name().into_string().ok());
    let projects = names.filter(|n| n.ends_with(".xcworkspace") || n.ends_with(".xcodeproj"));
    projects.min_by_key(|n| (!n.ends_with(".xcworkspace"), n.clone())).map(|n| path.join(n))
}

/// Emulator names. Read from the AVD folder, which needs no repo or SDK path.
pub fn avds(paths: &Paths) -> Vec<String> {
    let Ok(entries) = fs::read_dir(&paths.avd_dir) else { return vec![] };
    let names = entries.filter_map(|entry| entry.ok()?.file_name().into_string().ok());
    let mut avds: Vec<String> = names.filter_map(|n| n.strip_suffix(".ini").map(str::to_owned)).collect();
    avds.sort();
    avds
}

/// Available iOS simulators as ("iPhone 17 Pro (iOS 26.3)", UDID), newest iOS first. Empty without Xcode.
pub fn simulators(paths: &Paths) -> Vec<(String, String)> {
    let listed = cmd::run(cmd::command(paths, "xcrun").args(["simctl", "list", "devices", "available", "-j"]));
    listed.map(|json| parse_simulators(&json)).unwrap_or_default()
}

/// Reads `xcrun simctl list devices available -j`.
pub fn parse_simulators(json: &str) -> Vec<(String, String)> {
    let listed: Value = serde_json::from_str(json).unwrap_or_default();
    // Runtime ids end in e.g. "iOS-26-3"; watchOS, tvOS and visionOS runtimes are skipped.
    let ios = listed["devices"].as_object().into_iter().flatten().filter_map(|(runtime, sims)| {
        let version = runtime.rsplit_once(".iOS-")?.1.split('-').map(|n| n.parse().ok()).collect::<Option<Vec<u32>>>()?;
        Some((version, sims))
    });
    let mut ios: Vec<_> = ios.collect();
    ios.sort_by(|a, b| b.0.cmp(&a.0));
    let mut found: Vec<(String, String)> = vec![];
    for (version, sims) in ios {
        let version = version.iter().map(u32::to_string).collect::<Vec<_>>().join(".");
        for sim in sims.as_array().into_iter().flatten() {
            let (Some(name), Some(udid)) = (sim["name"].as_str(), sim["udid"].as_str()) else { continue };
            let label = format!("{name} (iOS {version})");
            // shortcut: two simulators with the same name and iOS share a label; the first one is used.
            if !found.iter().any(|(known, _)| *known == label) {
                found.push((label, udid.to_owned()));
            }
        }
    }
    found
}

/// What the settings can choose from: installed IDEs, emulators and simulators.
pub fn choices(paths: &Paths) -> Value {
    let names = |ides| installed_ides(paths, ides).into_iter().map(|(name, _)| name).collect::<Vec<_>>();
    let simulators = simulators(paths).into_iter().map(|(label, udid)| json!({ "id": udid, "label": label }));
    json!({ "ides": names(IDES), "iosIdes": names(IOS_IDES), "avds": avds(paths), "simulators": simulators.collect::<Vec<_>>() })
}

/// "ios" for a repo whose main clone has an Xcode project at its root, else "android".
pub fn platform(root: Option<&Path>) -> &'static str {
    if root.and_then(xcode_project).is_some() { "ios" } else { "android" }
}

/// What a PR page needs: whether the repo is added, its platform, whether adb is in its Android SDK (sdk.dir, which
/// phone and emulator runs use), the choices and what is chosen.
pub fn detect(core: &Core, repo: &Value) -> Value {
    let settings = core.settings.get();
    let root = repo.as_str().and_then(|repo| settings.repos.get(repo)).map(|repo| core.paths.expand_user(&repo.path));
    let sdk = root.as_deref().and_then(|root| android::sdk_dir(root).ok());
    let mut reply = choices(&core.paths);
    reply.as_object_mut().expect("an object").extend(json!({
        "ok": true,
        "repoConfigured": root.is_some(),
        "platform": platform(root.as_deref()),
        "adb": sdk.is_some_and(|sdk| is_executable(&sdk.join("platform-tools/adb"))),
        "settings": {
            "worktreeDir": settings.worktree_dir,
            "ide": settings.ide,
            "iosIde": settings.ios_ide,
            "emulator": settings.emulator,
            "simulator": settings.simulator,
        },
    }).as_object().expect("an object").clone());
    reply
}

/// What the settings page needs: every setting, each repo's platform, and the choices.
pub fn settings(core: &Core) -> Value {
    let settings = core.settings.get();
    let platforms: serde_json::Map<String, Value> =
        settings.repos.iter().map(|(name, repo)| (name.clone(), platform(Some(&core.paths.expand_user(&repo.path))).into())).collect();
    let mut reply = choices(&core.paths);
    reply.as_object_mut().expect("an object").extend(json!({ "ok": true, "settings": settings, "platforms": platforms }).as_object().expect("an object").clone());
    reply
}
