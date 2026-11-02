//! What every Unix answers the same way. Each Unix's own file uses it.

use std::path::{Path, PathBuf};

use super::HOSTNAME_BUFFER;

/// `app`'s configuration directory for the system.
#[expect(clippy::unnecessary_wraps, reason = "Windows's may be unknown")]
pub(crate) fn system_config_dir(app: &str) -> Option<PathBuf> {
    Some(PathBuf::from("/etc").join(app))
}

/// The drive letter of a Windows path such as `C:\Users`, and what follows it.
pub(crate) fn windows_drive(path: &str) -> Option<(char, &str)> {
    let mut chars = path.chars();
    let (letter, colon, separator) = (chars.next()?, chars.next()?, chars.next()?);
    let windows = letter.is_ascii_alphabetic() && colon == ':' && (separator == '\\' || separator == '/');
    windows.then(|| (letter, path.get(2..).unwrap_or_default()))
}

/// Whether `path` is an absolute path of another system: a Windows drive's. Its folder has no place here.
pub(crate) fn foreign_path(path: &str) -> bool {
    windows_drive(path).is_some()
}

/// Where `install` places rusticlone for root.
pub(crate) fn system_bin_dir() -> PathBuf {
    PathBuf::from("/usr").join("local").join("bin")
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
    read.then(|| (existing, to_u64(stats.f_bavail) * to_u64(stats.f_frsize)))
}

/// `statvfs` counts in integers narrower than `u64` on some systems.
#[cfg(feature = "server")]
fn to_u64(n: impl Into<u64>) -> u64 {
    n.into()
}

/// Whether stdout interprets ANSI escape codes: every terminal outside Windows does.
pub(crate) fn ansi() -> bool {
    true
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    use crate::sys::{device, is_root, test_dir, writable};

    /// Another system's absolute folders, which have no place here.
    #[test]
    fn a_windows_drive_is_foreign() {
        assert!(
            super::foreign_path(r"C:\Users\me") && super::foreign_path("d:/data") && !super::foreign_path("/home/me")
        );
    }

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

/// The system packages' folders, which root can write too.
#[cfg(any(feature = "desktop", feature = "self-update"))]
pub(crate) fn system_bins() -> Vec<PathBuf> {
    vec![Path::new("/usr").join("bin"), PathBuf::from("/bin"), Path::new("/usr").join("sbin")]
}

/// Windows's alone: a terminal's process here has no console of its own.
#[cfg(feature = "server")]
pub(crate) fn release_own_console() {}

/// The installer's command here, without `install`: on a new computer, the server would back up the folders
/// before they are restored.
#[cfg(feature = "server")]
pub(crate) const INSTALLER: &str = concat!(
    "curl -LsSf ",
    env!("CARGO_PKG_REPOSITORY"),
    "/releases/latest/download/",
    env!("CARGO_PKG_NAME"),
    "-installer.sh | sh"
);

/// Read and written by its owner alone.
#[cfg(feature = "server")]
const OWNER_ONLY: u32 = 0o600;

/// Lets only this user read and write the file `options` opens.
#[cfg(feature = "server")]
pub(crate) fn only_owner(options: &mut std::fs::OpenOptions) {
    std::os::unix::fs::OpenOptionsExt::mode(options, OWNER_ONLY);
}
