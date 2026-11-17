//! Shared by the integration tests: temp dirs and the stand-in tools from tests/fixtures.
#![allow(dead_code)] // each test file uses its own part

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use github_magic_core::platform::Paths;

/// A fresh temp dir, resolved like the paths the core returns.
pub fn tmp() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("gm-test-{nanos}-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// Copies a stand-in tool into `dir`, where it logs its calls to calls.log.
pub fn fixture(name: &str, dir: &Path) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name), dir.join(name)).unwrap();
    dir.join(name)
}

/// Everything inside `tmp`, with stand-ins for xcrun, xcodebuild and open first on the PATH (`tmp/bin`).
pub fn paths(tmp: &Path) -> Paths {
    let bin = tmp.join("bin");
    for tool in ["xcrun", "xcodebuild", "open"] {
        fixture(tool, &bin);
    }
    Paths {
        home: tmp.join("home"),
        app_dirs: vec![tmp.join("Applications"), tmp.join("home/Applications")],
        avd_dir: tmp.join("avd"),
        sessions_dir: tmp.join("sessions"),
        log_dir: tmp.join("logs"),
        path: format!("{}:/usr/bin:/bin", bin.display()).into(),
    }
}

/// The calls a stand-in tool in `dir` logged.
pub fn calls(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join("calls.log")).unwrap_or_default().lines().map(str::to_owned).collect()
}

pub fn write(path: impl AsRef<Path>, text: &str) {
    fs::create_dir_all(path.as_ref().parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
