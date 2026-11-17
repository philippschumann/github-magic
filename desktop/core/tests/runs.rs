mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use common::{calls, fixture, paths, tmp, write};
use github_magic_core::runs::{self, Run, State};
use github_magic_core::settings::Config;
use github_magic_core::{Core, android};

/// A worktree with an Android SDK whose adb is the stand-in.
struct Fixture {
    tmp: PathBuf,
    core: Arc<Core>,
    adb: PathBuf,
    worktree: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tmp();
        let adb = fixture("adb", &tmp.join("sdk/platform-tools"));
        let worktree = tmp.join("worktree");
        write(worktree.join("local.properties"), &format!("sdk.dir={}\n", tmp.join("sdk").display()));
        write(tmp.join("sdk/platform-tools/activities.txt"), ACTIVITIES);
        let config = Config {
            app_id: "com.example.myapp".into(),
            phone_models: vec!["model:Pixel_10".into(), "model:Pixel_6".into()],
            ..Config::default()
        };
        Self { core: Arc::new(Core::new(config, paths(&tmp))), tmp, adb, worktree }
    }

    /// What `adb devices -l` and `adb devices` list.
    fn devices(&self, long_lines: &[&str], short_lines: &[&str]) {
        let tools = self.tmp.join("sdk/platform-tools");
        write(tools.join("devices_l.txt"), &format!("List of devices attached\n{}\n", long_lines.join("\n")));
        write(tools.join("devices.txt"), &format!("List of devices attached\n{}\n", short_lines.join("\n")));
    }

    /// A run that isn't going, for calling its steps one by one.
    fn run(&self, target: &'static str) -> Run {
        Run::new(&self.core.paths.log_dir, target, 1, "feat/a", &self.worktree, "").unwrap()
    }

    fn start(&self, branch: &str) -> Arc<Run> {
        runs::start(&self.core, "phone", 1, branch, &self.worktree, "", "phone").unwrap()
    }

    fn adb_calls(&self) -> Vec<String> {
        calls(&self.tmp.join("sdk/platform-tools"))
    }
}

/// What the device lists for an app with LeakCanary's launcher icon next to its own.
const ACTIVITIES: &str = "2 activities found:
  Activity #0:
    priority=0 preferredOrder=0 match=0x108000 specificIndex=-1 isDefault=false
    com.example.myapp/leakcanary.internal.activity.LeakLauncherActivity
  Activity #1:
    priority=0 preferredOrder=0 match=0x108000 specificIndex=-1 isDefault=false
    com.example.myapp/.MainActivity
";

fn finished(run: &Run) -> State {
    let deadline = Instant::now() + Duration::from_secs(10);
    while run.is_running() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    run.state().0
}

#[test]
fn phone_prefers_pixel_10_then_6_then_any() {
    let f = Fixture::new();
    let run = f.run("phone");
    let other = "R5CT device usb:1 model:SM_S911B";
    let p6 = "1A2B device usb:2 model:Pixel_6";
    let p10 = "adb-12345-tls device product:blazer model:Pixel_10_Pro transport_id:4";
    let emu = "emulator-5554 device product:sdk model:sdk_gphone64";
    let offline = "XYZ offline usb:3 model:Pixel_10";
    f.devices(&[emu, other, p6, offline, p10], &[]);
    assert_eq!(android::phone_serial(&f.core, &run, &f.adb).unwrap(), "adb-12345-tls");
    assert_eq!(run.device(), "Pixel 10 Pro");
    f.devices(&[emu, other, p6], &[]);
    assert_eq!(android::phone_serial(&f.core, &run, &f.adb).unwrap(), "1A2B");
    f.devices(&[other, emu], &[]);
    assert_eq!(android::phone_serial(&f.core, &run, &f.adb).unwrap(), "R5CT");
    f.devices(&[emu, offline], &[]);
    assert!(android::phone_serial(&f.core, &run, &f.adb).unwrap_err().contains("no phone"));
}

#[test]
fn running_emulator_is_reused() {
    // No emulator binary in the stand-in SDK: booting one would fail, so this passes only without a boot.
    let f = Fixture::new();
    f.devices(&[], &["emulator-5556\tdevice"]);
    let serial = android::emulator_serial(&f.core, &f.run("emulator"), &f.adb, &f.tmp.join("sdk"), "Pixel_9_API_35");
    assert_eq!(serial.unwrap(), "emulator-5556");
}

#[test]
fn chosen_emulator_is_booted_when_another_one_runs() {
    let f = Fixture::new();
    fixture("emulator", &f.tmp.join("sdk/emulator"));
    f.devices(&[], &["emulator-5554\tdevice"]); // running Pixel_9_API_35
    let serial = android::emulator_serial(&f.core, &f.run("emulator"), &f.adb, &f.tmp.join("sdk"), "Medium_Phone");
    assert_eq!(serial.unwrap(), "emulator-5556");
    assert!(f.core.paths.log_dir.join("Medium_Phone.log").is_file());
}

#[test]
fn downgrade_uninstalls_and_retries_then_starts_the_launcher_activity() {
    let f = Fixture::new();
    fixture("gradlew", &f.worktree);
    f.devices(&["PHONE1 device usb:1 model:Pixel_6"], &[]);
    let mut events = f.core.runs.subscribe();

    let run = f.start("feat/a");
    assert_eq!(finished(&run), State::Started("PHONE1".into()));
    assert_eq!(run.status()["message"], "started on PHONE1");
    assert_eq!(calls(&f.worktree), ["gradlew :app:installDebug serial=PHONE1"; 2]);
    let adb_calls = f.adb_calls();
    assert!(adb_calls.contains(&"-s PHONE1 uninstall com.example.myapp".to_owned()), "{adb_calls:?}");
    // Not LeakCanary's launcher icon.
    assert_eq!(adb_calls.last().unwrap(), "-s PHONE1 shell am start -W -n com.example.myapp/.MainActivity");
    // The tray menu and the notification hear of the start and the end.
    assert!(Arc::ptr_eq(&events.try_recv().unwrap(), &run) && Arc::ptr_eq(&events.try_recv().unwrap(), &run));
    // The log is for people to read.
    let log = std::fs::read_to_string(&run.log).unwrap();
    assert!(log.contains("INSTALL_FAILED_VERSION_DOWNGRADE") && log.ends_with("BUILD SUCCESSFUL\nstarted on PHONE1\n"), "{log}");
}

#[test]
fn other_install_failure_does_not_uninstall() {
    let f = Fixture::new();
    write(f.worktree.join("gradlew"), "#!/bin/sh\necho 'compilation failed'; exit 1\n");
    make_executable(&f.worktree.join("gradlew"));
    f.devices(&["PHONE1 device usb:1 model:Pixel_6"], &[]);
    assert_eq!(finished(&f.start("feat/a")), State::Failed("installDebug failed on PHONE1".into()));
    assert!(!f.adb_calls().iter().any(|call| call.contains("uninstall")));
}

#[test]
fn app_without_a_launcher_activity_fails() {
    let f = Fixture::new();
    write(f.worktree.join("gradlew"), "#!/bin/sh\n");
    make_executable(&f.worktree.join("gradlew"));
    write(f.tmp.join("sdk/platform-tools/activities.txt"), "No activities found\n");
    f.devices(&["PHONE1 device usb:1 model:Pixel_6"], &[]);
    assert_eq!(finished(&f.start("feat/a")), State::Failed("com.example.myapp has no launcher activity on PHONE1".into()));
}

/// A build that takes long, on a connected phone.
fn slow_build(f: &Fixture) {
    write(f.worktree.join("gradlew"), "#!/bin/sh\nsleep 30\n");
    make_executable(&f.worktree.join("gradlew"));
    f.devices(&["P1 device model:Pixel_6"], &[]);
}

#[test]
fn new_run_cancels_the_one_going_on_its_target() {
    let f = Fixture::new();
    slow_build(&f);
    let first = f.start("feat/a");
    thread::sleep(Duration::from_millis(500));
    assert_eq!(first.status()["state"], "running");
    let second = f.start("feat/b");
    assert_eq!(first.status()["state"], "cancelled"); // at once: the new run waited for it to stop
    assert_eq!(second.status()["state"], "running");
    assert_eq!(f.core.runs.latest().len(), 1);
    f.core.runs.cancel_all();
    assert_eq!(finished(&second), State::Cancelled);
}

#[test]
fn cancelled_run_stops_its_build_and_says_so() {
    let f = Fixture::new();
    slow_build(&f);
    let run = f.start("feat/a");
    thread::sleep(Duration::from_millis(500));
    run.cancel();
    assert_eq!(finished(&run), State::Cancelled);
    assert!(std::fs::read_to_string(&run.log).unwrap().ends_with("CANCELLED\n"));
    assert_eq!(f.core.runs.by_log(&run.log).unwrap().status()["state"], "cancelled");
}

fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}
