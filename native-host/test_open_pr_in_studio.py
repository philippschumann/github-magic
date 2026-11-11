"""Self-check for the native host with stand-ins for adb, gradlew and the Claude app. Run:
    /usr/bin/python3 native-host/test_open_pr_in_studio.py
"""
import importlib.util
import json
import os
import stat
import tempfile
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
  *"emu avd name") printf 'Pixel_9_API_35\\r\\nOK\\r\\n' ;;
  *"getprop sys.boot_completed") echo 1 ;;
  *"pm path android") echo package:/system/framework/framework-res.apk ;;
esac
"""
# Fails like a downgrade on the first run, succeeds on the second.
FAKE_GRADLEW = """#!/bin/sh
echo "gradlew $* serial=$ANDROID_SERIAL" >> calls.log
if [ ! -f .ran ]; then touch .ran; echo 'INSTALL_FAILED_VERSION_DOWNGRADE'; exit 1; fi
echo 'BUILD SUCCESSFUL'
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
        ok = {"repo": "example/MyAndroidApp", "pr": 1, "action": "open"}
        for bad, error in [
            ({"repo": "evil/repo"}, "not configured"),
            ({"pr": "1; rm -rf /"}, "invalid PR"),
            ({"pr": True}, "invalid PR"),
            ({"pr": -3}, "invalid PR"),
            ({"action": "rm"}, "unknown action"),
            ({"worktreeDir": "/does/not/exist"}, "does not exist"),
            ({"worktreeDir": "relative/dir"}, "does not exist"),
        ]:
            with self.assertRaisesRegex(ValueError, error):
                host.handle(dict(ok, **bad))


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
        self.assertEqual(host.emulator_serial(self.adb, os.path.join(self.tmp, "sdk")), "emulator-5556")


class Launch(Fixture):
    def test_downgrade_uninstalls_and_retries_then_starts(self):
        worktree = os.path.join(self.tmp, "worktree")
        os.makedirs(worktree)
        with open(os.path.join(worktree, "local.properties"), "w") as f:
            f.write(f"sdk.dir={os.path.join(self.tmp, 'sdk')}\n")
        write_exe(os.path.join(worktree, "gradlew"), FAKE_GRADLEW)
        self.devices(["PHONE1 device usb:1 model:Pixel_6"])

        self.assertEqual(host.launch("phone", worktree), "PHONE1")
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
            host.launch("phone", worktree)
        self.assertFalse(any("uninstall" in c for c in self.calls(self.tools)))


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
