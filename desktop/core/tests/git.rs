mod common;

use std::fs;
use std::path::Path;

use common::{Repo, git, head, paths, tmp, write};
use github_magic_core::git::{branch_deleted, ensure_worktree, update};

#[test]
fn existing_worktree_gets_missing_local_properties_but_keeps_its_own() {
    let tmp = tmp();
    let paths = paths(&tmp);
    let root = tmp.join("App");
    let dir = |path: &Path| path.to_str().unwrap().to_owned();
    git(&["init", "-q", "-b", "main", &dir(&root)]);
    git(&["-C", &dir(&root), "commit", "-q", "--allow-empty", "-m", "x"]);
    git(&["-C", &dir(&root), "worktree", "add", "-q", "-b", "feat/a", &dir(&tmp.join("elsewhere"))]);
    git(&["-C", &dir(&root), "worktree", "add", "-q", "-b", "feat/b", &dir(&tmp.join("own"))]);
    write(root.join("local.properties"), "sdk.dir=/main\n");
    write(tmp.join("own/local.properties"), "sdk.dir=/own\n");

    let path = ensure_worktree(&paths, &root, "feat/a", None).unwrap();
    assert_eq!(path, tmp.join("elsewhere"));
    assert_eq!(fs::read_to_string(path.join("local.properties")).unwrap(), "sdk.dir=/main\n");
    let path = ensure_worktree(&paths, &root, "feat/b", None).unwrap();
    assert_eq!(fs::read_to_string(path.join("local.properties")).unwrap(), "sdk.dir=/own\n");
}

#[test]
fn new_worktree_tracks_origin_and_updates_fast_forward_only() {
    let tmp = tmp();
    let paths = paths(&tmp);
    let repo = Repo::new(&tmp, "App");
    let pushed = repo.push_commit("feat/a"); // the main clone has never fetched feat/a

    let path = ensure_worktree(&paths, &repo.root, "feat/a", Some(&tmp)).unwrap();
    assert_eq!(path, tmp.join("App-feat-a"));
    let dir = path.to_str().unwrap();
    assert_eq!(git(&["-C", dir, "rev-parse", "--abbrev-ref", "HEAD", "@{upstream}"]), "feat/a\norigin/feat/a");
    assert_eq!(head(&path), pushed);

    let pushed = repo.push_commit("feat/a");
    update(&paths, &path, "feat/a").unwrap();
    assert_eq!(head(&path), pushed);

    git(&["-C", dir, "commit", "-q", "--allow-empty", "-m", "local"]);
    let local = head(&path);
    repo.push_commit("feat/a");
    assert!(update(&paths, &path, "feat/a").unwrap_err().starts_with("git -C"));
    assert_eq!(head(&path), local); // the local commit is kept
}

#[test]
fn branch_deleted_only_when_origin_says_so() {
    let tmp = tmp();
    let paths = paths(&tmp);
    let repo = Repo::new(&tmp, "App");
    repo.push_commit("feat/a");
    assert!(!branch_deleted(&paths, &repo.root, "feat/a"));
    assert!(branch_deleted(&paths, &repo.root, "feat/gone"));
    assert!(branch_deleted(&paths, &repo.root, "a")); // no suffix match on feat/a
    git(&["-C", repo.root.to_str().unwrap(), "remote", "set-url", "origin", tmp.join("unreachable.git").to_str().unwrap()]);
    assert!(!branch_deleted(&paths, &repo.root, "feat/gone")); // offline: carry on as before
}
