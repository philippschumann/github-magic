# Github Magic

Two buttons on GitHub pull request pages. One checks the PR out locally and opens it in Android Studio, IntelliJ IDEA, VS Code or Xcode, or runs it on a device; the other opens a Claude session on it. It comes in two parts: a Chrome and Firefox extension that adds the buttons, and a menu bar app that does the work. macOS only.

- **IDE ▾**: Open in your IDE, Run on phone, Run on emulator. The button shows your IDE's name once you've picked one. The run options only appear when adb is found, and Run on emulator only when you have an emulator.
  - On iOS repos: Open in Xcode or VS Code, Run on iPhone, Run on simulator. The run options only appear when Xcode is installed, and Run on simulator only when you have a simulator. A repo is an iOS repo when its main clone has an `.xcworkspace` or `.xcodeproj` at its root.
- **Claude**: opens the Claude desktop session for the PR's worktree, or starts a new one

Every action first checks the PR out as a git worktree named `<repo>-<branch>` inside the folder you choose. If the branch is already checked out somewhere (your main clone, another worktree), it uses that folder instead. The branch name comes from the PR page and is fetched from `origin`, so PRs from forks aren't supported.

## Requirements

- The Xcode command line tools (`xcode-select --install`), for `git`
- A main clone of each repo whose `origin` is the GitHub repo and can fetch without a prompt (SSH key or saved credentials)
- Android Studio, IntelliJ IDEA or VS Code in `/Applications` or `~/Applications`
- For Run on phone / emulator: an Android SDK with adb, whose path is set as `sdk.dir` in the main clone's `local.properties`
- For iOS repos: Xcode, selected with `xcode-select`. For Run on iPhone, your Apple account signed in to Xcode, and the iPhone paired with this Mac, in Developer Mode and unlocked
- The Claude desktop app, for the Claude button

## Install

1. Install the app:

   ```bash
   brew install --cask philippschumann/tap/github-magic
   ```

   Or open the `.dmg` from the [latest release](https://github.com/philippschumann/github-magic/releases/latest) and drag Github Magic to Applications.

2. Open Github Magic. The app isn't signed with an Apple certificate, so macOS blocks it the first time: open System Settings → Privacy & Security and click Open Anyway. It then lives in the menu bar and opens at login.

3. Install the extension:
   - **Chrome:** `chrome://extensions` → Developer mode → Load unpacked → a checkout of this repo.
   - **Firefox:** open the `.xpi` from the latest release.

4. Add your repos: click the menu bar icon → Settings…, then Add… and choose each repo's main clone.

## First use

- **A repo you haven't added:** the IDE button only offers "Add repo…", which asks for that repo's main clone.
- **Worktree folder:** the first time you use the buttons, they ask for the folder new worktrees go in (the IDE button shows only "Set up" until it's chosen).
- **IDE:** the first click on the IDE button asks which IDE to use; only installed ones are offered. It asks again if that IDE is uninstalled. iOS repos have their own IDE setting: Xcode or VS Code.
- **Emulator and simulator:** the first Run on emulator asks which emulator to use, from the ones in `~/.android/avd`. It asks again if that emulator is deleted. Run on simulator does the same with the iOS simulators Xcode has.

## Settings

Open them from the menu bar icon → Settings…, or by clicking the extension's icon in the browser. Both show the same page, and a change in one shows up in the other. The app keeps the settings in `~/Library/Application Support/com.philippschumann.github-magic/settings.json`; the extension stores nothing.

- **Repos:** each GitHub repo with its main clone on this Mac. Android repos need their app ID (for example `com.example.myapp`) for device runs, iOS repos their Xcode scheme. The buttons refuse any repo that isn't listed.
- **Worktree folder**, **Android IDE**, **iOS IDE**, **Emulator**, **Simulator:** what the first-use questions set.
- **Phone order:** which connected phone Run on phone prefers, one per line as `adb devices -l` shows them, for example `model:Pixel_10`. Any other phone comes after these.
- **Open Github Magic at login**

## What the actions do

- **Open in your IDE:** `open -a <IDE app> <worktree>`. Opening the same PR again brings its window to the front. Xcode gets the worktree's workspace or project file instead of the folder.
- **Bringing the worktree up to date:** the IDE button's actions fetch the PR's branch from `origin` and merge it with `--ff-only`. That only fast-forwards: local commits, or changes a pull would overwrite, stop it with an error.
  - Open in your IDE opens the window first and updates while it loads.
  - Run on phone / emulator / iPhone / simulator updates first, then builds.
  - If the branch is gone from `origin` (usually deleted after the PR was merged), they stop with a "Branch not found" dialog before opening or building anything. If `origin` can't be reached, they go ahead as usual.
- **Run on phone / emulator:** builds and installs `:app:installDebug`, then starts the app with its launcher activity. The emulator is the one you chose, booted if it isn't running.
  - If the device refuses the build as a downgrade or because of a different signing key, the app is uninstalled and installed again, which wipes its data.
- **Run on iPhone / simulator:** builds the Debug configuration of the repo's scheme with `xcodebuild`, installs it and starts it. It uses Xcode's default DerivedData, so it shares the build cache with Xcode builds of the same worktree. The app and its bundle ID come from the scheme's build settings.
  - iPhone: the first paired iPhone within reach, connected ones first. The build passes `-allowProvisioningUpdates`, so automatic signing can register the iPhone and fetch a profile.
  - Simulator: the one you chose, booted if it isn't running, with the Simulator app brought to the front.
- **While a run is going:** the button says "Installing…" until the app has started. The menu bar menu shows the run with its elapsed time, with Cancel and Show log, and keeps the last result of each device. A notification reports the result.
  - A new run cancels the one still going on that device, whatever PR it was for.
  - Quitting the app asks first if a run is going, and cancels it. Emulators and simulators keep running.
- **Claude:** reopens the newest unarchived session in that folder or starts a new one.

## Logs

Each run writes `<branch>-<device>.log` to `~/Library/Logs/com.philippschumann.github-magic/`, and an emulator the app boots writes `<emulator>.log` there. Show log in the menu bar menu or in the notification opens the run's log.

## Development

The app is a [Tauri](https://v2.tauri.app) app in [desktop/](desktop): `core` holds the logic, `app` the tray menu and the Settings window, and `bridge` is the native messaging host the browsers start, which passes each request to the running app. The plan and the reasons behind it are in [docs/desktop-app.md](docs/desktop-app.md).

- **Run the app:** with Rust and `cargo install tauri-cli`:

  ```bash
  cd desktop/app && cargo tauri dev
  ```

  Starting the app, this build or an installed one, points the browsers' native messaging host at its own bridge.
- **Load the extension:**
  - Chrome: Load unpacked, as above.
  - Firefox: `npx web-ext run --source-dir .` starts Firefox with the extension and reloads it on every change.
- **Tests:** they use stand-ins for `adb`, Gradle, `xcrun`, `xcodebuild` and the Claude app, so no device is needed.

  ```bash
  cd desktop && cargo test
  ```

## Releases

The app and the extension share one version. To release, set the same `version` in `manifest.json` and `desktop/Cargo.toml`, merge that, and publish a GitHub release tagged `v<version>`. [release.yml](.github/workflows/release.yml) then:

- **Firefox:** signs the extension on addons.mozilla.org (unlisted) and attaches it as `github-magic-<version>.xpi`.
- **Chrome:** uploads the extension, without `key`, to the Chrome Web Store and publishes it with the listing's visibility, which is set to unlisted in the Web Store dashboard. This step is skipped until the listing exists.
- **macOS:** builds the universal app and attaches the `.dmg`, plus the files the app's updater reads: the app archive, its signature and `latest.json`.
- **Homebrew:** points the cask in [philippschumann/homebrew-tap](https://github.com/philippschumann/homebrew-tap) at the new `.dmg`. This step is skipped until the tap's deploy key is set.

Installed apps look for a newer release when they start and once a day, offer it as "Update to x.y.z" in the menu bar menu, and install it only when that is clicked.

The repo needs these secrets:

| Secret | For |
|---|---|
| `AMO_JWT_ISSUER`, `AMO_JWT_SECRET` | Firefox signing: your addons.mozilla.org API key |
| `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Signing the updater's app archive. The public half is `plugins.updater.pubkey` in [tauri.conf.json](desktop/app/tauri.conf.json). Keep the key safe: without it, installed apps can't be updated |
| `HOMEBREW_TAP_DEPLOY_KEY` | The private half of a deploy key with write access on the tap repo |
| `CWS_EXTENSION_ID`, `CWS_PUBLISHER_ID`, `CWS_CLIENT_ID`, `CWS_CLIENT_SECRET`, `CWS_REFRESH_TOKEN` | The Chrome Web Store upload, see [chrome-webstore-upload-keys](https://github.com/fregante/chrome-webstore-upload-keys) |

To build the app locally as the release does, with the updater key at hand:

```bash
cd desktop/app && TAURI_SIGNING_PRIVATE_KEY=~/.tauri/github-magic.key TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(cat ~/.tauri/github-magic.key.password)" cargo tauri build
```

## Quick filters

The extension also has quick filter buttons for a repo's pull request list. They're switched off with `SHOW_FILTERS = false` at the top of [content.js](content.js).
