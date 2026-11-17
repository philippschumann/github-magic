//! The user's settings. The app owns them: `settings.json` in the app's config folder is the only copy, and every
//! write, whichever window or PR page it comes from, is validated here.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::platform::{IDES, IOS_IDES, installed_ides};
use crate::{Core, cmd, detect};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Repo {
    /// The local main clone.
    pub path: String,
    /// Android: the app that device runs start.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub app_id: String,
    /// iOS: the scheme whose Debug build device runs install.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub scheme: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// GitHub repo ("owner/name") -> its main clone and app.
    pub repos: BTreeMap<String, Repo>,
    /// The folder new worktrees go in. Empty: not set yet.
    pub worktree_dir: String,
    pub ide: String,
    pub ios_ide: String,
    /// An AVD name.
    pub emulator: String,
    /// A simulator's UDID: labels aren't unique and change when a simulator is renamed.
    pub simulator: String,
    /// `adb devices -l` substrings such as "model:Pixel_10", preferred in this order, else any other phone.
    pub phone_models: Vec<String>,
    pub open_at_login: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            repos: BTreeMap::new(),
            worktree_dir: String::new(),
            ide: String::new(),
            ios_ide: String::new(),
            emulator: String::new(),
            simulator: String::new(),
            phone_models: vec![],
            open_at_login: true,
        }
    }
}

/// The settings in memory and their file.
pub struct Store {
    file: PathBuf,
    settings: Mutex<Settings>,
    changes: broadcast::Sender<()>,
}

impl Store {
    /// Reads `settings.json` in this folder. Without one, everything starts unset.
    pub fn load(config_dir: &Path) -> Self {
        let file = config_dir.join("settings.json");
        let settings = match fs::read(&file).map(|json| serde_json::from_slice(&json)) {
            Ok(Ok(settings)) => settings,
            // Unreadable JSON: start fresh, but keep the file so nothing typed by hand is lost.
            Ok(Err(_)) => {
                let _ = fs::rename(&file, file.with_extension("json.invalid"));
                Settings::default()
            }
            Err(_) => Settings::default(),
        };
        Self { file, settings: Mutex::new(settings), changes: broadcast::channel(16).0 }
    }

    pub fn get(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    /// Every write arrives here, so an open settings window can show it.
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.changes.subscribe()
    }

    /// Applies a change and replaces settings.json in one step (a temp file, then a rename), so a crash never leaves
    /// half a file.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> Result<(), String> {
        let mut settings = self.settings.lock().unwrap();
        let mut changed = settings.clone();
        change(&mut changed);
        let temp = self.file.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(&changed).map_err(|e| e.to_string())?;
        fs::create_dir_all(self.file.parent().unwrap_or(Path::new("/")))
            .and_then(|()| fs::write(&temp, json))
            .and_then(|()| fs::rename(&temp, &self.file))
            .map_err(|e| format!("can't save {}: {e}", self.file.display()))?;
        *settings = changed;
        let _ = self.changes.send(());
        Ok(())
    }
}

/// The `set` action: one setting, checked before it is saved. `repo` says whose `appId` or `scheme` it is.
pub fn set(core: &Core, key: &str, value: &Value, repo: &Value) -> Result<(), String> {
    let text = || value.as_str().map(str::to_owned).ok_or_else(|| format!("{key} must be text"));
    // Empty text unsets a choice.
    let one_of = |known: Vec<String>, what: &str| match text()? {
        name if name.is_empty() || known.contains(&name) => Ok(name),
        name => Err(format!("{what} {name} not found")),
    };
    let ide = |ides| one_of(installed_ides(&core.paths, ides).into_iter().map(|(name, _)| name.to_owned()).collect(), "IDE");
    let of_repo = |value: String, set: fn(&mut Repo, String)| match repo.as_str() {
        Some(name) if core.settings.get().repos.contains_key(name) => core.settings.update(|s| {
            if let Some(repo) = s.repos.get_mut(name) {
                set(repo, value);
            }
        }),
        _ => Err(format!("{} isn't one of your repos", repo_name(repo))),
    };
    match key {
        "worktreeDir" => {
            let dir = text()?;
            let folder = core.paths.expand_user(&dir);
            if !dir.is_empty() && !(folder.is_absolute() && folder.is_dir()) {
                return Err(format!("the folder {dir} does not exist"));
            }
            core.settings.update(|s| s.worktree_dir = dir)
        }
        "ide" => ide(IDES).and_then(|name| core.settings.update(|s| s.ide = name)),
        "iosIde" => ide(IOS_IDES).and_then(|name| core.settings.update(|s| s.ios_ide = name)),
        "emulator" => one_of(detect::avds(&core.paths), "emulator").and_then(|avd| core.settings.update(|s| s.emulator = avd)),
        "simulator" => {
            let udids = detect::simulators(&core.paths).into_iter().map(|(_, udid)| udid).collect();
            one_of(udids, "simulator").and_then(|udid| core.settings.update(|s| s.simulator = udid))
        }
        "phoneModels" => {
            let model = |model: &Value| model.as_str().filter(|m| !m.is_empty() && !m.contains(char::is_whitespace)).map(str::to_owned);
            let models = value.as_array().and_then(|models| models.iter().map(model).collect::<Option<Vec<_>>>());
            let models = models.ok_or("the phone order must be adb device names without spaces, like model:Pixel_10")?;
            core.settings.update(|s| s.phone_models = models)
        }
        "openAtLogin" => {
            let on = value.as_bool().ok_or("openAtLogin must be true or false")?;
            core.settings.update(|s| s.open_at_login = on)
        }
        "appId" => {
            let app_id = text()?;
            let segment = |s: &str| s.starts_with(|c: char| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if app_id.split('.').count() < 2 || !app_id.split('.').all(segment) {
                return Err(format!("{app_id} is not an Android application ID, like com.example.myapp"));
            }
            of_repo(app_id, |repo, app_id| repo.app_id = app_id)
        }
        "scheme" => match text()? {
            scheme if scheme.trim().is_empty() => Err("the scheme needs a name".into()),
            scheme => of_repo(scheme, |repo, scheme| repo.scheme = scheme),
        },
        _ => Err(format!("unknown setting: {key}")),
    }
}

/// Adds the repo whose main clone is `clone`: its "owner/name" comes from the clone's `origin`. A PR page says
/// which repo it `expected`; a clone of another one is refused.
pub fn add_repo(core: &Core, clone: &Path, expected: Option<&str>) -> Result<String, String> {
    let origin = cmd::run(cmd::command(&core.paths, "git").arg("-C").arg(clone).args(["remote", "get-url", "origin"]));
    let origin = origin.map_err(|_| format!("{} is not a git clone with an origin", clone.display()))?;
    // "git@github.com:owner/name.git" or "https://github.com/owner/name"
    let mut segments = origin.trim_end_matches('/').trim_end_matches(".git").rsplit(['/', ':']);
    let (Some(name), Some(owner)) = (segments.next().filter(|s| !s.is_empty()), segments.next().filter(|s| !s.is_empty())) else {
        return Err(format!("can't tell the repo from the origin {origin}"));
    };
    let repo = format!("{owner}/{name}");
    // GitHub ignores case; the PR page's spelling is the one later requests come with.
    let repo = match expected {
        Some(expected) if !expected.eq_ignore_ascii_case(&repo) => {
            return Err(format!("{} is a clone of {repo}, not of {expected}", clone.display()));
        }
        Some(expected) => expected.to_owned(),
        None => repo,
    };
    let path = clone.to_string_lossy().into_owned();
    core.settings.update(|s| s.repos.entry(repo.clone()).or_default().path = path)?;
    Ok(repo)
}

pub fn remove_repo(core: &Core, repo: &str) -> Result<(), String> {
    core.settings.update(|s| drop(s.repos.remove(repo)))
}

/// A repo as error messages name it.
pub fn repo_name(repo: &Value) -> String {
    repo.as_str().map_or_else(|| repo.to_string(), str::to_owned)
}

/// The folder new worktrees go in. None: next to the main clone.
pub fn worktree_dir(core: &Core, settings: &Settings) -> Result<Option<PathBuf>, String> {
    if settings.worktree_dir.is_empty() {
        return Ok(None);
    }
    let dir = Some(core.paths.expand_user(&settings.worktree_dir)).filter(|dir| dir.is_absolute() && dir.is_dir());
    dir.map(Some).ok_or_else(|| format!("the worktree folder {} does not exist; change it in Settings", settings.worktree_dir))
}
