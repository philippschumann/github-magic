//! Everything that differs between systems: where apps live, which PATH the tools need, where Java is.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::cmd;

/// IDE name -> app bundles to look for in `Paths::app_dirs`; the first one found is opened.
pub const IDES: &[(&str, &[&str])] = &[
    ("Android Studio", &["Android Studio.app"]),
    ("IntelliJ IDEA", &["IntelliJ IDEA.app", "IntelliJ IDEA Ultimate.app", "IntelliJ IDEA CE.app"]),
    ("VS Code", &["Visual Studio Code.app"]),
];
/// The same for iOS repos. Xcode gets the project file; iPhone and simulator runs need it too.
pub const IOS_IDES: &[(&str, &[&str])] = &[("Xcode", &["Xcode.app", "Xcode-beta.app"]), ("VS Code", &["Visual Studio Code.app"])];

/// Where things live. Tests point these at temp dirs.
pub struct Paths {
    pub home: PathBuf,
    pub app_dirs: Vec<PathBuf>,
    /// Each emulator has a `<name>.ini` here.
    pub avd_dir: PathBuf,
    pub sessions_dir: PathBuf,
    pub log_dir: PathBuf,
    /// PATH of every command the core starts.
    pub path: OsString,
}

impl Paths {
    pub fn system() -> Self {
        let home = std::env::home_dir().expect("a home directory");
        // LaunchServices starts the app with a minimal PATH; keep Homebrew's tools (git, anything the build calls)
        // reachable.
        let mut path = OsString::from("/opt/homebrew/bin:/usr/local/bin:");
        path.push(std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()));
        Self {
            app_dirs: vec!["/Applications".into(), home.join("Applications")], // ~/Applications: JetBrains Toolbox
            avd_dir: home.join(".android/avd"),
            sessions_dir: home.join("Library/Application Support/Claude/claude-code-sessions"),
            log_dir: home.join("Library/Logs/com.philippschumann.github-magic"),
            path,
            home,
        }
    }

    /// `~` and `~/…` become the home folder, like a shell does.
    pub fn expand_user(&self, path: &str) -> PathBuf {
        match path.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => self.home.join(rest.trim_start_matches('/')),
            _ => PathBuf::from(path),
        }
    }
}

/// Path of the first of these app bundles found in the app dirs.
pub fn find_app(paths: &Paths, apps: &[&str]) -> Option<PathBuf> {
    apps.iter().flat_map(|app| paths.app_dirs.iter().map(move |dir| dir.join(app))).find(|p| p.is_dir())
}

/// Installed IDEs (of `IDES` or `IOS_IDES`) as name and app bundle path, in the list's order.
pub fn installed_ides(paths: &Paths, ides: &[(&'static str, &[&str])]) -> Vec<(&'static str, PathBuf)> {
    ides.iter().filter_map(|(name, apps)| Some((*name, find_app(paths, apps)?))).collect()
}

/// JAVA_HOME for Gradle: Java 21 if installed, else the default one.
pub fn java_home(paths: &Paths) -> String {
    let java_home = |args: &[&str]| cmd::run(cmd::command(paths, "/usr/libexec/java_home").args(args));
    java_home(&["-v", "21"]).or_else(|_| java_home(&[])).unwrap_or_default()
}

pub fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
