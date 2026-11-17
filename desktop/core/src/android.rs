//! Android device runs: pick the phone or boot the emulator, install the debug build, start the app.

use std::fs;
use std::path::{Path, PathBuf};

/// The Android SDK the repo builds with: `sdk.dir` in its local.properties.
pub fn sdk_dir(path: &Path) -> Result<PathBuf, String> {
    let file = path.join("local.properties");
    let text = fs::read_to_string(&file).map_err(|e| format!("can't read {}: {e}", file.display()))?;
    let dir = text.lines().find_map(|line| line.strip_prefix("sdk.dir="));
    dir.map(|dir| PathBuf::from(dir.trim())).ok_or_else(|| format!("no sdk.dir in {}", file.display()))
}
