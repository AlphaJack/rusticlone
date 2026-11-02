//! What every Unix answers the same way. Each Unix's own file uses it.

use std::path::{Path, PathBuf};

use super::HOSTNAME_BUFFER;

/// Where `install` places rusticlone for root.
pub(crate) fn system_bin_dir() -> PathBuf {
    PathBuf::from("/usr/local/bin")
}

/// Whether this process may create files in `dir`, or in the folder that creating `dir` would start from.
/// Unix asks the kernel, which also counts read-only mounts and ACLs.
#[cfg(any(feature = "server", feature = "self-update"))]
#[expect(unsafe_code, reason = "faccessat reads the NUL-terminated path it is given")]
pub(crate) fn writable(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    for dir in dir.ancestors() {
        let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else { return false };
        // SAFETY: `path` is NUL-terminated and outlives the call.
        if unsafe { libc::faccessat(libc::AT_FDCWD, path.as_ptr(), libc::W_OK | libc::X_OK, libc::AT_EACCESS) } == 0 {
            return true;
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::NotFound {
            return false;
        }
    }
    false
}

#[expect(unsafe_code, reason = "gethostname fills a buffer of the length given")]
pub(super) fn read_hostname() -> Option<String> {
    let mut name = [0u8; HOSTNAME_BUFFER];
    // SAFETY: a buffer of the length given.
    if unsafe { libc::gethostname(name.as_mut_ptr().cast(), name.len()) } != 0 {
        return None;
    }
    Some(std::ffi::CStr::from_bytes_until_nul(&name).ok()?.to_string_lossy().into_owned())
}

#[expect(unsafe_code, reason = "geteuid has no preconditions")]
pub(crate) fn is_root() -> bool {
    // SAFETY: no preconditions.
    unsafe { libc::geteuid() == 0 }
}

/// The device of `path`, or of its nearest existing ancestor.
#[expect(clippy::disallowed_methods, reason = "reads the device of `path`")]
pub(crate) fn device(path: &Path) -> Option<u64> {
    let meta = path.ancestors().find_map(|p| std::fs::metadata(p).ok())?;
    Some(std::os::unix::fs::MetadataExt::dev(&meta))
}

/// The space left to this user on the disk that holds `path`, or would hold it: that of its nearest existing
/// ancestor, which it returns too, since an unmounted disk's folder is on the system's disk.
#[cfg(feature = "server")]
#[expect(unsafe_code, reason = "statvfs fills the structure it is given")]
#[expect(clippy::disallowed_methods, reason = "finds the nearest existing ancestor of `path`")]
pub(crate) fn free_space(path: &Path) -> Option<(&Path, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let existing = path.ancestors().find(|p| p.is_dir())?;
    let path = std::ffi::CString::new(existing.as_os_str().as_bytes()).ok()?;
    // SAFETY: integers, for which zeros are valid.
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a NUL-terminated path, and a structure to fill.
    let read = unsafe { libc::statvfs(path.as_ptr(), &raw mut stats) } == 0;
    #[cfg_attr(
        any(target_vendor = "apple", target_os = "linux"),
        expect(clippy::useless_conversion, reason = "narrower on other systems")
    )]
    read.then(|| (existing, u64::from(stats.f_bavail) * u64::from(stats.f_frsize)))
}

/// Whether stdout interprets ANSI escape codes: every terminal outside Windows does.
pub(crate) fn ansi() -> bool {
    true
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    use crate::sys::{device, is_root, test_dir, writable};

    /// A folder is writable, and so is one not created yet below it, until it is read-only.
    #[test]
    fn a_read_only_folder_is_not_writable() {
        if is_root() {
            return;
        }
        let dir = test_dir("writable");
        assert!(writable(&dir) && writable(&dir.join("rustic")));
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        let read_only = !writable(&dir) && !writable(&dir.join("rustic"));
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(read_only);
    }

    /// Runs on the same volume wait for each other on a spinning disk.
    #[test]
    fn paths_on_a_volume_share_its_device() {
        let root = std::path::Path::new("/");
        assert!(device(root).is_some());
        assert_eq!(device(&root.join("not yet").join("created")), device(root));
    }
}
