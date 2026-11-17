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

/// The repo's platform, installed IDEs, emulators and simulators, and whether adb is in the repo's Android SDK
/// (sdk.dir), which phone and emulator runs use.
pub fn detect(core: &Core, repo: &Value) -> Value {
    let root = repo.as_str().and_then(|repo| core.config.repos.get(repo)).map(|path| core.paths.expand_user(path));
    let sdk = root.as_deref().and_then(|root| android::sdk_dir(root).ok());
    let names = |ides| installed_ides(&core.paths, ides).into_iter().map(|(name, _)| name).collect::<Vec<_>>();
    json!({
        "ok": true,
        "platform": if root.as_deref().and_then(xcode_project).is_some() { "ios" } else { "android" },
        "ides": names(IDES),
        "iosIdes": names(IOS_IDES),
        "avds": avds(&core.paths),
        "simulators": simulators(&core.paths).into_iter().map(|(label, _)| label).collect::<Vec<_>>(),
        "adb": sdk.is_some_and(|sdk| is_executable(&sdk.join("platform-tools/adb"))),
    })
}
