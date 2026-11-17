//! The PR's worktree: found or created, then brought up to date.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cmd;
use crate::platform::Paths;

/// Gitignored files a fresh worktree needs to build.
const COPY_FILES: [&str; 1] = ["local.properties"];

fn git(paths: &Paths, dir: &Path) -> Command {
    let mut git = cmd::command(paths, "git");
    git.arg("-C").arg(dir);
    git
}

/// The branch name ends up in git arguments and the worktree path: no options, no "..". A fork's "owner:branch"
/// fails here too.
pub fn valid_branch(paths: &Paths, branch: &str) -> bool {
    let check = cmd::run(cmd::command(paths, "git").args(["check-ref-format", &format!("refs/heads/{branch}")]));
    !branch.starts_with('-') && check.is_ok()
}

/// The folder (main clone or worktree) that has the branch checked out.
pub fn worktree_for_branch(paths: &Paths, root: &Path, branch: &str) -> Result<Option<PathBuf>, String> {
    let mut path = None;
    for line in cmd::run(git(paths, root).args(["worktree", "list", "--porcelain"]))?.lines() {
        if let Some(worktree) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(worktree));
        } else if line.strip_prefix("branch refs/heads/") == Some(branch) {
            return Ok(path);
        }
    }
    Ok(None)
}

/// The branch's worktree, created as `<clone>-<branch>` in `base_dir` (default: next to the main clone) if the
/// branch isn't checked out anywhere yet.
pub fn ensure_worktree(paths: &Paths, root: &Path, branch: &str, base_dir: Option<&Path>) -> Result<PathBuf, String> {
    // Branch already checked out (main clone or another worktree): use that instead of a second copy.
    let path = match worktree_for_branch(paths, root, branch)? {
        Some(path) => path,
        None => {
            let name = format!("{}-{}", root.file_name().unwrap_or_default().to_string_lossy(), branch.replace('/', "-"));
            let path = base_dir.or(root.parent()).unwrap_or(root).join(name);
            if !path.exists() {
                cmd::run(git(paths, root).args(["worktree", "add", "--detach"]).arg(&path))?;
                if let Err(e) = update(paths, &path, branch) {
                    let _ = cmd::run(git(paths, root).args(["worktree", "remove", "--force"]).arg(&path));
                    return Err(e);
                }
            }
            path
        }
    };
    // Worktrees made elsewhere can lack these too; an existing copy is never overwritten.
    for name in COPY_FILES {
        let (src, dst) = (root.join(name), path.join(name));
        if src.exists() && !dst.exists() {
            fs::copy(&src, &dst).map_err(|e| format!("can't copy {name} into {}: {e}", path.display()))?;
        }
    }
    // Android Studio and the Claude app match folders by resolved path; same path = same window/session.
    path.canonicalize().map_err(|e| format!("can't resolve {}: {e}", path.display()))
}

/// True only when origin answers that it has no such branch; an unreachable origin doesn't count.
pub fn branch_deleted(paths: &Paths, root: &Path, branch: &str) -> bool {
    let listed = git(paths, root).args(["ls-remote", "--exit-code", "origin", &format!("refs/heads/{branch}")]).output();
    listed.is_ok_and(|output| output.status.code() == Some(2))
}

/// Fetch the branch, switch to it (a new local branch tracks origin's), then only fast-forward: local commits or
/// changes it would overwrite stop it with an error instead of being lost.
pub fn update(paths: &Paths, path: &Path, branch: &str) -> Result<(), String> {
    // shortcut: the branch must live on the clone's "origin"; PRs from forks would need their own remote.
    cmd::run(git(paths, path).args(["fetch", "origin", &format!("+refs/heads/{branch}:refs/remotes/origin/{branch}")]))?;
    cmd::run(git(paths, path).args(["-c", "checkout.defaultRemote=origin", "switch", branch]))?;
    cmd::run(git(paths, path).args(["merge", "--ff-only", &format!("refs/remotes/origin/{branch}")]))?;
    Ok(())
}
