//! Runs the tools (git, adb, xcrun, open, …): always a `Command` with separate arguments, never a shell.

use std::ffi::OsStr;
use std::process::{Command, Output, Stdio};

use crate::platform::Paths;

pub fn command(paths: &Paths, program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    cmd.env("PATH", &paths.path).stdin(Stdio::null());
    cmd
}

/// The command's trimmed stdout, or an error naming the command and what it printed.
pub fn run(cmd: &mut Command) -> Result<String, String> {
    let output = cmd.output();
    checked(cmd, output)
}

pub fn checked(cmd: &Command, output: std::io::Result<Output>) -> Result<String, String> {
    let name: Vec<_> = [cmd.get_program()].into_iter().chain(cmd.get_args().take(2)).map(OsStr::to_string_lossy).collect();
    let name = name.join(" ");
    let output = output.map_err(|e| format!("{name} failed: {e}"))?;
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_owned();
    if output.status.success() {
        return Ok(text(&output.stdout));
    }
    let stderr = text(&output.stderr);
    Err(format!("{name} failed: {}", if stderr.is_empty() { text(&output.stdout) } else { stderr }))
}
