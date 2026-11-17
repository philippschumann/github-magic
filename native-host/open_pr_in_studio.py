#!/usr/bin/python3
"""Native messaging host: checks out a GitHub PR in a sibling git worktree, then opens it in an IDE,
runs it on a device (Android phone or emulator, iPhone or iOS simulator), or opens a Claude desktop session there.

Receives {"repo": "owner/name", "pr": 123, "action": "open|phone|emulator|iphone|simulator|claude", "branch": ...,
"title": ..., "ide": "<IDES key>", "iosIde": "<IOS_IDES key>", "emulator": "<AVD name>", "simulator": "<simulators() key>"} from the browser
extension, which reads the PR's branch and title from the page. Replies {"ok": true, "path": ...} or
{"ok": false, "error": ...}, plus a "title" for errors the extension shows in a dialog (the PR's branch is gone).
Only repos listed in REPOS (config.py) are accepted. A repo whose main clone has an Xcode project at its root is an iOS repo:
it opens in "iosIde" and runs on an iPhone or simulator; any other repo is an Android repo and opens in "ide". {"action": "detect"} lists the repo's platform, the installed IDEs of both platforms, the
emulators and simulators, and whether
the repo's Android SDK has adb.
Device runs take minutes: they continue in a background process (this script with --launch)
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

CONFIG = os.path.join(os.path.dirname(os.path.realpath(__file__)), "config.py")
try:  # your repos and app; see config.example.py
    from config import REPOS, IOS_SCHEME, APP_ID, START_ACTIVITY, PHONE_MODELS
except ImportError:
    REPOS, IOS_SCHEME, APP_ID, START_ACTIVITY, PHONE_MODELS = {}, "", "", "", []
# gitignored files a fresh worktree needs to build.
COPY_FILES = ["local.properties"]
# IDE name -> app bundles to look for in APP_DIRS; the first one found is opened.
IDES = {
    "Android Studio": ["Android Studio.app"],
    "IntelliJ IDEA": ["IntelliJ IDEA.app", "IntelliJ IDEA Ultimate.app", "IntelliJ IDEA CE.app"],
    "VS Code": ["Visual Studio Code.app"],
}
# The same for iOS repos. Xcode gets the project file; iPhone and simulator runs need it too.
IOS_IDES = {
    "Xcode": ["Xcode.app", "Xcode-beta.app"],
    "VS Code": ["Visual Studio Code.app"],
}
APP_DIRS = ["/Applications", os.path.expanduser("~/Applications")]  # ~/Applications: JetBrains Toolbox installs
RUN_TARGETS = ("phone", "emulator", "iphone", "simulator")
ACTIONS = ("open", "claude", *RUN_TARGETS)

AVD_DIR = os.path.expanduser("~/.android/avd")  # each emulator has a <name>.ini here
BOOT_TIMEOUT = 300
LOG_DIR = os.path.expanduser("~/Library/Logs/github-magic")
SESSIONS_DIR = os.path.expanduser("~/Library/Application Support/Claude/claude-code-sessions")

# Browsers start the host with a minimal PATH and no JAVA_HOME; keep Homebrew's tools (git, anything the build
# calls) reachable.
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


def ensure_worktree(root, branch, base_dir):
    # Branch already checked out (main clone or another worktree): use that instead of a second copy.
    path = worktree_for_branch(root, branch)
    if not path:
        name = f"{os.path.basename(root)}-{branch.replace('/', '-')}"
        path = os.path.join(base_dir, name) if base_dir else os.path.join(os.path.dirname(root), name)
        if not os.path.exists(path):
            run("git", "-C", root, "worktree", "add", "--detach", path)
            try:
                update(path, branch)
            except Exception:
                run("git", "-C", root, "worktree", "remove", "--force", path)
                raise
    # Worktrees made elsewhere can lack these too; an existing copy is never overwritten.
    for name in COPY_FILES:
        src, dst = os.path.join(root, name), os.path.join(path, name)
        if os.path.exists(src) and not os.path.exists(dst):
            shutil.copy2(src, dst)
    # Android Studio and the Claude app match folders by resolved path; same path = same window/session.
    return os.path.realpath(path)


def branch_deleted(root, branch):
    """True only when origin answers that it has no such branch; an unreachable origin doesn't count."""
    return subprocess.run(["git", "-C", root, "ls-remote", "--exit-code", "origin", f"refs/heads/{branch}"],
                          capture_output=True, stdin=subprocess.DEVNULL).returncode == 2


def update(path, branch):
    # Fetch the branch, switch to it (a new local branch tracks origin's), then only fast-forward: local commits
    # or changes it would overwrite stop it with an error instead of being lost.
    # shortcut: the branch must live on the clone's "origin"; PRs from forks would need their own remote.
    run("git", "-C", path, "fetch", "origin", f"+refs/heads/{branch}:refs/remotes/origin/{branch}")
    run("git", "-C", path, "-c", "checkout.defaultRemote=origin", "switch", branch)
    run("git", "-C", path, "merge", "--ff-only", f"refs/remotes/origin/{branch}")


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


def current_run(target):
    """The latest run on this target as {"pid", "log"}, if its process is still one of our runs."""
    try:
        with open(os.path.join(LOG_DIR, f"{target}.pid")) as f:
            info = json.load(f)
        pid = int(info["pid"])
    except (OSError, ValueError, TypeError, KeyError):
        return None
    # The pid may have been reused by an unrelated process since; only count one of our own runs.
    cmd = subprocess.run(["ps", "-o", "command=", "-p", str(pid)], capture_output=True, text=True).stdout
    return info if "--launch" in cmd and os.path.basename(__file__) in cmd else None


def cancel_previous(target):
    """Stop the run still going on this target (any PR), so a new click starts fresh."""
    info = current_run(target)
    if info:
        try:
            # Each run leads its own process group: this ends the gradle client too, and its daemon cancels the build.
            os.killpg(int(info["pid"]), signal.SIGTERM)
        except ProcessLookupError:
            pass


def start_launch(target, path, branch, device):
    os.makedirs(LOG_DIR, exist_ok=True)
    cancel_previous(target)
    log = os.path.join(LOG_DIR, f"{branch.replace('/', '-')}-{target}.log")
    with open(log, "w") as out:
        # New session: the browser may kill the host's process group once it has replied.
        proc = subprocess.Popen([sys.executable, os.path.abspath(__file__), "--launch", target, path, device],
                                stdin=subprocess.DEVNULL, stdout=out, stderr=subprocess.STDOUT, start_new_session=True)
    with open(os.path.join(LOG_DIR, f"{target}.pid"), "w") as f:
        json.dump({"pid": proc.pid, "log": log}, f)
    return log


def run_status(log):
    """State of a device run, read from the last line its background process wrote."""
    log = os.path.realpath(log) if isinstance(log, str) else ""
    m = re.search(rf"-({'|'.join(RUN_TARGETS)})\.log$", log)
    if os.path.dirname(log) != os.path.realpath(LOG_DIR) or not m or not os.path.isfile(log):
        raise ValueError("unknown run")
    with open(log, errors="replace") as f:
        lines = [line for line in f.read().splitlines() if line.strip()]
    last = lines[-1] if lines else ""
    if last.startswith("started on "):
        return {"ok": True, "state": "started", "message": last}
    if last.startswith("FAILED: "):
        return {"ok": True, "state": "failed", "message": last[len("FAILED: "):]}
    if last == "CANCELLED":
        return {"ok": True, "state": "cancelled"}
    info = current_run(m.group(1))
    if info and os.path.realpath(info.get("log", "")) == log:
        return {"ok": True, "state": "running"}
    return {"ok": True, "state": "failed", "message": "the run stopped without finishing"}


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


def find_app(apps):
    """Path of the first of these app bundles found in APP_DIRS, else None."""
    return next((p for p in (os.path.join(d, a) for a in apps for d in APP_DIRS) if os.path.isdir(p)), None)


def installed_ides(ides=IDES):
    """Installed IDEs (of IDES or IOS_IDES) as name -> app bundle path."""
    return {name: path for name, path in ((name, find_app(apps)) for name, apps in ides.items()) if path}


def xcode_project(path):
    """The Xcode workspace or project at the folder's root (a workspace wins: it includes the project), else None."""
    try:
        names = sorted(os.listdir(path), key=lambda n: not n.endswith(".xcworkspace"))
    except OSError:
        return None
    return next((os.path.join(path, n) for n in names if n.endswith((".xcworkspace", ".xcodeproj"))), None)


def avds():
    """Emulator names. Read from AVD_DIR, which needs no repo or SDK path, so the settings page can list them too."""
    try:
        return sorted(name[:-len(".ini")] for name in os.listdir(AVD_DIR) if name.endswith(".ini"))
    except OSError:
        return []


def simulators():
    """Available iOS simulators as "iPhone 17 Pro (iOS 26.3)" -> UDID, newest iOS first."""
    try:
        runtimes = json.loads(run("xcrun", "simctl", "list", "devices", "available", "-j"))["devices"]
    except (RuntimeError, ValueError, KeyError, TypeError):
        return {}  # no Xcode
    found = {}
    # Runtime ids end in e.g. "iOS-26-3"; watchOS, tvOS and visionOS runtimes are skipped.
    versions = {r: re.search(r"\.iOS-([\d-]+)$", r) for r in runtimes}
    for r in sorted((r for r in runtimes if versions[r]),
                    key=lambda r: [int(n) for n in versions[r].group(1).split("-")], reverse=True):
        for sim in runtimes[r]:
            # shortcut: two simulators with the same name and iOS share a label; the first one is used.
            found.setdefault(f"{sim['name']} (iOS {versions[r].group(1).replace('-', '.')})", sim["udid"])
    return found


def detect(repo):
    """The repo's platform, installed IDEs and Xcode, emulators and simulators, and whether adb is in the repo's
    Android SDK (sdk.dir), which phone/emulator runs use."""
    try:
        root = os.path.expanduser(REPOS[repo])
    except (KeyError, TypeError):
        root = None
    try:
        adb = os.access(os.path.join(sdk_dir(root), "platform-tools", "adb"), os.X_OK)
    except (TypeError, OSError, RuntimeError):
        adb = False
    return {"ok": True, "platform": "ios" if root and xcode_project(root) else "android", "ides": list(installed_ides()),
            "iosIdes": list(installed_ides(IOS_IDES)), "avds": avds(), "simulators": list(simulators()), "adb": adb}


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
    if action == "status":
        return run_status(msg.get("log"))
    if action == "detect":
        return detect(repo)
    if repo not in REPOS:
        raise ValueError(f"{repo} is not configured in {CONFIG}")
    if not isinstance(pr, int) or isinstance(pr, bool) or pr <= 0:
        raise ValueError(f"invalid PR number: {pr!r}")
    if action not in ACTIONS:
        raise ValueError(f"unknown action: {action!r}")
    # The branch name ends up in git arguments and the worktree path: no options, no "..". A fork's "owner:branch"
    # fails here too.
    branch = msg.get("branch")
    if not isinstance(branch, str) or branch.startswith("-") or subprocess.run(
            ["git", "check-ref-format", f"refs/heads/{branch}"], capture_output=True).returncode:
        raise ValueError(f"can't check out branch {branch!r}; PRs from forks aren't supported")
    root = os.path.expanduser(REPOS[repo])
    base_dir = worktree_dir(msg.get("worktreeDir"))
    project = xcode_project(root)
    ide = msg.get("iosIde" if project else "ide")
    app = installed_ides(IOS_IDES if project else IDES).get(ide) if action == "open" else None
    if action == "open" and not app:
        raise ValueError(f"IDE {ide!r} is not installed; choose another in the extension's settings")
    device = ""
    # The emulator name goes to the emulator command as an argument; the simulator becomes a UDID from simctl.
    if action == "emulator":
        device = msg.get("emulator") or ""
        if device not in avds():
            raise ValueError(f"emulator {device!r} not found; choose another in the extension's settings")
    if action == "simulator":
        device = simulators().get(msg.get("simulator"))
        if not device:
            raise ValueError(f"simulator {msg.get('simulator')!r} not found; choose another in the extension's settings")

    # Checked before anything opens: an existing worktree would open, then fail to update. The Claude button skips
    # it, so a merged PR's session can still be reopened.
    if action != "claude" and branch_deleted(root, branch):
        return {"ok": False, "title": "Branch not found", "error": f"The branch {branch} no longer exists on GitHub. "
                "It was probably deleted after the PR was merged or closed."}
    path = ensure_worktree(root, branch, base_dir)
    if action == "open":
        # Xcode gets the project file: a plain folder would open as a Swift package.
        run("open", "-a", app, os.path.join(path, os.path.basename(project)) if project and ide == "Xcode" else path)
        update(path, branch)  # `open` returns at once, so the IDE loads the project while the update runs
    elif action == "claude":
        open_claude(path, f"{msg.get('title') or ''} (#{pr}): ")
    else:
        update(path, branch)
        return {"ok": True, "path": path, "log": start_launch(action, path, branch, device)}
    return {"ok": True, "path": path}


# ---- background device run (--launch <RUN_TARGETS item> <worktree> <AVD name or simulator UDID>) ----

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


def avd_serial(adb, avd):
    for line in run(adb, "devices").splitlines()[1:]:
        parts = line.split()
        if len(parts) > 1 and parts[0].startswith("emulator-") and parts[1] == "device":
            try:
                if run(adb, "-s", parts[0], "emu", "avd", "name").splitlines()[0].strip() == avd:
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


def emulator_serial(adb, sdk, avd):
    serial = avd_serial(adb, avd)
    if not serial:
        print(f"booting {avd}", flush=True)
        with open(os.path.join(LOG_DIR, f"{avd}.log"), "w") as out:
            proc = subprocess.Popen([os.path.join(sdk, "emulator", "emulator"), "-avd", avd],
                                    stdin=subprocess.DEVNULL, stdout=out, stderr=subprocess.STDOUT, start_new_session=True)

        def booted():
            if proc.poll() is not None:
                raise RuntimeError(f"{avd} exited without booting (see {LOG_DIR}/{avd}.log)")
            return avd_serial(adb, avd)
        serial = wait_for(booted, f"{avd} did not come up")
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


def iphone_udid():
    """The first paired iPhone that isn't out of reach, connected ones first."""
    out = os.path.join(LOG_DIR, "devices.json")  # devicectl writes its JSON only to a file
    run("xcrun", "devicectl", "list", "devices", "--json-output", out)
    with open(out) as f:
        devices = json.load(f)["result"]["devices"]
    phones = [d for d in devices if d["hardwareProperties"].get("deviceType") == "iPhone"
              and d["hardwareProperties"].get("reality") == "physical"
              and d["connectionProperties"].get("pairingState") == "paired"
              and d["connectionProperties"].get("tunnelState") != "unavailable"]
    if not phones:
        raise RuntimeError("no iPhone connected; plug one in, unlock it and trust this Mac")
    phones.sort(key=lambda d: d["connectionProperties"].get("tunnelState") != "connected")
    return phones[0]["hardwareProperties"]["udid"]


def ios_launch(target, path, udid):
    """Build the scheme for the iPhone or the simulator with this UDID, install it there and start it."""
    if target == "simulator":
        print(f"booting simulator {udid}", flush=True)
        run("xcrun", "simctl", "bootstatus", udid, "-b")  # boots it if needed and waits until it has
        run("open", "-a", "Simulator")
    else:
        udid = iphone_udid()
    project = xcode_project(path)
    xcodebuild = ["xcodebuild", "-workspace" if project.endswith(".xcworkspace") else "-project", project,
                  "-scheme", IOS_SCHEME, "-configuration", "Debug", "-destination", f"id={udid}"]
    print(f"building {IOS_SCHEME} for {udid}", flush=True)
    # The iPhone needs a signed build: automatic signing may have to register it and fetch a profile.
    signing = ["-allowProvisioningUpdates"] if target == "iphone" else []
    p = subprocess.run([*xcodebuild, *signing, "build"], cwd=path, stdin=subprocess.DEVNULL,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    print(p.stdout, flush=True)
    if p.returncode != 0:
        raise RuntimeError(f"xcodebuild failed for {udid}")
    settings = next(t["buildSettings"] for t in json.loads(run(*xcodebuild, "-showBuildSettings", "-json", cwd=path))
                    if t["buildSettings"].get("WRAPPER_EXTENSION") == "app")
    app = os.path.join(settings["TARGET_BUILD_DIR"], settings["WRAPPER_NAME"])
    bundle_id = settings["PRODUCT_BUNDLE_IDENTIFIER"]
    print(f"installing {app} on {udid}", flush=True)
    if target == "simulator":
        run("xcrun", "simctl", "install", udid, app)
        run("xcrun", "simctl", "launch", "--terminate-running-process", udid, bundle_id)
    else:
        run("xcrun", "devicectl", "device", "install", "app", "--device", udid, app)
        run("xcrun", "devicectl", "device", "process", "launch", "--terminate-existing", "--device", udid, bundle_id)
    return udid


def launch(target, path, avd):
    if target in ("iphone", "simulator"):
        return ios_launch(target, path, avd)
    sdk = sdk_dir(path)
    adb = os.path.join(sdk, "platform-tools", "adb")
    serial = phone_serial(adb) if target == "phone" else emulator_serial(adb, sdk, avd)
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


def on_cancel(_signum, _frame):
    # A newer run on this target sent SIGTERM; the marker lets the extension tell "cancelled" from "failed".
    print("CANCELLED", flush=True)
    sys.exit(143)


def launch_main(target, path, avd):
    title = f"{os.path.basename(path)} on {target}"
    signal.signal(signal.SIGTERM, on_cancel)
    try:
        serial = launch(target, path, avd)
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
    if len(sys.argv) == 5 and sys.argv[1] == "--launch" and sys.argv[2] in RUN_TARGETS:
        launch_main(*sys.argv[2:])
    else:
        main()
