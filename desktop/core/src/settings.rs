//! The user's repos and app.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::cmd;
use crate::platform::Paths;

/// shortcut: phase 2 still reads native-host/config.py of this checkout; phase 3 replaces it with settings.json.
pub const CONFIG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../native-host/config.py");
const PRINT_CONFIG: &str = r#"
import json, os, sys
sys.path.insert(0, os.path.dirname(sys.argv[1]))
try:
    import config
except ImportError:
    config = None
print(json.dumps({k: getattr(config, k, None) for k in ("REPOS", "APP_ID", "IOS_SCHEME", "PHONE_MODELS")}))
"#;

#[derive(Default)]
pub struct Config {
    /// GitHub repo ("owner/name") -> local main clone.
    pub repos: BTreeMap<String, String>,
    pub app_id: String,
    pub ios_scheme: String,
    /// `adb devices -l` substrings, preferred in this order, else any other phone.
    pub phone_models: Vec<String>,
}

impl Config {
    /// Read once at start: the app must be restarted after config.py changes. Empty without a config.py.
    pub fn load(paths: &Paths) -> Self {
        let printed = cmd::run(cmd::command(paths, "/usr/bin/python3").args(["-c", PRINT_CONFIG, CONFIG]));
        let config: Value = printed.ok().and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default();
        let text = |key: &str| config[key].as_str().unwrap_or_default().to_owned();
        let strings = |value: &Value| value.as_str().map(str::to_owned);
        Self {
            repos: config["REPOS"]
                .as_object()
                .map(|repos| repos.iter().filter_map(|(repo, path)| Some((repo.clone(), strings(path)?))).collect())
                .unwrap_or_default(),
            app_id: text("APP_ID"),
            ios_scheme: text("IOS_SCHEME"),
            phone_models: config["PHONE_MODELS"].as_array().map(|m| m.iter().filter_map(strings).collect()).unwrap_or_default(),
        }
    }
}

/// The folder new worktrees go in, as sent by the extension. None: next to the main clone.
pub fn worktree_dir(paths: &Paths, value: &Value) -> Result<Option<PathBuf>, String> {
    if value.is_null() || value == "" {
        return Ok(None);
    }
    let dir = value.as_str().map(|dir| paths.expand_user(dir)).filter(|dir| dir.is_absolute() && dir.is_dir());
    dir.map(Some).ok_or_else(|| format!("worktree folder {value} does not exist; change it in the extension's settings"))
}
