"""Self-check for the native host with stand-ins for adb, gradlew, xcrun, xcodebuild and the Claude app. Run:
    /usr/bin/python3 native-host/test_open_pr_in_studio.py
"""
import importlib.util
import json
import os
import signal
import stat
import subprocess
import sys
import tempfile
import time
import unittest

spec = importlib.util.spec_from_file_location("host", os.path.join(os.path.dirname(os.path.abspath(__file__)), "open_pr_in_studio.py"))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)

FAKE_ADB = """#!/bin/sh
d="$(dirname "$0")"
echo "$*" >> "$d/calls.log"
case "$*" in
  "devices -l") cat "$d/devices_l.txt" ;;
  "devices") cat "$d/devices.txt" ;;
  *"emu avd name") printf '%s\\r\\nOK\\r\\n' "$(cat "$d/$2.avd" 2>/dev/null || echo Pixel_9_API_35)" ;;
  *"getprop sys.boot_completed") echo 1 ;;
  *"pm path android") echo package:/system/framework/framework-res.apk ;;
esac
"""
# Comes up as emulator-5556 running the AVD it was asked for, next to a running emulator-5554.
FAKE_EMULATOR = """#!/bin/sh
t="$(dirname "$0")/../platform-tools"
echo "$2" > "$t/emulator-5556.avd"
printf 'List of devices attached\\nemulator-5554\\tdevice\\nemulator-5556\\tdevice\\n' > "$t/devices.txt"
sleep 5
"""
# Fails like a downgrade on the first run, succeeds on the second.
FAKE_GRADLEW = """#!/bin/sh
echo "gradlew $* serial=$ANDROID_SERIAL" >> calls.log
if [ ! -f .ran ]; then touch .ran; echo 'INSTALL_FAILED_VERSION_DOWNGRADE'; exit 1; fi
echo 'BUILD SUCCESSFUL'
"""

# Logs each call; lists devices from devices.json and answers -showBuildSettings like a scheme with an app extension.
FAKE_XCRUN = """#!/bin/sh
d="$(dirname "$0")"
echo "xcrun $*" >> "$d/calls.log"
[ "$1 $2 $3 $4" = "devicectl list devices --json-output" ] && cp "$d/devices.json" "$5"
exit 0
"""
FAKE_XCODEBUILD = """#!/bin/sh
echo "xcodebuild $*" >> "$(dirname "$0")/calls.log"
case "$*" in *-showBuildSettings*) echo '[{"buildSettings": {"WRAPPER_EXTENSION": "appex"}}, {"buildSettings":
  {"WRAPPER_EXTENSION": "app", "TARGET_BUILD_DIR": "/b", "WRAPPER_NAME": "MyApp.app", "PRODUCT_BUNDLE_IDENTIFIER": "co.x"}}]' ;;
esac
"""
FAKE_OPEN = """#!/bin/sh
echo "open $*" >> "$(dirname "$0")/calls.log"
"""


def write_exe(path, text):
    with open(path, "w") as f:
        f.write(text)
    os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC)


class Fixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.tools = os.path.join(self.tmp, "sdk", "platform-tools")
        os.makedirs(self.tools)
        self.adb = os.path.join(self.tools, "adb")
        write_exe(self.adb, FAKE_ADB)

    def devices(self, long_lines, short_lines=()):
        with open(os.path.join(self.tools, "devices_l.txt"), "w") as f:
            f.write("List of devices attached\n" + "\n".join(long_lines) + "\n")
        with open(os.path.join(self.tools, "devices.txt"), "w") as f:
            f.write("List of devices attached\n" + "\n".join(short_lines) + "\n")

    def calls(self, directory):
        with open(os.path.join(directory, "calls.log")) as f:
            return f.read().splitlines()


class Validation(unittest.TestCase):
    def test_rejects_bad_input(self):
        ok = {"repo": "example/MyAndroidApp", "pr": 1, "action": "open", "branch": "feat/a"}
        for bad, error in [
            ({"repo": "evil/repo"}, "not configured"),
            ({"pr": "1; rm -rf /"}, "invalid PR"),
            ({"pr": True}, "invalid PR"),
            ({"pr": -3}, "invalid PR"),
            ({"action": "rm"}, "unknown action"),
            ({"branch": "--upload-pack=x"}, "can't check out"),
            ({"branch": "../../x"}, "can't check out"),
            ({"branch": "someone:feat/a"}, "forks"),
            ({"branch": None}, "can't check out"),
            ({"worktreeDir": "/does/not/exist"}, "does not exist"),
            ({"worktreeDir": "relative/dir"}, "does not exist"),
            ({"ide": "Notepad"}, "not installed"),
            ({"ide": "/Applications/Calculator.app"}, "not installed"),
        ]:
            with self.assertRaisesRegex(ValueError, error):
                host.handle(dict(ok, **bad))


class Update(unittest.TestCase):
    def setUp(self):
        self.calls = []
        self.orig = (host.run, host.ensure_worktree, host.update, host.start_launch, host.installed_ides, host.AVD_DIR,
                     host.REPOS, host.simulators, host.branch_deleted, host.open_claude)
        self.deleted = False
        host.branch_deleted = lambda root, branch: self.deleted
        host.open_claude = lambda path, prompt: self.calls.append("claude")
        host.run = lambda *cmd, **kw: self.calls.append(" ".join(cmd))
        host.ensure_worktree = lambda *a: "/wt"
        host.update = lambda *a: self.calls.append("update")
        host.start_launch = lambda target, path, branch, avd: self.calls.append(f"launch {avd}".strip()) or "/log"
        host.installed_ides = lambda ides=host.IDES: {"IntelliJ IDEA": "/Applications/IntelliJ IDEA CE.app"} if ides is host.IDES else {
            "Xcode": "/Applications/Xcode.app", "VS Code": "/Applications/Visual Studio Code.app"}
        host.AVD_DIR = tempfile.mkdtemp()
        open(os.path.join(host.AVD_DIR, "Medium_Phone.ini"), "w").close()
        ios_clone = tempfile.mkdtemp()
        os.makedirs(os.path.join(ios_clone, "App.xcodeproj"))
        host.REPOS = dict(host.REPOS, **{"o/ios": ios_clone})
        host.simulators = lambda: {"iPhone 17 (iOS 26.3)": "SIM-UDID"}

    def tearDown(self):
        (host.run, host.ensure_worktree, host.update, host.start_launch, host.installed_ides, host.AVD_DIR,
         host.REPOS, host.simulators, host.branch_deleted, host.open_claude) = self.orig

    def act(self, action, **msg):
        self.reply = host.handle({"repo": "example/MyAndroidApp", "pr": 1, "action": action, "branch": "feat/a",
                                  "ide": "IntelliJ IDEA", **msg})
        return self.calls

    def test_open_starts_the_chosen_ide_then_updates(self):
        self.assertEqual(self.act("open"), ["open -a /Applications/IntelliJ IDEA CE.app /wt", "update"])

    def test_ios_repo_opens_in_the_chosen_ios_ide_xcode_with_its_project(self):
        self.assertEqual(self.act("open", repo="o/ios", iosIde="Xcode"), ["open -a /Applications/Xcode.app /wt/App.xcodeproj", "update"])
        self.calls.clear()
        self.assertEqual(self.act("open", repo="o/ios", iosIde="VS Code"), ["open -a /Applications/Visual Studio Code.app /wt", "update"])
        # The Android IDE setting doesn't count for iOS repos.
        for ide in ["IntelliJ IDEA", None]:
            with self.assertRaisesRegex(ValueError, "not installed"):
                self.act("open", repo="o/ios", iosIde=ide)

    def test_simulator_run_uses_the_chosen_simulators_udid(self):
        self.assertEqual(self.act("simulator", repo="o/ios", simulator="iPhone 17 (iOS 26.3)"), ["update", "launch SIM-UDID"])

    def test_unknown_or_missing_simulator_is_rejected_before_anything_runs(self):
        for name in ["iPhone 1 (iOS 26.3)", "", None]:
            with self.assertRaisesRegex(ValueError, "not found"):
                self.act("simulator", repo="o/ios", simulator=name)
        self.assertEqual(self.calls, [])

    def test_device_run_updates_before_installing(self):
        self.assertEqual(self.act("phone"), ["update", "launch"])

    def test_emulator_run_uses_the_chosen_emulator(self):
        self.assertEqual(self.act("emulator", emulator="Medium_Phone"), ["update", "launch Medium_Phone"])

    def test_unknown_or_missing_emulator_is_rejected_before_anything_runs(self):
        for name in ["-wipe-data", "", None]:
            with self.assertRaisesRegex(ValueError, "not found"):
                self.act("emulator", emulator=name)
        self.assertEqual(self.calls, [])

    def test_deleted_branch_stops_ide_actions_before_anything_runs_but_not_claude(self):
        self.deleted = True
        for action, msg in [("open", {}), ("phone", {}), ("emulator", {"emulator": "Medium_Phone"}),
                            ("open", {"repo": "o/ios", "iosIde": "Xcode"})]:
            self.assertEqual(self.act(action, **msg), [])
            self.assertEqual(self.reply["title"], "Branch not found")
            self.assertIn("feat/a no longer exists on GitHub", self.reply["error"])
            self.assertFalse(self.reply["ok"])
        self.assertEqual(self.act("claude"), ["claude"])


class Detect(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.orig = host.APP_DIRS, host.REPOS, host.AVD_DIR, host.run
        host.AVD_DIR = os.path.join(self.tmp, "avd")
        host.APP_DIRS = [os.path.join(self.tmp, "Applications"), os.path.join(self.tmp, "home", "Applications")]
        host.REPOS = {"o/r": os.path.join(self.tmp, "clone")}
        os.makedirs(os.path.join(self.tmp, "clone"))

    def tearDown(self):
        host.APP_DIRS, host.REPOS, host.AVD_DIR, host.run = self.orig

    def test_lists_installed_ides_in_order_with_their_app(self):
        for app in ["home/Applications/IntelliJ IDEA CE.app", "Applications/Visual Studio Code.app", "Applications/Xcode.app"]:
            os.makedirs(os.path.join(self.tmp, app))
        self.assertEqual(host.installed_ides(), {
            "IntelliJ IDEA": os.path.join(self.tmp, "home/Applications/IntelliJ IDEA CE.app"),
            "VS Code": os.path.join(self.tmp, "Applications/Visual Studio Code.app"),
        })
        self.assertEqual(host.handle({"action": "detect"})["ides"], ["IntelliJ IDEA", "VS Code"])
        self.assertEqual(host.handle({"action": "detect"})["iosIdes"], ["Xcode", "VS Code"])

    def test_repo_with_an_xcode_project_at_its_root_is_ios(self):
        clone = os.path.join(self.tmp, "clone")
        self.assertEqual(host.detect("o/r")["platform"], "android")
        os.makedirs(os.path.join(clone, "App.xcodeproj"))
        self.assertEqual(host.detect("o/r")["platform"], "ios")
        self.assertEqual(host.detect("evil/repo")["platform"], "android")
        os.makedirs(os.path.join(clone, "App.xcworkspace"))
        self.assertEqual(host.xcode_project(clone), os.path.join(clone, "App.xcworkspace"))

    def test_simulators_newest_ios_first_without_other_platforms(self):
        host.run = lambda *cmd, **kw: json.dumps({"devices": {
            "com.apple.CoreSimulator.SimRuntime.iOS-18-6": [{"name": "iPhone 16", "udid": "A"}],
            "com.apple.CoreSimulator.SimRuntime.watchOS-11-0": [{"name": "Apple Watch", "udid": "W"}],
            "com.apple.CoreSimulator.SimRuntime.iOS-26-3": [{"name": "iPhone 17", "udid": "B"}, {"name": "iPad (A16)", "udid": "C"}],
            "com.apple.CoreSimulator.SimRuntime.iOS-9-3": [{"name": "iPhone 6s", "udid": "D"}]}})
        self.assertEqual(list(host.simulators().items()), [
            ("iPhone 17 (iOS 26.3)", "B"), ("iPad (A16) (iOS 26.3)", "C"), ("iPhone 16 (iOS 18.6)", "A"), ("iPhone 6s (iOS 9.3)", "D")])

        def no_xcode(*cmd, **kw):
            raise RuntimeError("xcrun simctl list failed")
        host.run = no_xcode
        self.assertEqual(host.simulators(), {})

    def test_lists_emulators_by_their_ini_files(self):
        self.assertEqual(host.detect(None)["avds"], [])  # no AVD folder
        for name in ["Pixel_9_API_35.ini", "Pixel_9_API_35.avd/config.ini", "Medium_Phone_API_36.1.ini", "gradle-managed/x"]:
            os.makedirs(os.path.dirname(os.path.join(host.AVD_DIR, name)), exist_ok=True)
            open(os.path.join(host.AVD_DIR, name), "w").close()
        self.assertEqual(host.handle({"action": "detect"})["avds"], ["Medium_Phone_API_36.1", "Pixel_9_API_35"])

    def test_adb_only_when_the_repos_sdk_has_it(self):
        self.assertFalse(host.detect("o/r")["adb"])  # no local.properties
        with open(os.path.join(self.tmp, "clone", "local.properties"), "w") as f:
            f.write(f"sdk.dir={os.path.join(self.tmp, 'sdk')}\n")
        self.assertFalse(host.detect("o/r")["adb"])  # sdk without platform-tools
        os.makedirs(os.path.join(self.tmp, "sdk", "platform-tools"))
        write_exe(os.path.join(self.tmp, "sdk", "platform-tools", "adb"), "#!/bin/sh\n")
        self.assertTrue(host.detect("o/r")["adb"])
        self.assertFalse(host.detect("evil/repo")["adb"])
        self.assertFalse(host.detect(None)["adb"])


class Devices(Fixture):
    def test_phone_prefers_pixel_10_then_6_then_any(self):
        other = "R5CT device usb:1 model:SM_S911B"
        p6 = "1A2B device usb:2 model:Pixel_6"
        p10 = "adb-12345-tls device product:blazer model:Pixel_10_Pro transport_id:4"
        emu = "emulator-5554 device product:sdk model:sdk_gphone64"
        offline = "XYZ offline usb:3 model:Pixel_10"
        self.devices([emu, other, p6, offline, p10])
        self.assertEqual(host.phone_serial(self.adb), "adb-12345-tls")
        self.devices([emu, other, p6])
        self.assertEqual(host.phone_serial(self.adb), "1A2B")
        self.devices([other, emu])
        self.assertEqual(host.phone_serial(self.adb), "R5CT")
        self.devices([emu, offline])
        with self.assertRaisesRegex(RuntimeError, "no phone"):
            host.phone_serial(self.adb)

    def test_running_emulator_is_reused(self):
        # No emulator binary in the fake SDK: booting one would fail, so this passes only without a boot.
        self.devices([], ["emulator-5556\tdevice"])
        self.assertEqual(host.emulator_serial(self.adb, os.path.join(self.tmp, "sdk"), "Pixel_9_API_35"), "emulator-5556")

    def test_chosen_emulator_is_booted_when_another_one_runs(self):
        self.addCleanup(setattr, host, "LOG_DIR", host.LOG_DIR)
        host.LOG_DIR = self.tmp
        os.makedirs(os.path.join(self.tmp, "sdk", "emulator"))
        write_exe(os.path.join(self.tmp, "sdk", "emulator", "emulator"), FAKE_EMULATOR)
        self.devices([], ["emulator-5554\tdevice"])  # running Pixel_9_API_35
        self.assertEqual(host.emulator_serial(self.adb, os.path.join(self.tmp, "sdk"), "Medium_Phone"), "emulator-5556")
        self.assertTrue(os.path.isfile(os.path.join(self.tmp, "Medium_Phone.log")))


class Launch(Fixture):
    def test_downgrade_uninstalls_and_retries_then_starts(self):
        worktree = os.path.join(self.tmp, "worktree")
        os.makedirs(worktree)
        with open(os.path.join(worktree, "local.properties"), "w") as f:
            f.write(f"sdk.dir={os.path.join(self.tmp, 'sdk')}\n")
        write_exe(os.path.join(worktree, "gradlew"), FAKE_GRADLEW)
        self.devices(["PHONE1 device usb:1 model:Pixel_6"])

        self.assertEqual(host.launch("phone", worktree, ""), "PHONE1")
        self.assertEqual(self.calls(worktree), ["gradlew :app:installDebug serial=PHONE1"] * 2)
        adb_calls = self.calls(self.tools)
        self.assertIn(f"-s PHONE1 uninstall {host.APP_ID}", adb_calls)
        self.assertEqual(adb_calls[-1], f"-s PHONE1 shell am start -W -n {host.START_ACTIVITY}")

    def test_other_install_failure_does_not_uninstall(self):
        worktree = os.path.join(self.tmp, "worktree")
        os.makedirs(worktree)
        with open(os.path.join(worktree, "local.properties"), "w") as f:
            f.write(f"sdk.dir={os.path.join(self.tmp, 'sdk')}\n")
        write_exe(os.path.join(worktree, "gradlew"), "#!/bin/sh\necho 'compilation failed'; exit 1\n")
        self.devices(["PHONE1 device usb:1 model:Pixel_6"])
        with self.assertRaisesRegex(RuntimeError, "installDebug failed"):
            host.launch("phone", worktree, "")
        self.assertFalse(any("uninstall" in c for c in self.calls(self.tools)))


class IosLaunch(unittest.TestCase):
    def setUp(self):
        self.bin = tempfile.mkdtemp()
        for name, text in [("xcrun", FAKE_XCRUN), ("xcodebuild", FAKE_XCODEBUILD), ("open", FAKE_OPEN)]:
            write_exe(os.path.join(self.bin, name), text)
        self.addCleanup(os.environ.__setitem__, "PATH", os.environ["PATH"])
        os.environ["PATH"] = self.bin + ":" + os.environ["PATH"]
        self.addCleanup(setattr, host, "LOG_DIR", host.LOG_DIR)
        host.LOG_DIR = self.bin
        self.worktree = tempfile.mkdtemp()
        os.makedirs(os.path.join(self.worktree, "App.xcodeproj"))
        self.build = (f"xcodebuild -project {os.path.join(self.worktree, 'App.xcodeproj')} -scheme {host.IOS_SCHEME} "
                      "-configuration Debug -destination id=")

    def calls(self):
        with open(os.path.join(self.bin, "calls.log")) as f:
            return f.read().splitlines()

    def devices(self, *devices):
        with open(os.path.join(self.bin, "devices.json"), "w") as f:
            json.dump({"result": {"devices": [
                {"hardwareProperties": {"udid": udid, "deviceType": kind, "reality": "physical"},
                 "connectionProperties": {"pairingState": "paired", "tunnelState": tunnel}} for udid, kind, tunnel in devices]}}, f)

    def test_iphone_gets_a_signed_build_on_a_connected_iphone(self):
        self.devices(("GONE", "iPhone", "unavailable"), ("IPAD", "iPad", "connected"), ("IDLE", "iPhone", "disconnected"),
                     ("USB", "iPhone", "connected"))
        self.assertEqual(host.ios_launch("iphone", self.worktree, ""), "USB")
        self.assertEqual(self.calls()[1:], [
            f"{self.build}USB -allowProvisioningUpdates build", f"{self.build}USB -showBuildSettings -json",
            "xcrun devicectl device install app --device USB /b/MyApp.app",
            "xcrun devicectl device process launch --terminate-existing --device USB co.x"])

    def test_no_reachable_iphone(self):
        self.devices(("GONE", "iPhone", "unavailable"), ("IPAD", "iPad", "connected"))
        with self.assertRaisesRegex(RuntimeError, "no iPhone"):
            host.ios_launch("iphone", self.worktree, "")

    def test_simulator_is_booted_and_shown_then_gets_the_build(self):
        self.assertEqual(host.ios_launch("simulator", self.worktree, "SIM"), "SIM")
        self.assertEqual(self.calls(), [
            "xcrun simctl bootstatus SIM -b", "open -a Simulator",
            f"{self.build}SIM build", f"{self.build}SIM -showBuildSettings -json",
            "xcrun simctl install SIM /b/MyApp.app", "xcrun simctl launch --terminate-running-process SIM co.x"])


class Worktrees(unittest.TestCase):
    def test_existing_worktree_gets_missing_local_properties_but_keeps_its_own(self):
        tmp = tempfile.mkdtemp()
        root = os.path.join(tmp, "App")
        for cmd in (["git", "init", "-q", "-b", "main", root],
                    ["git", "-C", root, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"],
                    ["git", "-C", root, "worktree", "add", "-q", "-b", "feat/a", os.path.join(tmp, "elsewhere")],
                    ["git", "-C", root, "worktree", "add", "-q", "-b", "feat/b", os.path.join(tmp, "own")]):
            subprocess.run(cmd, check=True, capture_output=True)
        with open(os.path.join(root, "local.properties"), "w") as f:
            f.write("sdk.dir=/main\n")
        with open(os.path.join(tmp, "own", "local.properties"), "w") as f:
            f.write("sdk.dir=/own\n")

        path = host.ensure_worktree(root, "feat/a", None)
        self.assertEqual(path, os.path.realpath(os.path.join(tmp, "elsewhere")))
        with open(os.path.join(path, "local.properties")) as f:
            self.assertEqual(f.read(), "sdk.dir=/main\n")
        path = host.ensure_worktree(root, "feat/b", None)
        with open(os.path.join(path, "local.properties")) as f:
            self.assertEqual(f.read(), "sdk.dir=/own\n")

    def test_new_worktree_tracks_origin_and_updates_fast_forward_only(self):
        tmp = tempfile.mkdtemp()
        origin, root, pusher = (os.path.join(tmp, n) for n in ("origin.git", "App", "pusher"))

        def git(*args):
            return subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *args],
                                  check=True, capture_output=True, text=True).stdout.strip()

        def push_commit():  # someone pushes a new commit to the PR's branch
            git("-C", pusher, "commit", "-q", "--allow-empty", "-m", "x")
            git("-C", pusher, "push", "-q", "origin", "HEAD:feat/a")
            return git("-C", pusher, "rev-parse", "HEAD")

        git("init", "-q", "--bare", origin)
        git("clone", "-q", origin, pusher)
        git("-C", pusher, "commit", "-q", "--allow-empty", "-m", "base")
        git("-C", pusher, "push", "-q", "origin", "HEAD:main")
        git("clone", "-q", "-b", "main", origin, root)
        head = push_commit()  # the main clone has never fetched feat/a

        path = host.ensure_worktree(root, "feat/a", tmp)
        self.assertEqual(path, os.path.realpath(os.path.join(tmp, "App-feat-a")))
        self.assertEqual(git("-C", path, "rev-parse", "--abbrev-ref", "HEAD", "@{upstream}"), "feat/a\norigin/feat/a")
        self.assertEqual(git("-C", path, "rev-parse", "HEAD"), head)

        head = push_commit()
        host.update(path, "feat/a")
        self.assertEqual(git("-C", path, "rev-parse", "HEAD"), head)

        git("-C", path, "commit", "-q", "--allow-empty", "-m", "local")
        local = git("-C", path, "rev-parse", "HEAD")
        push_commit()
        with self.assertRaisesRegex(RuntimeError, "git -C"):
            host.update(path, "feat/a")
        self.assertEqual(git("-C", path, "rev-parse", "HEAD"), local)  # the local commit is kept

    def test_branch_deleted_only_when_origin_says_so(self):
        tmp = tempfile.mkdtemp()
        origin, root = os.path.join(tmp, "origin.git"), os.path.join(tmp, "App")
        for cmd in (["git", "init", "-q", "--bare", "-b", "main", origin], ["git", "clone", "-q", origin, root],
                    ["git", "-C", root, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"],
                    ["git", "-C", root, "push", "-q", "origin", "HEAD:feat/a"]):
            subprocess.run(cmd, check=True, capture_output=True)
        self.assertFalse(host.branch_deleted(root, "feat/a"))
        self.assertTrue(host.branch_deleted(root, "feat/gone"))
        self.assertTrue(host.branch_deleted(root, "a"))  # no suffix match on feat/a
        subprocess.run(["git", "-C", root, "remote", "set-url", "origin", os.path.join(tmp, "unreachable.git")], check=True)
        self.assertFalse(host.branch_deleted(root, "feat/gone"))  # offline: carry on as before


class Cancel(unittest.TestCase):
    def setUp(self):
        self.orig = host.LOG_DIR
        host.LOG_DIR = tempfile.mkdtemp()

    def tearDown(self):
        host.LOG_DIR = self.orig

    def spawn(self, *args, log=""):
        p = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)", *args], start_new_session=True)
        with open(os.path.join(host.LOG_DIR, "phone.pid"), "w") as f:
            json.dump({"pid": p.pid, "log": log}, f)
        return p

    def test_new_run_kills_the_previous_one(self):
        p = self.spawn("--launch", "phone", "open_pr_in_studio.py")
        host.cancel_previous("phone")
        self.assertEqual(p.wait(timeout=5), -signal.SIGTERM)

    def test_reused_pid_of_another_process_is_left_alone(self):
        p = self.spawn("unrelated")
        host.cancel_previous("phone")
        time.sleep(0.3)
        self.assertIsNone(p.poll())
        p.kill()

    def test_no_previous_run(self):
        host.cancel_previous("emulator")  # no pid file: nothing to do

    def test_cancelled_run_writes_marker(self):
        tmp = tempfile.mkdtemp()
        tools = os.path.join(tmp, "sdk", "platform-tools")
        os.makedirs(tools)
        write_exe(os.path.join(tools, "adb"), '#!/bin/sh\n[ "$*" = "devices -l" ] && printf "List\\nP1 device model:Pixel_6\\n"\n')
        worktree = os.path.join(tmp, "wt")
        os.makedirs(worktree)
        with open(os.path.join(worktree, "local.properties"), "w") as f:
            f.write(f"sdk.dir={os.path.join(tmp, 'sdk')}\n")
        write_exe(os.path.join(worktree, "gradlew"), "#!/bin/sh\nsleep 30\n")  # a long build

        log = host.start_launch("phone", worktree, "feat/a", "")
        time.sleep(1.5)
        self.assertEqual(host.run_status(log)["state"], "running")
        host.cancel_previous("phone")
        deadline = time.time() + 5
        while host.run_status(log)["state"] == "running" and time.time() < deadline:
            time.sleep(0.2)
        self.assertEqual(host.run_status(log)["state"], "cancelled")


class Status(unittest.TestCase):
    def setUp(self):
        self.orig = host.LOG_DIR
        host.LOG_DIR = tempfile.mkdtemp()

    def tearDown(self):
        host.LOG_DIR = self.orig

    def status(self, *lines, name="feat-a-phone.log"):
        log = os.path.join(host.LOG_DIR, name)
        with open(log, "w") as f:
            f.write("\n".join(lines) + "\n\n")
        return host.run_status(log)

    def test_states_from_last_line(self):
        self.assertEqual(self.status("installing", "BUILD SUCCESSFUL", "started on P1")["state"], "started")
        self.assertEqual(self.status("installing", "FAILED: no phone connected"), {"ok": True, "state": "failed", "message": "no phone connected"})
        self.assertEqual(self.status("installing", "CANCELLED")["state"], "cancelled")
        # No final line and no live run behind it: it died without saying why.
        self.assertEqual(self.status("installing")["state"], "failed")

    def test_only_logs_in_log_dir(self):
        for bad in ["/etc/passwd", os.path.join(host.LOG_DIR, "..", "x-phone.log"), None, 3]:
            with self.assertRaisesRegex(ValueError, "unknown run"):
                host.run_status(bad)
        with self.assertRaisesRegex(ValueError, "unknown run"):
            self.status("x", name="notes.txt")


class Claude(unittest.TestCase):
    def setUp(self):
        self.sessions = tempfile.mkdtemp()
        self.opened = []
        self.orig = host.run, host.SESSIONS_DIR
        host.run = lambda *cmd, **kw: self.opened.append(cmd)
        host.SESSIONS_DIR = self.sessions

    def tearDown(self):
        host.run, host.SESSIONS_DIR = self.orig

    def session(self, name, **fields):
        os.makedirs(os.path.join(self.sessions, "acct"), exist_ok=True)
        with open(os.path.join(self.sessions, "acct", f"local_{name}.json"), "w") as f:
            json.dump(fields, f)

    def test_reopens_newest_unarchived_session_in_folder(self):
        self.session("a", sessionId="local_old", cwd="/w/x", isArchived=False, lastActivityAt=1)
        self.session("b", sessionId="local_new", cwd="/w/x", isArchived=False, lastActivityAt=5)
        self.session("c", sessionId="local_arch", cwd="/w/x", isArchived=True, lastActivityAt=9)
        self.session("d", sessionId="local_other", cwd="/w/y", isArchived=False, lastActivityAt=9)
        host.open_claude("/w/x", "ignored")
        self.assertEqual(self.opened, [("open", "claude://code/continue?session=local_new")])

    def test_new_session_composer_when_none(self):
        host.open_claude("/w/my repo", "Fix login & logout (#12): ")
        self.assertEqual(self.opened, [("open", "claude://code/new?folder=%2Fw%2Fmy%20repo&q=Fix%20login%20%26%20logout%20%28%2312%29%3A%20")])


if __name__ == "__main__":
    unittest.main()
