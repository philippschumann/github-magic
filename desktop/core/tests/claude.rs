mod common;

use std::path::Path;

use common::{calls, paths, tmp, write};
use github_magic_core::claude::open_claude;
use serde_json::json;

#[test]
fn reopens_newest_unarchived_session_in_folder() {
    let tmp = tmp();
    let paths = paths(&tmp);
    for (name, id, cwd, archived, last) in [
        ("a", "local_old", "/w/x", false, 1),
        ("b", "local_new", "/w/x", false, 5),
        ("c", "local_arch", "/w/x", true, 9),
        ("d", "local_other", "/w/y", false, 9),
    ] {
        let session = json!({ "sessionId": id, "cwd": cwd, "isArchived": archived, "lastActivityAt": last });
        write(paths.sessions_dir.join(format!("acct/local_{name}.json")), &session.to_string());
    }
    open_claude(&paths, Path::new("/w/x"), "ignored").unwrap();
    assert_eq!(calls(&tmp.join("bin")), ["open claude://code/continue?session=local_new"]);
}

#[test]
fn new_session_composer_when_none() {
    let tmp = tmp();
    open_claude(&paths(&tmp), Path::new("/w/my repo"), "Fix login & logout (#12): ").unwrap();
    let url = "claude://code/new?folder=%2Fw%2Fmy%20repo&q=Fix%20login%20%26%20logout%20%28%2312%29%3A%20";
    assert_eq!(calls(&tmp.join("bin")), [format!("open {url}")]);
}
