#!/usr/bin/python3
"""Native messaging host: checks out a GitHub PR in a sibling git worktree, then opens it in Android Studio,
runs it on a phone or emulator, or opens a Claude desktop session there.

Receives {"repo": "owner/name", "pr": 123, "action": "open|phone|emulator|claude"} from the browser extension,
replies {"ok": true, "path": ...} or {"ok": false, "error": ...}. Only repos listed in REPOS are accepted.
Phone and emulator runs take minutes: they continue in a background process (this script with --launch)
that reports through a macOS notification and logs to LOG_DIR.
"""
import json
import os
import re
import shutil
import signal
import struct
import subprocess
import sys
import time
from urllib.parse import quote

# GitHub repo -> local main clone. Worktrees are created next to it as <clone>-<branch>.
REPOS = {
    "example/MyAndroidApp": "~/code/MyAndroidApp",
}
# gitignored files a fresh worktree needs to build.
COPY_FILES = ["local.properties"]
OPEN = ["open", "-a", "Android Studio"]
ACTIONS = ("open", "phone", "emulator", "claude")

# Device runs.
APP_ID = "com.example.myapp"
START_ACTIVITY = APP_ID + "/com.example.myapp.MainActivity"
PHONE_MODELS = ["model:Pixel_10", "model:Pixel_6"]  # preferred in this order, else any other phone
AVD = "Pixel_9_API_35"
BOOT_TIMEOUT = 300
LOG_DIR = os.path.expanduser("~/Library/Logs/github-pr-quick-filters")
SESSIONS_DIR = os.path.expanduser("~/Library/Application Support/Claude/claude-code-sessions")

# Browsers start the host with a minimal PATH and no JAVA_HOME; gh and git come from Homebrew.
os.environ["PATH"] = "/opt/homebrew/bin:/usr/local/bin:" + os.environ.get("PATH", "/usr/bin:/bin")


def run(*cmd, cwd=None, env=None):
    p = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True, stdin=subprocess.DEVNULL)
    if p.returncode != 0:
        raise RuntimeError(f"{' '.join(cmd[:3])} failed: {(p.stderr or p.stdout).strip()}")
    return p.stdout.strip()


def worktree_for_branch(root, branch):
    path = None
    for line in run("git", "-C", root, "worktree", "list", "--porcelain").splitlines():
        if line.startswith("worktree "):
            path = line[len("worktree "):]
        elif line == f"branch refs/heads/{branch}":
            return path
    return None


def ensure_worktree(root, repo, pr, branch, base_dir):
    # Branch already checked out (main clone or another worktree): use that instead of a second copy.
    path = worktree_for_branch(root, branch)
    if not path:
        name = f"{os.path.basename(root)}-{branch.replace('/', '-')}"
        path = os.path.join(base_dir, name) if base_dir else os.path.join(os.path.dirname(root), name)
        if not os.path.exists(path):
            run("git", "-C", root, "worktree", "add", "--detach", path)
            try:
                run("gh", "pr", "checkout", str(pr), "--repo", repo, cwd=path)
            except Exception:
                run("git", "-C", root, "worktree", "remove", "--force", path)
                raise
    # Worktrees made elsewhere (by hand) can lack these too; an existing copy is never overwritten.
    for name in COPY_FILES:
        src, dst = os.path.join(root, name), os.path.join(path, name)
        if os.path.exists(src) and not os.path.exists(dst):
            shutil.copy2(src, dst)
    # Android Studio and the Claude app match folders by resolved path; same path = same window/session.
    return os.path.realpath(path)


def open_claude(path, prompt):
    """Reopen the newest unarchived Claude desktop session in this folder, else a new session's composer there."""
    best = None
    for dirpath, _, files in os.walk(SESSIONS_DIR):
        for name in files:
            if not (name.startswith("local_") and name.endswith(".json")):
                continue
            try:
                with open(os.path.join(dirpath, name)) as f:
                    s = json.load(f)
            except (OSError, ValueError):
                continue
            if s.get("cwd") == path and not s.get("isArchived") and s.get("sessionId"):
                if best is None or (s.get("lastActivityAt") or 0) > best[0]:
                    best = (s.get("lastActivityAt") or 0, s["sessionId"])
    if best:
        url = f"claude://code/continue?session={quote(best[1], safe='')}"
    else:
        url = f"claude://code/new?folder={quote(path, safe='')}&q={quote(prompt, safe='')}"
    run("open", url)


def cancel_previous(target):
    """Stop the run still going on this target (any PR), so a new click starts fresh."""
    try:
        with open(os.path.join(LOG_DIR, f"{target}.pid")) as f:
            pid = int(f.read())
    except (OSError, ValueError):
        return
    # The pid may have been reused by an unrelated process since; only touch one of our own runs.
    cmd = subprocess.run(["ps", "-o", "command=", "-p", str(pid)], capture_output=True, text=True).stdout
    if "--launch" in cmd and os.path.basename(__file__) in cmd:
        try:
            # Each run leads its own process group: this ends the gradle client too, and its daemon cancels the build.
            os.killpg(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass


def start_launch(target, path, branch):
    os.makedirs(LOG_DIR, exist_ok=True)
    cancel_previous(target)
    log = os.path.join(LOG_DIR, f"{branch.replace('/', '-')}-{target}.log")
    with open(log, "w") as out:
        # New session: the browser may kill the host's process group once it has replied.
        proc = subprocess.Popen([sys.executable, os.path.abspath(__file__), "--launch", target, path],
                                stdin=subprocess.DEVNULL, stdout=out, stderr=subprocess.STDOUT, start_new_session=True)
    with open(os.path.join(LOG_DIR, f"{target}.pid"), "w") as f:
        f.write(str(proc.pid))
    return log


def choose_folder(current):
    """Native macOS folder dialog: a browser's own file picker never reveals the absolute path."""
    script = ["-e", "on run argv", "-e", "activate"]
    if isinstance(current, str) and os.path.isdir(os.path.expanduser(current)):
        script += ["-e", "return POSIX path of (choose folder with prompt \"Worktree folder\" "
                         "default location (POSIX file (item 1 of argv)))"]
    else:
        script += ["-e", "return POSIX path of (choose folder with prompt \"Worktree folder\")"]
    p = subprocess.run(["osascript", *script, "-e", "end run", os.path.expanduser(current or "")],
                       capture_output=True, text=True, stdin=subprocess.DEVNULL)
    if p.returncode != 0:
        if "-128" in p.stderr:  # user pressed Cancel
            return {"ok": False, "cancelled": True}
        raise RuntimeError(f"folder dialog failed: {p.stderr.strip()}")
    return {"ok": True, "path": p.stdout.strip().rstrip("/") or "/"}


def worktree_dir(value):
    if not value:
        return None  # default: next to the main clone
    path = os.path.expanduser(value) if isinstance(value, str) else ""
    if not os.path.isabs(path) or not os.path.isdir(path):
        raise ValueError(f"worktree folder {value!r} does not exist; change it in the extension's settings")
    return path


def handle(msg):
    repo, pr, action = msg.get("repo"), msg.get("pr"), msg.get("action", "open")
    if action == "choose_folder":
        return choose_folder(msg.get("current"))
    if repo not in REPOS:
        raise ValueError(f"{repo} is not configured in {os.path.abspath(__file__)}")
    if not isinstance(pr, int) or isinstance(pr, bool) or pr <= 0:
        raise ValueError(f"invalid PR number: {pr!r}")
    if action not in ACTIONS:
        raise ValueError(f"unknown action: {action!r}")
    root = os.path.expanduser(REPOS[repo])
    base_dir = worktree_dir(msg.get("worktreeDir"))

    info = json.loads(run("gh", "pr", "view", str(pr), "--repo", repo, "--json", "headRefName,title"))
    path = ensure_worktree(root, repo, pr, info["headRefName"], base_dir)
    if action == "open":
        run(*OPEN, path)
    elif action == "claude":
        open_claude(path, f"{info['title']} (#{pr}): ")
    else:
        return {"ok": True, "path": path, "log": start_launch(action, path, info["headRefName"])}
    return {"ok": True, "path": path}


# ---- background device run (--launch phone|emulator <worktree>) ----

def sdk_dir(path):
    with open(os.path.join(path, "local.properties")) as f:
        for line in f:
            if line.startswith("sdk.dir="):
                return line[len("sdk.dir="):].strip()
    raise RuntimeError(f"no sdk.dir in {path}/local.properties")


def phone_serial(adb):
    phones = [line for line in run(adb, "devices", "-l").splitlines()[1:]
              if len(line.split()) > 1 and line.split()[1] == "device" and not line.startswith("emulator-")]
    for model in PHONE_MODELS + [""]:
        for line in phones:
            if model in line:
                return line.split()[0]
    raise RuntimeError("no phone connected; plug one in or pair it over Wi-Fi")


def avd_serial(adb):
    for line in run(adb, "devices").splitlines()[1:]:
        parts = line.split()
        if len(parts) > 1 and parts[0].startswith("emulator-") and parts[1] == "device":
            try:
                if run(adb, "-s", parts[0], "emu", "avd", "name").splitlines()[0].strip() == AVD:
                    return parts[0]
            except (RuntimeError, IndexError):
                continue
    return None


def emulator_ready(adb, serial):
    # boot_completed alone is not enough: some AVDs refuse installs until the package service answers.
    try:
        return (run(adb, "-s", serial, "shell", "getprop", "sys.boot_completed") == "1"
                and run(adb, "-s", serial, "shell", "pm", "path", "android").startswith("package:"))
    except RuntimeError:
        return False


def wait_for(check, what):
    deadline = time.time() + BOOT_TIMEOUT
    while True:
        result = check()
        if result:
            return result
        if time.time() > deadline:
            raise RuntimeError(f"{what} within {BOOT_TIMEOUT}s")
        time.sleep(2)


def emulator_serial(adb, sdk):
    serial = avd_serial(adb)
    if not serial:
        print(f"booting {AVD}", flush=True)
        with open(os.path.join(LOG_DIR, f"{AVD}.log"), "w") as out:
            proc = subprocess.Popen([os.path.join(sdk, "emulator", "emulator"), "-avd", AVD],
                                    stdin=subprocess.DEVNULL, stdout=out, stderr=subprocess.STDOUT, start_new_session=True)

        def booted():
            if proc.poll() is not None:
                raise RuntimeError(f"{AVD} exited without booting (see {LOG_DIR}/{AVD}.log)")
            return avd_serial(adb)
        serial = wait_for(booted, f"{AVD} did not come up")
    wait_for(lambda: emulator_ready(adb, serial), f"{serial} did not finish booting")
    return serial


def gradle_install(path, serial):
    env = dict(os.environ, ANDROID_SERIAL=serial)
    if "JAVA_HOME" not in env:
        java = subprocess.run(["/usr/libexec/java_home", "-v", "21"], capture_output=True, text=True)
        if java.returncode != 0:
            java = subprocess.run(["/usr/libexec/java_home"], capture_output=True, text=True)
        env["JAVA_HOME"] = java.stdout.strip()
    p = subprocess.run(["./gradlew", ":app:installDebug"], cwd=path, env=env, stdin=subprocess.DEVNULL,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    print(p.stdout, flush=True)
    return p.returncode == 0, p.stdout


def launch(target, path):
    sdk = sdk_dir(path)
    adb = os.path.join(sdk, "platform-tools", "adb")
    serial = phone_serial(adb) if target == "phone" else emulator_serial(adb, sdk)
    print(f"installing the debug build on {serial}", flush=True)
    ok, out = gradle_install(path, serial)
    if not ok:
        # A lower version code or another signing key: only a fresh install replaces the build (drops its data).
        if not re.search(r"INSTALL_FAILED_(VERSION_DOWNGRADE|UPDATE_INCOMPATIBLE)", out):
            raise RuntimeError(f"installDebug failed on {serial}")
        print(f"installed build is incompatible; uninstalling {APP_ID} from {serial}", flush=True)
        subprocess.run([adb, "-s", serial, "uninstall", APP_ID], stdin=subprocess.DEVNULL)
        ok, _ = gradle_install(path, serial)
        if not ok:
            raise RuntimeError(f"installDebug failed on {serial} after the uninstall")
    run(adb, "-s", serial, "shell", "am", "start", "-W", "-n", START_ACTIVITY)
    return serial


def notify(text, title):
    # Text goes in as argv, never into the AppleScript source.
    subprocess.run(["osascript", "-e", "on run argv", "-e", "display notification (item 1 of argv) with title (item 2 of argv)",
                    "-e", "end run", text, title], stdin=subprocess.DEVNULL, capture_output=True)


def launch_main(target, path):
    title = f"{os.path.basename(path)} on {target}"
    try:
        serial = launch(target, path)
        print(f"started on {serial}", flush=True)
        notify(f"Started on {serial}", title)
    except Exception as e:
        print(f"FAILED: {e}", flush=True)
        notify(f"Failed: {e}", title)
        sys.exit(1)


def main():
    header = sys.stdin.buffer.read(4)
    if len(header) < 4:
        return
    msg = json.loads(sys.stdin.buffer.read(struct.unpack("=I", header)[0]))
    try:
        resp = handle(msg)
    except Exception as e:
        resp = {"ok": False, "error": str(e)}
    out = json.dumps(resp).encode()
    sys.stdout.buffer.write(struct.pack("=I", len(out)) + out)
    sys.stdout.buffer.flush()


if __name__ == "__main__":
    if len(sys.argv) == 4 and sys.argv[1] == "--launch" and sys.argv[2] in ("phone", "emulator"):
        launch_main(sys.argv[2], sys.argv[3])
    else:
        main()
