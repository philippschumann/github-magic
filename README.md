# Github Magic

A Chrome and Firefox extension that adds two buttons to GitHub pull request pages. One checks the PR out locally and opens it in Android Studio or runs it on a device; the other opens a Claude session on it. macOS only.

- **Android Studio ▾**: Open in Android Studio, Run on phone, Run on emulator
- **Claude**: opens the Claude desktop session for the PR's worktree, or starts a new one

Every action first checks the PR out as a git worktree named `<repo>-<branch>` inside the folder you choose. If the branch is already checked out somewhere (your main clone, another worktree), it uses that folder instead. The branch name comes from the PR page and is fetched from `origin`, so PRs from forks aren't supported.

## Requirements

- The Xcode command line tools (`xcode-select --install`), for `git` and `/usr/bin/python3`
- A main clone of each repo whose `origin` is the GitHub repo and can fetch without a prompt (SSH key or saved credentials)
- Android Studio, and an Android SDK whose path is set as `sdk.dir` in the main clone's `local.properties`
- The Claude desktop app, for the Claude button

## Install

1. Register the native helper with both browsers:

   ```bash
   bash native-host/install.sh
   ```

   Run it again whenever you move this folder.

2. Load the extension:
   - **Chrome:** `chrome://extensions` → Developer mode → Load unpacked → this folder.
   - **Firefox:** `about:debugging#/runtime/this-firefox` → Load Temporary Add-on → `manifest.json`. That lasts until Firefox restarts. For development, `npx web-ext run --source-dir .` starts Firefox with the extension and reloads it on every change (that Firefox shows a red-striped address bar because web-ext remote-controls it).
   - **Firefox, permanent:** sign it as an unlisted add-on with your [addons.mozilla.org API key](https://addons.mozilla.org/developers/addon/api/key/), then drag the `.xpi` from `web-ext-artifacts/` into Firefox. Each signed build needs a higher `version` in `manifest.json`.

     ```bash
     read "K?AMO JWT issuer: " && read -s "S?AMO JWT secret: " && echo && WEB_EXT_API_KEY="$K" WEB_EXT_API_SECRET="$S" npx web-ext sign; unset K S
     ```

3. Add your repos to `REPOS` at the top of [native-host/open_pr_in_studio.py](native-host/open_pr_in_studio.py): GitHub repo → local main clone. The helper refuses any repo that isn't listed.

## First use

The first time you use the buttons, they ask for the folder new worktrees go in (Android Studio shows only "Set up" until it's chosen). You can change it later in the extension's settings (click the extension icon, or Chrome: Details → Extension options; Firefox: about:addons → Preferences).

## What the actions do

- **Open in Android Studio:** `open -a "Android Studio" <worktree>`. Opening the same PR again brings its window to the front.
- **Bringing the worktree up to date:** both Android Studio actions fetch the PR's branch from `origin` and merge it with `--ff-only`. That only fast-forwards: local commits, or changes a pull would overwrite, stop it with an error.
  - Open in Android Studio opens the window first and updates while it loads.
  - Run on phone / emulator updates first, then builds.
- **Run on phone / emulator:** builds and installs `:app:installDebug`, then starts the app. Phone order is Pixel 10, then Pixel 6, then any other phone. The emulator is `Pixel_9_API_35`, booted if it isn't running. The button says "Installing…" until the app has started, and a macOS notification reports the result.
  - A new run cancels the one still going on that device, whatever PR it was for.
  - If the device refuses the build as a downgrade or because of a different signing key, the app is uninstalled and installed again, which wipes its data.
  - Logs: `~/Library/Logs/github-pr-quick-filters/`.
- **Claude:** reopens the newest unarchived session in that folder or starts a new one.

The app id, start activity, emulator and phone order are constants at the top of the helper script.

## Releases

Publishing a GitHub release signs the Firefox build and attaches it as `github-magic-<version>.xpi` ([sign-firefox.yml](.github/workflows/sign-firefox.yml)). The tag must be `v` plus the `version` in `manifest.json`, and the repo needs your AMO JWT issuer and secret as the `AMO_JWT_ISSUER` and `AMO_JWT_SECRET` secrets.

## Tests

```bash
/usr/bin/python3 native-host/test_open_pr_in_studio.py
```

They use stand-ins for `adb`, Gradle and the Claude app, so no device is needed.

## Quick filters

The extension also has quick filter buttons for a repo's pull request list. They're switched off with `SHOW_FILTERS = false` at the top of [content.js](content.js).
