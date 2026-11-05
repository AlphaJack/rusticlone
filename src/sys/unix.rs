//! What every Unix answers the same way. Each Unix's own file uses it.

use std::path::{Path, PathBuf};

use super::HOSTNAME_BUFFER;

/// Moves `from` over `to`, which a running copy of it keeps as it was.
#[cfg(feature = "desktop")]
#[expect(clippy::disallowed_methods, reason = "replaces an installed file")]
pub(crate) fn replace(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

/// What a command needs before it to run as root, as a command line for an administrator shows it.
#[cfg(feature = "server")]
pub(crate) const AS_ROOT: &str = "sudo ";

/// `app`'s configuration directory for the system.
#[expect(clippy::unnecessary_wraps, reason = "Windows's may be unknown")]
pub(crate) fn system_config_dir(app: &str) -> Option<PathBuf> {
    Some(PathBuf::from("/etc").join(app))
}

/// Whether `\` separates the parts of a path, as `/` does. Here it is a character of a name.
pub(crate) const BACKSLASH_SEPARATES: bool = false;

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
pub(crate) fn writable(dir: &Path) -> bool {
    use nix::{
        errno::Errno,
        fcntl::AtFlags,
        unistd::{AccessFlags, faccessat},
    };
    // Writing to it and entering it.
    let create = AccessFlags::W_OK.union(AccessFlags::X_OK);
    for dir in dir.ancestors() {
        match faccessat(nix::fcntl::AT_FDCWD, dir, create, AtFlags::AT_EACCESS) {
            Ok(()) => return true,
            Err(Errno::ENOENT) => {}
            Err(_) => return false,
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
    #[expect(clippy::disallowed_methods, reason = "reads the device as the kernel gives it")]
    fn paths_on_a_volume_share_its_device() {
        let root = std::path::Path::new("/");
        let dev = std::os::unix::fs::MetadataExt::dev(&fs::metadata(root).unwrap());
        assert_eq!(device(root), Some(dev));
        assert_eq!(device(&root.join("not yet").join("created")), device(root));
    }

    /// The space left on a disk, as `df` counts it in KiB. Files written meanwhile change it a little.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "asks df, and removes its folder")]
    fn free_space_is_what_df_says() {
        let dir = test_dir("free-space");
        let missing = dir.join("not yet");
        let (existing, free) = super::free_space(&missing).unwrap();
        assert_eq!(existing, dir);
        let df = std::process::Command::new("df").arg("-Pk").arg(&dir).output().unwrap();
        let text = String::from_utf8(df.stdout).unwrap();
        let available: u64 = text.lines().nth(1).unwrap().split_whitespace().nth(3).unwrap().parse().unwrap();
        assert!(free.abs_diff(available * 1024) < 256 << 20, "{free} against {text}");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Where every Unix keeps what is the system's, and that its terminals read ANSI codes.
    #[test]
    fn unix_answers_the_same_way() {
        use std::path::Path;
        let usr = Path::new("/usr");
        assert_eq!(super::system_config_dir("app"), Some(Path::new("/etc").join("app")));
        assert_eq!(super::system_bin_dir(), usr.join("local").join("bin"));
        #[cfg(any(feature = "desktop", feature = "self-update"))]
        assert_eq!(super::system_bins(), [usr.join("bin"), std::path::PathBuf::from("/bin"), usr.join("sbin")]);
        assert!(super::ansi());
        let base = etcetera::base_strategy::Xdg::new().unwrap();
        let state = etcetera::BaseStrategy::state_dir(&base).unwrap();
        assert_eq!(super::user_state_dir(&base, "app"), state.join("app"));
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

/// Read and written by its owner alone.
#[cfg(any(feature = "server", all(feature = "desktop", feature = "self-update")))]
const OWNER_ONLY: u32 = 0o600;

/// Lets only this user read and write the file `options` opens.
#[cfg(any(feature = "server", all(feature = "desktop", feature = "self-update")))]
pub(crate) fn only_owner(options: &mut std::fs::OpenOptions) {
    std::os::unix::fs::OpenOptionsExt::mode(options, OWNER_ONLY);
}

/// Where `app` keeps its state for this user: machine-local, not the purgeable caches.
pub(crate) fn user_state_dir(base: &impl etcetera::BaseStrategy, app: &str) -> PathBuf {
    base.state_dir().unwrap_or_else(|| base.data_dir()).join(app)
}
