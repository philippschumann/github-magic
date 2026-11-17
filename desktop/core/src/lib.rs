//! Logic shared by the app and the bridge. Phase 2 moves the Python script's actions here.

use std::ffi::{CStr, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

/// Native messaging caps host replies at 1 MB; requests are tiny. Both sides refuse bigger frames.
pub const MAX_FRAME: usize = 1024 * 1024;

/// The app's socket, in the per-user private temp dir: `~/Library/Application Support/…` can exceed
/// the 104-byte limit of a Unix socket path.
pub fn socket_path() -> PathBuf {
    let mut buf = [0u8; libc::PATH_MAX as usize];
    // SAFETY: buf is writable for buf.len() bytes; confstr NUL-terminates what it writes.
    let len = unsafe { libc::confstr(libc::_CS_DARWIN_USER_TEMP_DIR, buf.as_mut_ptr().cast(), buf.len()) };
    assert!(len > 0 && len <= buf.len(), "confstr(_CS_DARWIN_USER_TEMP_DIR) failed");
    let dir = CStr::from_bytes_until_nul(&buf).expect("NUL-terminated");
    PathBuf::from(OsStr::from_bytes(dir.to_bytes())).join("github-magic.sock")
}

#[cfg(test)]
mod tests {
    #[test]
    fn socket_path_is_in_the_private_temp_dir() {
        let path = super::socket_path();
        assert!(path.starts_with("/var/folders/") || path.starts_with("/private/var/folders/"), "{path:?}");
        assert!(path.as_os_str().len() < 104, "{path:?}");
    }
}
