//! iOS device runs: pick the iPhone or boot the simulator, build the scheme, install and start the app.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::detect::xcode_project;
use crate::runs::Run;
use crate::{Core, cmd};

/// The first paired iPhone that isn't out of reach, connected ones first.
fn iphone_udid(core: &Core, run: &Run) -> Result<String, String> {
    let out = core.paths.log_dir.join("devices.json"); // devicectl writes its JSON only to a file
    run.capture(cmd::command(&core.paths, "xcrun").args(["devicectl", "list", "devices", "--json-output"]).arg(&out))?;
    let listed = fs::read(&out).ok().and_then(|json| serde_json::from_slice::<Value>(&json).ok()).unwrap_or_default();
    let phones = listed["result"]["devices"].as_array().into_iter().flatten().filter(|device| {
        let (hardware, connection) = (&device["hardwareProperties"], &device["connectionProperties"]);
        hardware["deviceType"] == "iPhone"
            && hardware["reality"] == "physical"
            && connection["pairingState"] == "paired"
            && connection["tunnelState"] != "unavailable"
    });
    let phone = phones.min_by_key(|device| device["connectionProperties"]["tunnelState"] != "connected");
    let phone = phone.ok_or("no iPhone connected; plug one in, unlock it and trust this Mac")?;
    if let Some(name) = phone["deviceProperties"]["name"].as_str() {
        run.set_device(name);
    }
    Ok(phone["hardwareProperties"]["udid"].as_str().unwrap_or_default().to_owned())
}

/// Builds the scheme for the iPhone or the simulator with this UDID, installs it there and starts it. Returns the
/// device's UDID.
pub fn launch(core: &Core, run: &Run, udid: &str) -> Result<String, String> {
    let scheme = &core.config.ios_scheme;
    if scheme.is_empty() {
        return Err("no iOS scheme is configured".into());
    }
    let xcrun = |args: &[&str]| run.capture(cmd::command(&core.paths, "xcrun").args(args));
    let udid = if run.target == "simulator" {
        run.say(&format!("booting simulator {udid}"));
        xcrun(&["simctl", "bootstatus", udid, "-b"])?; // boots it if needed and waits until it has
        run.capture(cmd::command(&core.paths, "open").args(["-a", "Simulator"]))?;
        udid.to_owned()
    } else {
        iphone_udid(core, run)?
    };
    let project = xcode_project(&run.path).ok_or_else(|| format!("no Xcode project in {}", run.path.display()))?;
    let xcodebuild = || {
        let mut xcodebuild = cmd::command(&core.paths, "xcodebuild");
        let kind = if project.extension().is_some_and(|e| e == "xcworkspace") { "-workspace" } else { "-project" };
        xcodebuild.arg(kind).arg(&project).args(["-scheme", scheme, "-configuration", "Debug", "-destination"]);
        xcodebuild.arg(format!("id={udid}")).current_dir(&run.path);
        xcodebuild
    };
    run.say(&format!("building {scheme} for {udid}"));
    // The iPhone needs a signed build: automatic signing may have to register it and fetch a profile.
    let signing: &[&str] = if run.target == "iphone" { &["-allowProvisioningUpdates"] } else { &[] };
    if !run.stream(xcodebuild().args(signing).arg("build"))?.0 {
        return Err(format!("xcodebuild failed for {udid}"));
    }
    let targets: Value = serde_json::from_str(&run.capture(xcodebuild().args(["-showBuildSettings", "-json"]))?).unwrap_or_default();
    let settings = targets.as_array().into_iter().flatten().map(|target| &target["buildSettings"]);
    let (Some(dir), Some(name), Some(bundle_id)) = settings
        .filter(|settings| settings["WRAPPER_EXTENSION"] == "app")
        .map(|s| (s["TARGET_BUILD_DIR"].as_str(), s["WRAPPER_NAME"].as_str(), s["PRODUCT_BUNDLE_IDENTIFIER"].as_str()))
        .next()
        .unwrap_or_default()
    else {
        return Err(format!("the scheme {scheme} builds no app"));
    };
    let app = Path::new(dir).join(name).display().to_string();
    run.say(&format!("installing {app} on {udid}"));
    if run.target == "simulator" {
        xcrun(&["simctl", "install", &udid, &app])?;
        xcrun(&["simctl", "launch", "--terminate-running-process", &udid, bundle_id])?;
    } else {
        xcrun(&["devicectl", "device", "install", "app", "--device", &udid, &app])?;
        xcrun(&["devicectl", "device", "process", "launch", "--terminate-existing", "--device", &udid, bundle_id])?;
    }
    Ok(udid)
}
