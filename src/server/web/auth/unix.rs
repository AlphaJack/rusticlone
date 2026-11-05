//! What only Unix lets a test make: a folder this user may not write in.

use std::os::unix::fs::PermissionsExt;

use super::{TOKEN_FILE, saved_token};

/// A token that cannot be saved says why, rather than that there is none to read.
#[test]
fn a_token_that_cannot_be_saved_says_why() {
    if crate::sys::is_root() {
        return;
    }
    let dir = crate::sys::test_dir("token-read-only");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let error = saved_token(&dir.join(TOKEN_FILE)).unwrap_err();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let kind = error.downcast_ref::<std::io::Error>().map(std::io::Error::kind);
    assert_eq!(kind, Some(std::io::ErrorKind::PermissionDenied), "{error:#}");
}
