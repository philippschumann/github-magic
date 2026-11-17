# Handoff: desktop app phase 2

Brief for whoever implements phase 2 of [desktop-app.md](desktop-app.md). Every decision is settled, so start building without asking again. Delete this file in the phase 2 PR.

## State

- Phase 1 is merged (#2). `desktop/` has the `core`, `bridge` and `app` crates. The socket server in `desktop/app/src/server.rs` pipes every request frame to `/usr/bin/python3 native-host/open_pr_in_studio.py` and returns its reply.
- The phase 3 plan (one `settings/` page shared by the extension and the app) is merged (#3).
- The extension currently talks to the Python host directly. The phase 1 session restored the native messaging manifests after testing.

## Task

Port the actions to Rust in this order, one commit each, on one branch with one PR against main:

1. `detect`
2. `open`
3. `claude`
4. The run manager with phone and emulator runs, `status` and cancel. This step also brings the tray run lines, the notifications with a Show log button, and the quit-with-run-going check.
5. iPhone and simulator runs.

The handler sends ported actions to `core` and the rest to Python. Done when no request reaches Python.

## Decisions

- **No `settings.json` until phase 3.**
  - Rust gets `REPOS`, `APP_ID`, `IOS_SCHEME` and `PHONE_MODELS` by running `/usr/bin/python3` once to import `native-host/config.py` and print them as JSON.
  - `APP_ID` and `IOS_SCHEME` apply to every repo.
  - `START_ACTIVITY` is ignored, because the `resolve-activity` lookup replaces it.
  - Mark this with a `shortcut:` comment saying phase 3 replaces it.
- **Today's message shapes stay:**
  - no `protocol` field
  - status polls send the `log` path
  - replies keep today's fields
  - the settings the extension sends with each request are used as today
- **Tests:**
  - Port the classes of `native-host/test_open_pr_in_studio.py` alongside their steps.
  - Copy the stand-in shell tools unchanged into `desktop/core/tests/fixtures/`.
- **Hand tests at the end:** the implementer runs these:
  - every button on a real Android PR and a real iOS PR (phase 1 used gocrush/Meet5AndroidApp #1102 and gocrush/Meet5iOSApp #785)
  - Run on phone on the connected Pixel
  - Run on iPhone only if an iPhone is connected; otherwise check only its error path
  - cancel from the tray
  - the notification and Show log
  - quitting with a run going
- **Risks to check in this phase:**
  - **Notification action buttons on macOS:** if they don't work, fall back to `mac-notification-sys`; failing that, notify without the button.
  - **Live tray menu text:** if it doesn't update while the menu is open, show the start time instead of the elapsed time.

## Facts

- `config.py` is gitignored and exists only in the main checkout (`native-host/config.py`). Copy it into your worktree and don't commit it.
- Run the app with `cargo tauri dev` to test through it. That points the native messaging manifests at the bridge. Restore them afterwards the way phase 1 did.
- CI is `.github/workflows/desktop.yml`: `cargo clippy --all-targets -- -D warnings` and `cargo test` on macos-latest.
- When done, commit, push and open the PR against main with an empty description. Don't merge it.
