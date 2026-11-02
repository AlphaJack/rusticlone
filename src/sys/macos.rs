//! macOS's answers, from `statfs`, `diskutil` and its privacy rules.

use std::path::Path;

use etcetera::BaseStrategy;

pub(super) use super::unix::read_hostname;
pub(crate) use super::unix::*;

/// `app`'s configuration directory for this user, where the `directories` crate that rustic uses puts it.
pub(crate) fn user_config_dir(base: &impl BaseStrategy, app: &str) -> std::path::PathBuf {
    base.data_dir().join(app)
}

/// Whether the process `pid` ended, for test directories to go. Unknown but on Linux.
#[cfg(test)]
pub(crate) fn ended(_: &str) -> bool {
    false
}

/// Whether macOS keeps this process out of folders in `path` until the process has Full Disk Access:
/// users' `Library`, which holds mail, messages and browser data. Documents, Desktop, Downloads and
/// other volumes need no such access: macOS asks the user at the first read.
#[expect(clippy::disallowed_methods, reason = "probes the system's privacy database, which macOS protects")]
pub(crate) fn needs_disk_access(path: &str) -> bool {
    let mut parts = Path::new(path).components().skip(1).map(|c| c.as_os_str().to_str().unwrap_or_default());
    let protected =
        matches!((parts.next(), parts.next(), parts.next()), (None, ..) | (Some("Users"), _, None | Some("Library")));
    protected && std::fs::File::open("/Library/Application Support/com.apple.TCC/TCC.db").is_err()
}

/// From `diskutil`, for the device the file system is mounted from.
#[expect(unsafe_code, reason = "statfs fills a zeroed struct for a NUL-terminated path")]
#[expect(clippy::disallowed_methods, reason = "runs diskutil for the disk of `path`")]
pub(super) fn solid_state(path: &Path) -> Option<bool> {
    use std::os::unix::ffi::OsStrExt;
    let existing = path.ancestors().find(|p| p.exists())?;
    let path = std::ffi::CString::new(existing.as_os_str().as_bytes()).ok()?;
    // SAFETY: a valid C string and a struct statfs fills.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &raw mut stat) } != 0 {
        return None;
    }
    // SAFETY: statfs NUL-terminates the name.
    let from = unsafe { std::ffi::CStr::from_ptr(stat.f_mntfromname.as_ptr()) }.to_str().ok()?.to_owned();
    let info = std::process::Command::new("diskutil").args(["info", "-plist", &from]).output().ok()?;
    let plist = String::from_utf8(info.stdout).ok()?;
    let value = plist.split_once("<key>SolidState</key>")?.1.trim_start();
    [("<true/>", true), ("<false/>", false)].into_iter().find_map(|(tag, v)| value.starts_with(tag).then_some(v))
}

#[expect(unsafe_code, reason = "statfs fills a zeroed struct for a NUL-terminated path")]
pub(super) fn network_fs(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    const NETWORK: [&str; 8] = ["nfs", "smbfs", "afpfs", "webdav", "cifs", "macfuse", "osxfuse", "fusefs"];
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    // SAFETY: a valid C string and a struct statfs fills.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &raw mut stat) } != 0 {
        return false;
    }
    // SAFETY: statfs NUL-terminates the name.
    let name = unsafe { std::ffi::CStr::from_ptr(stat.f_fstypename.as_ptr()) }.to_string_lossy();
    NETWORK.iter().any(|n| name.starts_with(n))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{needs_disk_access, solid_state};

    /// The startup disk says whether it is solid-state, if `diskutil` knows; the same below a folder not created
    /// yet.
    #[test]
    fn a_local_disk_says_whether_it_seeks() {
        let root = Path::new("/");
        assert_eq!(solid_state(&root.join("not yet").join("created")), solid_state(root));
    }

    /// Folders that macOS asks about at the first read need no Full Disk Access.
    #[test]
    fn only_a_home_with_its_library_needs_disk_access() {
        let full = !needs_disk_access("/Users/me");
        for path in ["/", "/Users", "/Users/me", "/Users/me/Library/Mail"] {
            assert_eq!(needs_disk_access(path), !full, "{path}");
        }
        for path in ["/Users/me/Documents", "/Volumes/usb", "/opt/data"] {
            assert!(!needs_disk_access(path), "{path}");
        }
    }

    /// The system volume is read-only, even for root.
    #[cfg(any(feature = "server", feature = "self-update"))]
    #[test]
    fn the_system_volume_is_not_writable() {
        assert!(!crate::sys::writable(&Path::new("/usr").join("bin")));
    }
}
