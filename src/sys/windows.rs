//! Windows's answers, from the Win32 API.

use std::path::{Path, PathBuf};

use etcetera::BaseStrategy;

use super::{HOSTNAME_BUFFER, fnv1a};

/// Whether a variable moved one of the XDG folders: only Linux reads them.
#[cfg(test)]
pub(crate) fn xdg_moved() -> bool {
    false
}

/// How the book writes `path`: under the folder a variable names, `%APPDATA%\rustic`.
#[cfg(test)]
#[expect(clippy::disallowed_methods, reason = "the book names folders by the variables that hold them")]
pub(crate) fn as_documented(path: &Path) -> String {
    let vars = ["LOCALAPPDATA", "APPDATA", "PROGRAMDATA", "WINDIR"];
    vars.iter()
        .find_map(|var| Some(format!("%{var}%\\{}", path.strip_prefix(std::env::var_os(var)?).ok()?.display())))
        .unwrap_or_else(|| path.display().to_string())
}

/// `app`'s configuration directory for this user, laid out as the `directories` crate that rustic uses lays
/// it out.
pub(crate) fn user_config_dir(base: &impl BaseStrategy, app: &str) -> PathBuf {
    base.config_dir().join(app).join("config")
}

/// `app`'s configuration directory for the system, in `ProgramData`.
#[expect(clippy::disallowed_methods, reason = "the system's directory on Windows is in the environment")]
pub(crate) fn system_config_dir(app: &str) -> Option<PathBuf> {
    std::env::var_os("PROGRAMDATA").map(|data| PathBuf::from(data).join(app).join("config"))
}

/// Whether `path` is an absolute path of another system: a Unix root's. Its folder has no place here.
pub(crate) fn foreign_path(path: &str) -> bool {
    path.starts_with('/')
}

/// Whether the process `pid` ended, for test directories to go. Unknown here.
#[cfg(test)]
pub(crate) fn ended(_: &str) -> bool {
    false
}

/// Where `install` places rusticlone for an administrator.
#[expect(clippy::disallowed_methods, reason = "the system's program folder on Windows is in the environment")]
pub(crate) fn system_bin_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_else(|| r"C:\Program Files".into())).join(crate::APP_NAME)
}

/// Windows's own folders are writable by whom they belong to, so it assumes they are.
#[cfg(any(feature = "server", feature = "self-update"))]
pub(crate) fn writable(_: &Path) -> bool {
    true
}

/// The DNS host name, as `hostname` prints, rather than the `NetBIOS` name.
#[expect(unsafe_code, reason = "GetComputerNameExW fills a buffer of the length given")]
pub(super) fn read_hostname() -> Option<String> {
    use windows_sys::Win32::System::SystemInformation::{ComputerNamePhysicalDnsHostname, GetComputerNameExW};
    let mut name = [0u16; HOSTNAME_BUFFER];
    let mut len = u32::try_from(name.len()).ok()?;
    // SAFETY: a buffer of the length given, which becomes the length written.
    if unsafe { GetComputerNameExW(ComputerNamePhysicalDnsHostname, name.as_mut_ptr(), &raw mut len) } == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(name.get(..usize::try_from(len).ok()?)?))
}

/// Whether this process is elevated, as an administrator's shell or a system service is: it can install a
/// service, and reads and writes the `ProgramData` collections.
#[expect(unsafe_code, reason = "GetTokenInformation fills a value of the size given")]
pub(crate) fn is_root() -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    let (mut token, mut elevation): (HANDLE, TOKEN_ELEVATION) = (std::ptr::null_mut(), TOKEN_ELEVATION::default());
    let (size, mut written) = (u32::try_from(size_of::<TOKEN_ELEVATION>()).unwrap_or_default(), 0);
    // SAFETY: this process's token, closed after it fills a value of the size given.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) == 0 {
            return false;
        }
        let read = GetTokenInformation(token, TokenElevation, (&raw mut elevation).cast(), size, &raw mut written);
        CloseHandle(token);
        read != 0 && elevation.TokenIsElevated != 0
    }
}

/// `text` as Windows's calls take it: UTF-16, ending with a NUL.
pub(crate) fn wide(text: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    text.as_ref().encode_wide().chain([0]).collect()
}

/// The volume of `path`: runs on the same volume share its disk.
pub(crate) fn device(path: &Path) -> Option<u64> {
    volume_root(path).map(|root| fnv1a(root.as_bytes()))
}

/// The space left to this user on the disk that holds `path`, or would hold it: that of its nearest existing
/// ancestor, which it returns too, since an unmounted disk's folder is on the system's disk.
#[cfg(feature = "server")]
#[expect(unsafe_code, reason = "GetDiskFreeSpaceExW fills the integer it is given")]
#[expect(clippy::disallowed_methods, reason = "finds the nearest existing ancestor of `path`")]
pub(crate) fn free_space(path: &Path) -> Option<(&Path, u64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let existing = path.ancestors().find(|p| p.is_dir())?;
    let wide = wide(existing);
    let mut free = 0u64;
    // SAFETY: a NUL-terminated path, and the one count asked for.
    let read = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &raw mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    (read != 0).then_some((existing, free))
}

/// The root of the volume that holds `path`, or of its nearest existing ancestor: `C:\`, or a folder a
/// volume is mounted in.
#[expect(unsafe_code, reason = "GetVolumePathNameW fills a buffer of the length given")]
#[expect(clippy::disallowed_methods, reason = "finds the nearest existing ancestor of `path`")]
fn volume_root(path: &Path) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::GetVolumePathNameW;
    /// `MAX_PATH`, with its NUL.
    const VOLUME_PATH: usize = 261;
    let existing = path.ancestors().find(|p| p.exists())?;
    let wide = wide(existing);
    let mut root = [0u16; VOLUME_PATH];
    // SAFETY: a NUL-terminated path, and a buffer of the length given.
    if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), u32::try_from(root.len()).ok()?) } == 0 {
        return None;
    }
    String::from_utf16(root.get(..root.iter().position(|&c| c == 0)?)?).ok()
}

/// Whether `\` separates the parts of a path, as `/` does.
pub(crate) const BACKSLASH_SEPARATES: bool = true;

/// Moves `from` over `to`. Windows keeps an executable locked a moment after its process exits, while the system or
/// an antivirus still holds it: retried meanwhile.
#[cfg(feature = "desktop")]
#[expect(clippy::disallowed_methods, reason = "replaces an installed file, retried while Windows holds it")]
pub(crate) fn replace(from: &Path, to: &Path) -> std::io::Result<()> {
    const RETRY: std::time::Duration = std::time::Duration::from_millis(100);
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && std::time::Instant::now() < deadline => {
                std::thread::sleep(RETRY);
            }
            done => return done,
        }
    }
}

/// macOS's alone.
pub(crate) fn needs_disk_access(_: &str) -> bool {
    false
}

/// Whether the volume's disk has a seek penalty.
#[expect(unsafe_code, reason = "queries the volume's disk through a local handle")]
pub(super) fn solid_state(path: &Path) -> Option<bool> {
    use std::{
        mem::size_of,
        ptr::{null, null_mut},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
        System::{
            IO::DeviceIoControl,
            Ioctl::{
                DEVICE_SEEK_PENALTY_DESCRIPTOR, IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery,
                STORAGE_PROPERTY_QUERY, StorageDeviceSeekPenaltyProperty,
            },
        },
    };
    let root = volume_root(path)?;
    // `C:\` is the volume `\\.\C:`; one mounted in a folder is `\\?\Volume{...}`; a share has none.
    let volume = match root.trim_end_matches('\\') {
        v if v.starts_with(r"\\?\") => v.to_owned(),
        v if v.len() == 2 && v.ends_with(':') => format!(r"\\.\{v}"),
        _ => return None,
    };
    let volume = wide(volume);
    // SAFETY: a NUL-terminated name; no access asked, which a query of properties needs none for.
    let handle = unsafe {
        CreateFileW(volume.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut())
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceSeekPenaltyProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut penalty = DEVICE_SEEK_PENALTY_DESCRIPTOR::default();
    let query_size = u32::try_from(size_of::<STORAGE_PROPERTY_QUERY>()).ok()?;
    let penalty_size = u32::try_from(size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>()).ok()?;
    let mut returned = 0;
    // SAFETY: an open handle, and buffers of the sizes given.
    let asked = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            (&raw const query).cast(),
            query_size,
            (&raw mut penalty).cast(),
            penalty_size,
            &raw mut returned,
            null_mut(),
        )
    };
    // SAFETY: opened above, closed once.
    unsafe { CloseHandle(handle) };
    (asked != 0 && returned >= penalty_size).then_some(!penalty.IncursSeekPenalty)
}

/// A UNC path (`\\server\share`). A mapped network drive is not detected.
pub(super) fn network_fs(path: &Path) -> bool {
    path.as_os_str().to_string_lossy().starts_with(r"\\")
}

/// Whether stdout interprets ANSI escape codes. A Windows console does once asked to; a legacy one
/// cannot.
#[expect(unsafe_code, reason = "sets the mode of this process's console")]
pub(crate) fn ansi() -> bool {
    use windows_sys::Win32::System::Console::{
        CONSOLE_MODE, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle, STD_OUTPUT_HANDLE,
        SetConsoleMode,
    };
    let mut mode: CONSOLE_MODE = 0;
    // SAFETY: the process's own stdout handle, and a mode it reads into.
    unsafe {
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        GetConsoleMode(out, &raw mut mode) != 0
            && (mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0
                || SetConsoleMode(out, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0)
    }
}

/// Windows's build number, which unlike `GetVersionEx`'s holds without a manifest that lists the version.
#[cfg(feature = "desktop")]
#[expect(unsafe_code, reason = "asks Windows its version")]
pub(crate) fn windows_build() -> u32 {
    use windows_sys::{Wdk::System::SystemServices::RtlGetVersion, Win32::System::SystemInformation::OSVERSIONINFOW};
    // SAFETY: plain integers and an array of them, for which zeros are valid.
    let mut info: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
    info.dwOSVersionInfoSize = u32::try_from(size_of::<OSVERSIONINFOW>()).unwrap_or_default();
    // SAFETY: a structure of the size it states. It always succeeds.
    unsafe { RtlGetVersion(&raw mut info) };
    info.dwBuildNumber
}

/// Whether this process started with no console or one of its own, from the `Run` key at login, a double
/// click or `install`, rather than from a terminal or another program, whose console it shares.
#[cfg(feature = "server")]
pub(crate) static ALONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Releases a console opened for this process alone, as at login from the `Run` key, so that its window
/// closes. A terminal's console is shared with the shell, and stays.
#[cfg(feature = "server")]
#[expect(unsafe_code, reason = "lists and releases this process's console")]
pub(crate) fn release_own_console() {
    use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    let mut ids = [0u32; 2];
    // SAFETY: a buffer of the length given.
    let sharing = unsafe { GetConsoleProcessList(ids.as_mut_ptr(), 2) };
    ALONE.store(sharing <= 1, std::sync::atomic::Ordering::Relaxed);
    if sharing == 1 {
        // SAFETY: no preconditions.
        unsafe { FreeConsole() };
    }
}

/// Unix's alone: Windows keeps no packages' folder that a copy of root's own would share.
#[cfg(any(feature = "desktop", feature = "self-update"))]
pub(crate) fn system_bins() -> Vec<PathBuf> {
    Vec::new()
}

/// Linux's alone: containers run Linux.
#[cfg(any(feature = "desktop", feature = "self-update"))]
pub(crate) fn container_markers() -> Option<[PathBuf; 2]> {
    None
}

/// Linux's alone.
pub(crate) fn mount_tables() -> Option<[PathBuf; 2]> {
    None
}

/// A file in the user's profile is the user's alone already.
#[cfg(any(feature = "server", all(feature = "desktop", feature = "self-update")))]
pub(crate) fn only_owner(_: &mut std::fs::OpenOptions) {}

/// Where `app` keeps its state for the system.
#[expect(clippy::disallowed_methods, reason = "the system's data folder is in the environment")]
pub(crate) fn system_state_dir(app: &str) -> PathBuf {
    PathBuf::from(std::env::var_os("PROGRAMDATA").unwrap_or_else(|| r"C:\ProgramData".into())).join(app).join("state")
}

/// Where `app` keeps its state for this user: machine-local, so not the roaming profile.
pub(crate) fn user_state_dir(base: &impl BaseStrategy, app: &str) -> PathBuf {
    base.state_dir().unwrap_or_else(|| base.cache_dir()).join(app)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{device, foreign_path, network_fs, solid_state};

    /// Another system's absolute folders, which have no place here.
    #[test]
    fn a_unix_root_is_foreign() {
        assert!(foreign_path("/home/me") && !foreign_path(r"C:\Users\me"));
    }

    /// A local volume says whether its disk seeks, if its disk answers, as a virtual one may not; the same below
    /// a folder not created yet. A share has no volume.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "the system drive is in the environment")]
    fn a_local_disk_says_whether_it_seeks() {
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let root = Path::new(&format!(r"{system}\")).to_path_buf();
        assert_eq!(solid_state(&root.join("not yet").join("created")), solid_state(&root));
        assert_eq!(solid_state(Path::new(r"\\server\share")), None);
        assert!(network_fs(Path::new(r"\\server\share")) && !network_fs(Path::new(&system)));
    }

    /// Runs on the same volume wait for each other on a spinning disk.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "the system drive is in the environment")]
    fn paths_on_a_volume_share_its_device() {
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let root = std::path::PathBuf::from(format!(r"{system}\"));
        assert!(device(&root).is_some());
        assert_eq!(device(&root.join("not yet").join("created")), device(&root));
    }

    /// Windows is not asked: its own folders are writable by whom they belong to.
    #[cfg(any(feature = "server", feature = "self-update"))]
    #[test]
    fn any_folder_is_writable() {
        assert!(crate::sys::writable(Path::new(r"C:\Windows")));
    }
}
