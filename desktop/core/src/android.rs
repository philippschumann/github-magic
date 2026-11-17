//! Android device runs: pick the phone or boot the emulator, install the debug build, start the app.

use std::fs::{self, File};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::platform::java_home;
use crate::runs::Run;
use crate::{Core, cmd};

const BOOT_TIMEOUT: Duration = Duration::from_secs(300);

/// The Android SDK the repo builds with: `sdk.dir` in its local.properties.
pub fn sdk_dir(path: &Path) -> Result<PathBuf, String> {
    let file = path.join("local.properties");
    let text = fs::read_to_string(&file).map_err(|e| format!("can't read {}: {e}", file.display()))?;
    let dir = text.lines().find_map(|line| line.strip_prefix("sdk.dir="));
    dir.map(|dir| PathBuf::from(dir.trim())).ok_or_else(|| format!("no sdk.dir in {}", file.display()))
}

/// The connected phone to use: the first of the preferred models, else any other phone.
pub fn phone_serial(core: &Core, run: &Run, adb: &Path) -> Result<String, String> {
    let listed = run.capture(cmd::command(&core.paths, adb).args(["devices", "-l"]))?;
    let phones = listed.lines().skip(1).filter(|line| {
        let mut words = line.split_whitespace();
        words.next().is_some_and(|serial| !serial.starts_with("emulator-")) && words.next() == Some("device")
    });
    let phones: Vec<&str> = phones.collect();
    let preferred = core.settings.get().phone_models;
    let models = preferred.iter().map(String::as_str).chain([""]);
    let phone = models.flat_map(|model| phones.iter().filter(move |line| line.contains(model))).next();
    let mut words = phone.ok_or("no phone connected; plug one in or pair it over Wi-Fi")?.split_whitespace();
    let serial = words.next().unwrap_or_default().to_owned();
    if let Some(model) = words.find_map(|word| word.strip_prefix("model:")) {
        run.set_device(&model.replace('_', " "));
    }
    Ok(serial)
}

/// The serial of the running emulator with this AVD.
fn avd_serial(core: &Core, run: &Run, adb: &Path, avd: &str) -> Result<Option<String>, String> {
    let listed = run.capture(cmd::command(&core.paths, adb).arg("devices"))?;
    for line in listed.lines().skip(1) {
        let mut words = line.split_whitespace();
        let (Some(serial), Some("device")) = (words.next(), words.next()) else { continue };
        let name = run.capture(cmd::command(&core.paths, adb).args(["-s", serial, "emu", "avd", "name"]));
        if serial.starts_with("emulator-") && name.is_ok_and(|name| name.lines().next().map(str::trim) == Some(avd)) {
            return Ok(Some(serial.to_owned()));
        }
    }
    Ok(None)
}

fn emulator_ready(core: &Core, run: &Run, adb: &Path, serial: &str) -> bool {
    let shell = |args: &[&str]| run.capture(cmd::command(&core.paths, adb).args(["-s", serial, "shell"]).args(args));
    // boot_completed alone is not enough: some AVDs refuse installs until the package service answers.
    shell(&["getprop", "sys.boot_completed"]).is_ok_and(|booted| booted == "1")
        && shell(&["pm", "path", "android"]).is_ok_and(|path| path.starts_with("package:"))
}

fn wait_for<T>(run: &Run, what: &str, mut check: impl FnMut() -> Result<Option<T>, String>) -> Result<T, String> {
    let deadline = Instant::now() + BOOT_TIMEOUT;
    loop {
        if let Some(result) = check()? {
            return Ok(result);
        }
        if Instant::now() > deadline {
            return Err(format!("{what} within {}s", BOOT_TIMEOUT.as_secs()));
        }
        run.pause(Duration::from_secs(2))?;
    }
}

/// The serial of the emulator with this AVD, booted first if it isn't running.
pub fn emulator_serial(core: &Core, run: &Run, adb: &Path, sdk: &Path, avd: &str) -> Result<String, String> {
    let serial = match avd_serial(core, run, adb, avd)? {
        Some(serial) => serial,
        None => {
            run.say(&format!("booting {avd}"));
            let log = core.paths.log_dir.join(format!("{avd}.log"));
            let io_error = |e| format!("can't start {avd} (see {}): {e}", log.display());
            let out = File::create(&log).map_err(io_error)?;
            let mut emulator = cmd::command(&core.paths, sdk.join("emulator/emulator"));
            emulator.args(["-avd", avd]).stdout(out.try_clone().map_err(io_error)?).stderr(out);
            // Its own session: the emulator outlives the run and the app.
            // SAFETY: setsid is async-signal-safe, as pre_exec needs.
            unsafe { emulator.pre_exec(|| (libc::setsid() != -1).then_some(()).ok_or_else(std::io::Error::last_os_error)) };
            let mut emulator = emulator.spawn().map_err(io_error)?;
            let serial = wait_for(run, &format!("{avd} did not come up"), || match emulator.try_wait() {
                Ok(None) => avd_serial(core, run, adb, avd),
                _ => Err(format!("{avd} exited without booting (see {})", log.display())),
            });
            thread::spawn(move || emulator.wait()); // no zombie once it is closed
            serial?
        }
    };
    wait_for(run, &format!("{serial} did not finish booting"), || Ok(emulator_ready(core, run, adb, &serial).then_some(())))?;
    Ok(serial)
}

/// Builds and installs the debug build. Returns whether that worked and what Gradle printed.
fn gradle_install(core: &Core, run: &Run, serial: &str) -> Result<(bool, String), String> {
    let mut gradle = cmd::command(&core.paths, run.path.join("gradlew"));
    gradle.arg(":app:installDebug").current_dir(&run.path).env("ANDROID_SERIAL", serial);
    if std::env::var_os("JAVA_HOME").is_none() {
        gradle.env("JAVA_HOME", java_home(&core.paths));
    }
    run.stream(&mut gradle)
}

/// The activity the launcher starts the app with. Debug builds often have a second launcher icon for LeakCanary,
/// so the device can't name one: the first that isn't LeakCanary's is used.
fn launcher_activity(core: &Core, run: &Run, adb: &Path, serial: &str, app_id: &str) -> Result<String, String> {
    let query = ["query-activities", "--brief", "-a", "android.intent.action.MAIN", "-c", "android.intent.category.LAUNCHER"];
    let listed = run.capture(cmd::command(&core.paths, adb).args(["-s", serial, "shell", "cmd", "package"]).args(query).arg(app_id))?;
    // Components are the lines like "com.example.myapp/.MainActivity".
    let components = listed.lines().map(str::trim).filter(|line| line.contains('/') && !line.contains(' '));
    let components: Vec<&str> = components.collect();
    let activity = components.iter().find(|c| !c.contains("leakcanary")).or(components.first());
    activity.map(|c| c.to_string()).ok_or_else(|| format!("{app_id} has no launcher activity on {serial}"))
}

/// Installs the worktree's debug build on the phone or the emulator with this AVD and starts the app with this ID.
/// Returns the device's serial.
pub fn launch(core: &Core, run: &Run, avd: &str, app_id: &str) -> Result<String, String> {
    if app_id.is_empty() {
        return Err("this repo has no app ID yet; set it in Settings".into());
    }
    let sdk = sdk_dir(&run.path)?;
    let adb = sdk.join("platform-tools/adb");
    let serial = if run.target == "phone" { phone_serial(core, run, &adb)? } else { emulator_serial(core, run, &adb, &sdk, avd)? };
    run.say(&format!("installing the debug build on {serial}"));
    let (installed, printed) = gradle_install(core, run, &serial)?;
    if !installed {
        // A lower version code or another signing key: only a fresh install replaces the build (drops its data).
        if !["INSTALL_FAILED_VERSION_DOWNGRADE", "INSTALL_FAILED_UPDATE_INCOMPATIBLE"].iter().any(|e| printed.contains(e)) {
            return Err(format!("installDebug failed on {serial}"));
        }
        run.say(&format!("installed build is incompatible; uninstalling {app_id} from {serial}"));
        let _ = run.stream(cmd::command(&core.paths, &adb).args(["-s", &serial, "uninstall", app_id]));
        if !gradle_install(core, run, &serial)?.0 {
            return Err(format!("installDebug failed on {serial} after the uninstall"));
        }
    }
    let activity = launcher_activity(core, run, &adb, &serial, app_id)?;
    run.capture(cmd::command(&core.paths, &adb).args(["-s", &serial, "shell", "am", "start", "-W", "-n", &activity]))?;
    Ok(serial)
}
