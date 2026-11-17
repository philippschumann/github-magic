mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use common::{calls, tmp, write};
use github_magic_core::runs::Run;
use github_magic_core::{Core, ios};
use serde_json::json;

/// A worktree with an Xcode project; xcrun, xcodebuild and open are the stand-ins in `tmp/bin`.
struct Fixture {
    tmp: PathBuf,
    core: Arc<Core>,
    worktree: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tmp();
        let worktree = tmp.join("worktree");
        fs::create_dir_all(worktree.join("App.xcodeproj")).unwrap();
        Self { core: common::core(&tmp, |_| {}), tmp, worktree }
    }

    /// The devices `xcrun devicectl list devices` reports, as (UDID, kind, tunnel state).
    fn devices(&self, devices: &[(&str, &str, &str)]) {
        let devices = devices.iter().map(|(udid, kind, tunnel)| json!({
            "hardwareProperties": { "udid": udid, "deviceType": kind, "reality": "physical" },
            "connectionProperties": { "pairingState": "paired", "tunnelState": tunnel },
            "deviceProperties": { "name": format!("{udid}'s {kind}") },
        }));
        let devices: Vec<_> = devices.collect();
        write(self.tmp.join("bin/devices.json"), &json!({ "result": { "devices": devices } }).to_string());
    }

    fn launch(&self, target: &'static str, udid: &str) -> (Result<String, String>, Run) {
        let run = Run::new(&self.core.paths.log_dir, 0, target, 1, "feat/a", &self.worktree, "").unwrap();
        (ios::launch(&self.core, &run, udid, "MyApp"), run)
    }

    /// The start of every xcodebuild call, up to the device's UDID.
    fn build(&self) -> String {
        format!("xcodebuild -project {} -scheme MyApp -configuration Debug -destination id=", self.worktree.join("App.xcodeproj").display())
    }
}

#[test]
fn iphone_gets_a_signed_build_on_a_connected_iphone() {
    let f = Fixture::new();
    f.devices(&[("GONE", "iPhone", "unavailable"), ("IPAD", "iPad", "connected"), ("IDLE", "iPhone", "disconnected"), ("USB", "iPhone", "connected")]);
    let (udid, run) = f.launch("iphone", "");
    assert_eq!(udid.unwrap(), "USB");
    assert_eq!(run.device(), "USB's iPhone");
    let build = f.build();
    assert_eq!(calls(&f.tmp.join("bin"))[1..], [
        format!("{build}USB -allowProvisioningUpdates build"),
        format!("{build}USB -showBuildSettings -json"),
        "xcrun devicectl device install app --device USB /b/MyApp.app".to_owned(),
        "xcrun devicectl device process launch --terminate-existing --device USB co.x".to_owned(),
    ]);
}

#[test]
fn no_reachable_iphone() {
    let f = Fixture::new();
    f.devices(&[("GONE", "iPhone", "unavailable"), ("IPAD", "iPad", "connected")]);
    assert!(f.launch("iphone", "").0.unwrap_err().contains("no iPhone"));
}

#[test]
fn simulator_is_booted_and_shown_then_gets_the_build() {
    let f = Fixture::new();
    assert_eq!(f.launch("simulator", "SIM").0.unwrap(), "SIM");
    let build = f.build();
    assert_eq!(calls(&f.tmp.join("bin")), [
        "xcrun simctl bootstatus SIM -b".to_owned(),
        "open -a Simulator".to_owned(),
        format!("{build}SIM build"),
        format!("{build}SIM -showBuildSettings -json"),
        "xcrun simctl install SIM /b/MyApp.app".to_owned(),
        "xcrun simctl launch --terminate-running-process SIM co.x".to_owned(),
    ]);
}
