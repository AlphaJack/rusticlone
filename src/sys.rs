//! Facts about this computer: user, disks, mounts and paths.

use std::{path::PathBuf, sync::LazyLock};

use etcetera::BaseStrategy;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;
/// The longest host name, with its NUL.
#[cfg(any(unix, windows))]
const HOSTNAME_BUFFER: usize = 256;

/// Stable across runs, unlike `DefaultHasher`: names lock files and asset versions.
pub(crate) fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(FNV_OFFSET, |h, b| (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME))
}

/// The random bytes of a generated password, token or session id.
#[cfg(feature = "server")]
const RANDOM_BYTES: usize = 32;

/// [`RANDOM_BYTES`] random bytes, hex-encoded.
#[cfg(feature = "server")]
pub(crate) fn random_hex() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; RANDOM_BYTES];
    getrandom::fill(&mut bytes)?;
    Ok(hex(&bytes))
}

#[cfg(any(feature = "server", feature = "self-update"))]
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The SHA-256 of `data`, hex-encoded: sessions are kept by it, and an update checks its download.
#[cfg(any(feature = "server", feature = "self-update"))]
pub(crate) fn sha256(data: impl AsRef<[u8]>) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, data.as_ref()).as_ref())
}

/// `app`'s configuration directories, the user's then the system's, laid out as the `directories` crate
/// that rustic uses lays them out.
#[expect(clippy::disallowed_methods, reason = "the system's directory on Windows is in the environment")]
pub(crate) fn config_dirs(app: &str) -> Result<Vec<PathBuf>, etcetera::HomeDirError> {
    let base = etcetera::base_strategy::choose_native_strategy()?;
    let user = if cfg!(windows) {
        base.config_dir().join(app).join("config")
    } else if cfg!(target_vendor = "apple") {
        base.data_dir().join(app)
    } else {
        base.config_dir().join(app)
    };
    let system = match std::env::var_os("PROGRAMDATA") {
        Some(data) if cfg!(windows) => Some(PathBuf::from(data).join(app).join("config")),
        _ => (!cfg!(windows)).then(|| PathBuf::from("/etc").join(app)),
    };
    Ok([user].into_iter().chain(system).collect())
}

/// Where `install` places rusticlone, on `PATH`, unless it runs from [`installer_dir`] already: that folder,
/// or for root or an administrator, whose server is the system's, a system folder. A downloaded rustic goes
/// there too.
#[cfg_attr(
    windows,
    expect(clippy::disallowed_methods, reason = "the system's program folder on Windows is in the environment")
)]
pub(crate) fn bin_dir() -> Result<PathBuf, etcetera::HomeDirError> {
    if is_root() {
        #[cfg(windows)]
        return Ok(PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_else(|| r"C:\Program Files".into()))
            .join(crate::APP_NAME));
        #[cfg(not(windows))]
        return Ok(PathBuf::from("/usr/local/bin"));
    }
    installer_dir()
}

/// Where the installer script puts rusticlone, for root too: `$CARGO_HOME/bin`, else `~/.cargo/bin`.
#[expect(clippy::disallowed_methods, reason = "the installer script's folder is in the environment")]
pub(crate) fn installer_dir() -> Result<PathBuf, etcetera::HomeDirError> {
    let cargo = std::env::var_os("CARGO_HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    Ok(cargo.map_or_else(|| etcetera::home_dir().map(|h| h.join(".cargo")), Ok)?.join("bin"))
}

/// Where `app`'s configuration is written: the system's directory for root, else the user's.
pub(crate) fn own_config_dir(app: &str) -> Option<PathBuf> {
    config_dirs(app).ok()?.into_iter().nth(usize::from(is_root()))
}

/// Whether this process may create files in `dir`, or in the folder that creating `dir` would start from.
/// Unix asks the kernel, which also counts read-only mounts and ACLs. Windows's own folders are writable
/// by whom they belong to, so it assumes they are.
#[cfg(any(feature = "server", feature = "self-update"))]
#[cfg_attr(unix, expect(unsafe_code, reason = "faccessat reads the NUL-terminated path it is given"))]
pub(crate) fn writable(dir: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        for dir in dir.ancestors() {
            let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else { return false };
            // SAFETY: `path` is NUL-terminated and outlives the call.
            if unsafe { libc::faccessat(libc::AT_FDCWD, path.as_ptr(), libc::W_OK | libc::X_OK, libc::AT_EACCESS) } == 0
            {
                return true;
            }
            if std::io::Error::last_os_error().kind() != std::io::ErrorKind::NotFound {
                return false;
            }
        }
        false
    }
    #[cfg(not(unix))]
    {
        _ = dir;
        true
    }
}

/// Empty if the system cannot tell.
pub(crate) fn hostname() -> &'static str {
    static NAME: LazyLock<String> = LazyLock::new(|| read_hostname().unwrap_or_default());
    &NAME
}

#[cfg(unix)]
#[expect(unsafe_code, reason = "gethostname fills a buffer of the length given")]
fn read_hostname() -> Option<String> {
    let mut name = [0u8; HOSTNAME_BUFFER];
    // SAFETY: a buffer of the length given.
    if unsafe { libc::gethostname(name.as_mut_ptr().cast(), name.len()) } != 0 {
        return None;
    }
    Some(std::ffi::CStr::from_bytes_until_nul(&name).ok()?.to_string_lossy().into_owned())
}

/// The DNS host name, as `hostname` prints, rather than the `NetBIOS` name.
#[cfg(windows)]
#[expect(unsafe_code, reason = "GetComputerNameExW fills a buffer of the length given")]
fn read_hostname() -> Option<String> {
    use windows_sys::Win32::System::SystemInformation::{ComputerNamePhysicalDnsHostname, GetComputerNameExW};
    let mut name = [0u16; HOSTNAME_BUFFER];
    let mut len = u32::try_from(name.len()).ok()?;
    // SAFETY: a buffer of the length given, which becomes the length written.
    if unsafe { GetComputerNameExW(ComputerNamePhysicalDnsHostname, name.as_mut_ptr(), &raw mut len) } == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(name.get(..usize::try_from(len).ok()?)?))
}

/// On Windows, whether this process is elevated, as an administrator's shell or a system service is: it
/// can install a service, and reads and writes the `ProgramData` collections.
#[expect(unsafe_code, reason = "geteuid has no preconditions; GetTokenInformation fills a value of the size given")]
pub(crate) fn is_root() -> bool {
    #[cfg(unix)]
    // SAFETY: no preconditions.
    return unsafe { libc::geteuid() } == 0;
    #[cfg(windows)]
    {
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
    #[cfg(not(any(unix, windows)))]
    false
}

/// What `restore --missing` finds at a source path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Presence {
    /// Not empty: left as it is.
    Present,
    /// Absent, or an empty directory: restored.
    Missing,
    /// An empty directory where `/etc/fstab` mounts a device that is not mounted. A restore there would
    /// fill the disk below and be hidden once the device mounts.
    Unmounted,
}

#[expect(clippy::disallowed_methods, reason = "inspects `path` and the mount table")]
pub(crate) fn presence(path: &std::path::Path) -> Presence {
    let empty = match std::fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_none(),
        Err(_) => !path.exists(),
    };
    if !empty {
        return Presence::Present;
    }
    // Both files write a space as `\040`.
    let points = |file: &str| -> Vec<PathBuf> {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        let lines = text.lines().filter(|l| !l.trim_start().starts_with('#'));
        lines.filter_map(|l| l.split_whitespace().nth(1)).map(|p| p.replace("\\040", " ").into()).collect()
    };
    let mounted = points("/proc/self/mounts");
    let expected = points("/etc/fstab").into_iter().filter(|m| m != std::path::Path::new("/") && path.starts_with(m));
    match expected.max_by_key(|m| m.components().count()) {
        Some(mount) if !mounted.contains(&mount) => Presence::Unmounted,
        _ => Presence::Missing,
    }
}

/// The device of `path`, or of its nearest existing ancestor. Unknown outside Unix.
#[cfg(unix)]
#[expect(clippy::disallowed_methods, reason = "reads the device of `path`")]
pub(crate) fn device(path: &std::path::Path) -> Option<u64> {
    let meta = path.ancestors().find_map(|p| std::fs::metadata(p).ok())?;
    Some(std::os::unix::fs::MetadataExt::dev(&meta))
}

/// On Windows, the volume of `path`: runs on the same volume share its disk.
#[cfg(windows)]
pub(crate) fn device(path: &std::path::Path) -> Option<u64> {
    volume_root(path).map(|root| fnv1a(root.as_bytes()))
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn device(_: &std::path::Path) -> Option<u64> {
    None
}

/// The space left to this user on the disk that holds `path`, or would hold it: that of its nearest existing
/// ancestor, which it returns too, since an unmounted disk's folder is on the system's disk.
#[cfg(all(unix, feature = "server"))]
#[expect(unsafe_code, reason = "statvfs fills the structure it is given")]
#[expect(clippy::disallowed_methods, reason = "finds the nearest existing ancestor of `path`")]
pub(crate) fn free_space(path: &std::path::Path) -> Option<(&std::path::Path, u64)> {
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

#[cfg(all(windows, feature = "server"))]
#[expect(unsafe_code, reason = "GetDiskFreeSpaceExW fills the integer it is given")]
#[expect(clippy::disallowed_methods, reason = "finds the nearest existing ancestor of `path`")]
pub(crate) fn free_space(path: &std::path::Path) -> Option<(&std::path::Path, u64)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let existing = path.ancestors().find(|p| p.is_dir())?;
    let wide: Vec<u16> = existing.as_os_str().encode_wide().chain([0]).collect();
    let mut free = 0u64;
    // SAFETY: a NUL-terminated path, and the one count asked for.
    let read = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &raw mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    (read != 0).then_some((existing, free))
}

/// The root of the volume that holds `path`, or of its nearest existing ancestor: `C:\`, or a folder a
/// volume is mounted in.
#[cfg(windows)]
#[expect(unsafe_code, reason = "GetVolumePathNameW fills a buffer of the length given")]
#[expect(clippy::disallowed_methods, reason = "finds the nearest existing ancestor of `path`")]
fn volume_root(path: &std::path::Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetVolumePathNameW;
    /// `MAX_PATH`, with its NUL.
    const VOLUME_PATH: usize = 261;
    let existing = path.ancestors().find(|p| p.exists())?;
    let wide: Vec<u16> = existing.as_os_str().encode_wide().chain([0]).collect();
    let mut root = [0u16; VOLUME_PATH];
    // SAFETY: a NUL-terminated path, and a buffer of the length given.
    if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), u32::try_from(root.len()).ok()?) } == 0 {
        return None;
    }
    String::from_utf16(root.get(..root.iter().position(|&c| c == 0)?)?).ok()
}

/// Whether macOS keeps this process out of folders in `path` until the process has Full Disk Access:
/// users' `Library`, which holds mail, messages and browser data. Documents, Desktop, Downloads and
/// other volumes need no such access: macOS asks the user at the first read.
#[cfg(target_vendor = "apple")]
#[expect(clippy::disallowed_methods, reason = "probes the system's privacy database, which macOS protects")]
pub(crate) fn needs_disk_access(path: &str) -> bool {
    let mut parts = std::path::Path::new(path).components().skip(1).map(|c| c.as_os_str().to_str().unwrap_or_default());
    let protected =
        matches!((parts.next(), parts.next(), parts.next()), (None, ..) | (Some("Users"), _, None | Some("Library")));
    protected && std::fs::File::open("/Library/Application Support/com.apple.TCC/TCC.db").is_err()
}

#[cfg(not(target_vendor = "apple"))]
pub(crate) fn needs_disk_access(_: &str) -> bool {
    false
}

/// Whether `path` may be on a spinning disk, where concurrent runs slow each other down. True unless
/// the system reports a solid-state disk: btrfs, tmpfs and shares are unknown.
pub(crate) fn spinning(path: &std::path::Path) -> bool {
    solid_state(path) != Some(true)
}

/// From sysfs, which also reports the disks under LVM and dm-crypt.
#[cfg(target_os = "linux")]
fn solid_state(path: &std::path::Path) -> Option<bool> {
    let dev = device(path)?;
    solid_state_in(std::path::Path::new("/sys"), libc::major(dev), libc::minor(dev))
}

/// `sys` is where sysfs is mounted. A partition has no queue of its own: its disk's applies.
#[cfg(target_os = "linux")]
#[expect(clippy::disallowed_methods, reason = "reads disk properties from sysfs")]
fn solid_state_in(sys: &std::path::Path, major: u32, minor: u32) -> Option<bool> {
    let block = std::fs::canonicalize(sys.join(format!("dev/block/{major}:{minor}"))).ok()?;
    let rotational = block.ancestors().take(2).find_map(|d| std::fs::read_to_string(d.join("queue/rotational")).ok());
    rotational.map(|r| r.trim() == "0")
}

/// From `diskutil`, for the device the file system is mounted from.
#[cfg(target_vendor = "apple")]
#[expect(unsafe_code, reason = "statfs fills a zeroed struct for a NUL-terminated path")]
#[expect(clippy::disallowed_methods, reason = "runs diskutil for the disk of `path`")]
fn solid_state(path: &std::path::Path) -> Option<bool> {
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

/// Whether stdout interprets ANSI escape codes. A Windows console does once asked to; a legacy one
/// cannot.
#[cfg(windows)]
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
#[cfg(all(windows, feature = "desktop"))]
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

#[cfg(all(windows, feature = "server"))]
static ALONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Releases a console opened for this process alone, as at login from the `Run` key, so that its window
/// closes. A terminal's console is shared with the shell, and stays.
#[cfg(all(windows, feature = "server"))]
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

/// Whether this process started with no console or one of its own, from the `Run` key at login, a double
/// click or `install`, rather than from a terminal or another program, whose console it shares.
#[cfg(all(windows, tray))]
pub(crate) fn started_alone() -> bool {
    ALONE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether stdout interprets ANSI escape codes: every terminal outside Windows does.
#[cfg(not(windows))]
pub(crate) fn ansi() -> bool {
    true
}

/// Whether the volume's disk has a seek penalty.
#[cfg(windows)]
#[expect(unsafe_code, reason = "queries the volume's disk through a local handle")]
fn solid_state(path: &std::path::Path) -> Option<bool> {
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
    let volume: Vec<u16> = volume.encode_utf16().chain([0]).collect();
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

#[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
fn solid_state(_: &std::path::Path) -> Option<bool> {
    None
}

/// The mount point of the file system that holds `path`: its topmost ancestor on the same device.
pub(crate) fn mount_point(path: &std::path::Path) -> Option<&std::path::Path> {
    let dev = device(path)?;
    path.ancestors().filter(|p| device(p) == Some(dev)).last()
}

/// Whether `path` is on a network or FUSE file system (NFS, SMB, rclone, sshfs). Rustic treats it
/// as local, but its listings may lag what was just written.
#[expect(clippy::disallowed_methods, reason = "checks the file system of `path`")]
pub(crate) fn on_network_mount(path: &std::path::Path) -> bool {
    path.ancestors().find(|p| p.exists()).is_some_and(network_fs)
}

#[cfg(target_os = "linux")]
#[expect(unsafe_code, reason = "statfs fills a zeroed struct for a NUL-terminated path")]
fn network_fs(path: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    const NETWORK: [u32; 8] = [
        0x6969,      // NFS
        0x517b,      // SMB
        0xff53_4d42, // CIFS
        0xfe53_4d42, // SMB2
        0x6573_5546, // FUSE: rclone, sshfs...
        0x0102_1997, // 9P
        0x00c3_6400, // Ceph
        0x5346_414f, // AFS
    ];
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    // SAFETY: a valid C string and a struct statfs fills.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &raw mut stat) } != 0 {
        return false;
    }
    // A 32-bit magic number, in a field whose integer type depends on the libc and the architecture.
    let magic = i128::from(stat.f_type) & i128::from(u32::MAX);
    NETWORK.iter().any(|&m| i128::from(m) == magic)
}

#[cfg(target_vendor = "apple")]
#[expect(unsafe_code, reason = "statfs fills a zeroed struct for a NUL-terminated path")]
fn network_fs(path: &std::path::Path) -> bool {
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

/// A UNC path (`\\server\share`). A mapped network drive is not detected.
#[cfg(windows)]
fn network_fs(path: &std::path::Path) -> bool {
    path.as_os_str().to_string_lossy().starts_with(r"\\")
}

#[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
fn network_fs(_: &std::path::Path) -> bool {
    false
}

/// An empty directory for test `name`, below one directory per test process in the temporary
/// directory. Directories of ended processes are removed.
#[cfg(test)]
#[expect(clippy::disallowed_methods, reason = "creates test directories")]
pub(crate) fn test_dir(name: &str) -> PathBuf {
    static ENDED_GONE: std::sync::Once = std::sync::Once::new();
    let root = std::env::temp_dir().join(crate::NAME);
    ENDED_GONE.call_once(|| {
        for run in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let pid = run.file_name().to_string_lossy().into_owned();
            if pid.parse::<u32>().is_ok()
                && !std::path::Path::new("/proc").join(&pid).exists()
                && cfg!(target_os = "linux")
            {
                _ = std::fs::remove_dir_all(run.path());
            }
        }
    });
    let dir = root.join(std::process::id().to_string()).join(name);
    _ = std::fs::remove_dir_all(&dir);
    _ = std::fs::create_dir_all(&dir);
    dir
}

#[cfg(all(test, unix, feature = "server"))]
mod unix_tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    /// A folder is writable, and so is one not created yet below it, until it is read-only.
    #[test]
    fn a_read_only_folder_is_not_writable() {
        if super::is_root() {
            return;
        }
        let dir = super::test_dir("writable");
        assert!(super::writable(&dir) && super::writable(&dir.join("rustic")));
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        let read_only = !super::writable(&dir) && !super::writable(&dir.join("rustic"));
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(read_only);
    }

    /// Runs on the same volume wait for each other on a spinning disk.
    #[test]
    fn paths_on_a_volume_share_its_device() {
        let root = std::path::Path::new("/");
        assert!(super::device(root).is_some());
        assert_eq!(super::device(&root.join("not yet").join("created")), super::device(root));
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::{fs, os::unix::fs::symlink};

    use super::solid_state_in;

    #[test]
    #[expect(clippy::disallowed_methods, reason = "creates a fake /sys")]
    fn a_disk_is_solid_state_if_the_kernel_says_so() {
        let sys = super::test_dir("sys");
        let disk = |name: &str, rotational: &str| {
            fs::create_dir_all(sys.join(format!("devices/{name}/queue"))).unwrap();
            fs::write(sys.join(format!("devices/{name}/queue/rotational")), rotational).unwrap();
        };
        disk("sda", "1\n");
        disk("dm-1", "0\n");
        fs::create_dir_all(sys.join("devices/sda/sda1")).unwrap();
        fs::create_dir_all(sys.join("dev/block")).unwrap();
        symlink(sys.join("devices/sda/sda1"), sys.join("dev/block/8:1")).unwrap();
        symlink(sys.join("devices/dm-1"), sys.join("dev/block/253:1")).unwrap();
        // A partition, an LVM volume, and a device the kernel has no disk for (tmpfs, btrfs).
        let found = [solid_state_in(&sys, 8, 1), solid_state_in(&sys, 253, 1), solid_state_in(&sys, 0, 26)];
        assert_eq!(found, [Some(false), Some(true), None]);
    }
}

#[cfg(all(test, target_vendor = "apple"))]
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
}

#[cfg(all(test, windows))]
mod tests {
    use std::path::Path;

    use super::{network_fs, solid_state};

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
        assert!(super::device(&root).is_some());
        assert_eq!(super::device(&root.join("not yet").join("created")), super::device(&root));
    }
}

/// Whether `path` is an absolute path of a system other than this one: a Windows drive's elsewhere, or a Unix
/// root's on Windows. Its folder has no place here.
pub(crate) fn foreign_path(path: &str) -> bool {
    if cfg!(windows) { path.starts_with('/') } else { windows_drive(path).is_some() }
}

/// The drive letter of a Windows path such as `C:\Users`, and what follows it.
pub(crate) fn windows_drive(path: &str) -> Option<(char, &str)> {
    let mut chars = path.chars();
    let (letter, colon, separator) = (chars.next()?, chars.next()?, chars.next()?);
    let windows = letter.is_ascii_alphabetic() && colon == ':' && (separator == '\\' || separator == '/');
    windows.then(|| (letter, path.get(2..).unwrap_or_default()))
}

#[cfg(test)]
mod path_tests {
    use super::foreign_path;

    /// Another system's absolute folders, which have no place here. Relative ones are this system's.
    #[test]
    fn another_systems_folders_are_told_apart() {
        if cfg!(windows) {
            assert!(foreign_path("/home/me") && !foreign_path(r"C:\Users\me"));
        } else {
            assert!(foreign_path(r"C:\Users\me") && foreign_path("d:/data") && !foreign_path("/home/me"));
        }
        assert!(!foreign_path("C:relative") && !foreign_path(""));
    }
}

/// What scheduled runs may wait for, as this computer tells it. The engine asks through it so that tests can
/// answer instead.
#[cfg(feature = "server")]
pub(crate) trait Conditions: Send + Sync {
    /// Not plugged in. False when unknown, as on a desktop.
    fn on_battery(&self) -> bool;
    /// On a connection that costs, as [`watch_conditions`] last heard. False when unknown or before it watches.
    fn metered(&self) -> bool;
}

/// This computer, as its system tells it.
#[cfg(feature = "server")]
pub(crate) struct ThisComputer;

#[cfg(feature = "server")]
impl Conditions for ThisComputer {
    fn on_battery(&self) -> bool {
        on_battery()
    }

    fn metered(&self) -> bool {
        metered()
    }
}

/// Called when this computer goes on or off battery, or its connection's cost changes.
#[cfg(feature = "server")]
static CHANGED: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

/// The connection's cost as the system last told it.
#[cfg(all(feature = "server", any(target_vendor = "apple", target_os = "linux")))]
static METERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "server")]
fn changed(metered: Option<bool>) {
    #[cfg(any(target_vendor = "apple", target_os = "linux"))]
    if let Some(metered) = metered {
        METERED.store(metered, std::sync::atomic::Ordering::Relaxed);
    }
    #[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
    let _ = metered;
    if let Some(changed) = CHANGED.get() {
        changed();
    }
}

/// Has the system call `changed` whenever this computer goes on or off battery, or its connection's cost changes.
/// The first call starts watching. Later ones change nothing.
#[cfg(feature = "server")]
pub(crate) fn watch_conditions(on_change: impl Fn() + Send + Sync + 'static) {
    if CHANGED.set(Box::new(on_change)).is_ok() {
        watch_power();
        watch_network();
    }
}

#[cfg(all(feature = "server", target_vendor = "apple"))]
#[expect(unsafe_code, reason = "IOKit's estimate is a C function without arguments")]
fn on_battery() -> bool {
    /// What `IOKit` estimates on external power. Unknown or a number of seconds is on battery.
    const UNLIMITED: f64 = -2.0;
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        safe fn IOPSGetTimeRemainingEstimate() -> f64;
    }
    IOPSGetTimeRemainingEstimate() > UNLIMITED
}

/// The power sources' notification, which `IOKit` posts when the source or the battery's state changes.
#[cfg(all(feature = "server", target_vendor = "apple"))]
#[expect(unsafe_code, reason = "registers for a system notification")]
fn watch_power() {
    use std::ffi::{c_char, c_void};
    unsafe extern "C" {
        fn notify_register_dispatch(
            name: *const c_char,
            token: *mut i32,
            queue: *mut c_void,
            handler: &block2::Block<dyn Fn(i32)>,
        ) -> u32;
        fn dispatch_get_global_queue(identifier: isize, flags: usize) -> *mut c_void;
    }
    let handler = block2::RcBlock::new(|_: i32| changed(None));
    let mut token = 0;
    // SAFETY: a name that lives through the call, the system's queue and a handler that the call copies.
    unsafe {
        notify_register_dispatch(
            c"com.apple.system.powersources.source".as_ptr(),
            &raw mut token,
            dispatch_get_global_queue(0, 0),
            &handler,
        );
    }
}

/// The Network framework's path monitor, which tells each change of path. A path is "expensive" on an iPhone's
/// hotspot or a cellular link, and "constrained" on a network the user put in Low Data Mode, which an Android
/// hotspot needs.
#[cfg(all(feature = "server", target_vendor = "apple"))]
#[expect(unsafe_code, reason = "starts the Network framework's path monitor")]
fn watch_network() {
    use std::ffi::c_void;
    #[link(name = "Network", kind = "framework")]
    unsafe extern "C" {
        fn nw_path_monitor_create() -> *mut c_void;
        fn nw_path_monitor_set_queue(monitor: *mut c_void, queue: *mut c_void);
        fn nw_path_monitor_set_update_handler(monitor: *mut c_void, handler: &block2::Block<dyn Fn(*mut c_void)>);
        fn nw_path_monitor_start(monitor: *mut c_void);
        fn nw_path_is_expensive(path: *mut c_void) -> bool;
        fn nw_path_is_constrained(path: *mut c_void) -> bool;
        fn dispatch_get_global_queue(identifier: isize, flags: usize) -> *mut c_void;
    }
    let handler = block2::RcBlock::new(|path: *mut c_void| {
        // SAFETY: the path the monitor passes is valid during the call.
        changed(Some(unsafe { nw_path_is_expensive(path) || nw_path_is_constrained(path) }));
    });
    // SAFETY: a monitor that is never released so that it keeps running. The system's queue. The handler that the
    // monitor copies.
    unsafe {
        let monitor = nw_path_monitor_create();
        nw_path_monitor_set_queue(monitor, dispatch_get_global_queue(0, 0));
        nw_path_monitor_set_update_handler(monitor, &handler);
        nw_path_monitor_start(monitor);
    }
}

/// A battery discharging as the kernel reports power supplies.
#[cfg(all(feature = "server", target_os = "linux"))]
fn on_battery() -> bool {
    supplies_on_battery(std::path::Path::new("/sys/class/power_supply"))
}

/// Whether the power supplies in `dir`, laid out as the kernel's, have a battery discharging and no mains plugged
/// in.
#[cfg(all(feature = "server", any(test, target_os = "linux")))]
#[expect(clippy::disallowed_methods, reason = "reads the kernel's power supplies")]
fn supplies_on_battery(dir: &std::path::Path) -> bool {
    let read = |p: &std::path::Path, f: &str| std::fs::read_to_string(p.join(f)).unwrap_or_default().trim().to_owned();
    let supplies: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    let plugged = supplies.iter().any(|s| read(s, "type") == "Mains" && read(s, "online") == "1");
    !plugged && supplies.iter().any(|s| read(s, "type") == "Battery" && read(s, "status") == "Discharging")
}

/// The kernel's events for power supplies, which it sends as a supply is plugged, unplugged or changes state.
#[cfg(all(feature = "server", target_os = "linux"))]
fn watch_power() {
    use std::os::fd::AsRawFd;

    use nix::sys::socket::{
        AddressFamily, MsgFlags, NetlinkAddr, SockFlag, SockProtocol, SockType, bind, recv, socket,
    };
    /// The group of the kernel's device events.
    const KERNEL: u32 = 1;
    /// Larger than an event: the kernel caps them at a page.
    const EVENT: usize = 8192;
    std::thread::spawn(|| {
        let flags = SockFlag::SOCK_CLOEXEC;
        let Ok(events) = socket(AddressFamily::Netlink, SockType::Datagram, flags, SockProtocol::NetlinkKObjectUEvent)
        else {
            return;
        };
        if bind(events.as_raw_fd(), &NetlinkAddr::new(0, KERNEL)).is_err() {
            return;
        }
        let mut buffer = vec![0; EVENT];
        while let Ok(read) = recv(events.as_raw_fd(), &mut buffer, MsgFlags::empty()) {
            let event = buffer.get(..read).unwrap_or_default();
            if event.split(|b| *b == 0).any(|field| field == b"SUBSYSTEM=power_supply") {
                changed(None);
            }
        }
    });
}

/// `NetworkManager`'s `Metered` property and its changes, on a thread of its own. Without `NetworkManager`,
/// as on servers and with `systemd-networkd`, the connection is never metered.
#[cfg(all(feature = "server", target_os = "linux"))]
fn watch_network() {
    use futures_util::StreamExt;
    let watch = async {
        let nm = "org.freedesktop.NetworkManager";
        let bus = zbus::Connection::system().await?;
        let proxy = zbus::Proxy::new(&bus, nm, "/org/freedesktop/NetworkManager", nm).await?;
        changed(Some(metered_value(proxy.get_property("Metered").await?)));
        let mut changes = proxy.receive_property_changed::<u32>("Metered").await;
        while let Some(change) = changes.next().await {
            changed(Some(metered_value(change.get().await?)));
        }
        Ok::<_, zbus::Error>(())
    };
    std::thread::spawn(|| {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
        _ = runtime.map(|r| r.block_on(watch));
    });
}

/// Whether a `Metered` value of `NetworkManager` is metered or guessed so. Others are not metered, or unknown.
#[cfg(all(feature = "server", any(test, target_os = "linux")))]
fn metered_value(value: u32) -> bool {
    /// `NM_METERED_YES` and `NM_METERED_GUESS_YES`.
    const METERED: [u32; 2] = [1, 3];
    METERED.contains(&value)
}

#[cfg(all(feature = "server", any(target_vendor = "apple", target_os = "linux")))]
fn metered() -> bool {
    METERED.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(all(feature = "server", windows))]
#[expect(unsafe_code, reason = "GetSystemPowerStatus fills the structure it is given")]
fn on_battery() -> bool {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    // SAFETY: integers for which zeros are valid.
    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    // SAFETY: a structure to fill. `ACLineStatus` is 0 offline and 1 online. 255 is unknown.
    unsafe { GetSystemPowerStatus(&raw mut status) != 0 && status.ACLineStatus == 0 }
}

/// The power source's setting, whose callback Windows calls when it changes.
#[cfg(all(feature = "server", windows))]
#[expect(unsafe_code, reason = "registers a callback with the power manager")]
fn watch_power() {
    use std::ffi::c_void;
    use windows_sys::Win32::{
        System::{
            Power::{DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, PowerSettingRegisterNotification},
            SystemServices::GUID_ACDC_POWER_SOURCE,
        },
        UI::WindowsAndMessaging::DEVICE_NOTIFY_CALLBACK,
    };
    unsafe extern "system" fn power_changed(_: *const c_void, _: u32, _: *const c_void) -> u32 {
        changed(None);
        0
    }
    // Kept for as long as the process runs, which the registration is.
    let recipient = Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
        Callback: Some(power_changed),
        Context: std::ptr::null_mut(),
    }));
    let mut registration = std::ptr::null_mut();
    // SAFETY: a recipient that lives as long as the process, and where it writes the registration.
    unsafe {
        PowerSettingRegisterNotification(
            &GUID_ACDC_POWER_SOURCE,
            DEVICE_NOTIFY_CALLBACK,
            std::ptr::from_mut(recipient).cast(),
            &raw mut registration,
        );
    }
}

/// The connectivity hint, whose callback Windows calls when the connection or its cost changes.
#[cfg(all(feature = "server", windows))]
#[expect(unsafe_code, reason = "registers a callback with the IP helper")]
fn watch_network() {
    use std::ffi::c_void;
    use windows_sys::Win32::{
        NetworkManagement::IpHelper::NotifyNetworkConnectivityHintChange,
        Networking::WinSock::NL_NETWORK_CONNECTIVITY_HINT,
    };
    unsafe extern "system" fn hint_changed(_: *const c_void, _: NL_NETWORK_CONNECTIVITY_HINT) {
        changed(None);
    }
    let mut handle = std::ptr::null_mut();
    // SAFETY: a callback without context, called at once and on each change, and where it writes the registration,
    // kept as long as the process runs.
    unsafe { NotifyNetworkConnectivityHintChange(Some(hint_changed), std::ptr::null(), true, &raw mut handle) };
}

/// Whether Windows says the connection costs: one set as metered or a cellular link.
#[cfg(all(feature = "server", windows))]
#[expect(unsafe_code, reason = "GetNetworkConnectivityHint fills the structure it is given")]
fn metered() -> bool {
    use windows_sys::Win32::{
        NetworkManagement::IpHelper::GetNetworkConnectivityHint,
        Networking::WinSock::{
            NL_NETWORK_CONNECTIVITY_HINT, NetworkConnectivityCostHintFixed, NetworkConnectivityCostHintVariable,
        },
    };
    // SAFETY: integers and booleans for which zeros are valid.
    let mut hint: NL_NETWORK_CONNECTIVITY_HINT = unsafe { std::mem::zeroed() };
    // SAFETY: a structure to fill.
    let read = unsafe { GetNetworkConnectivityHint(&raw mut hint) } == 0;
    read && [NetworkConnectivityCostHintFixed, NetworkConnectivityCostHintVariable].contains(&hint.ConnectivityCost)
}

#[cfg(all(feature = "server", not(any(target_vendor = "apple", target_os = "linux", windows))))]
fn on_battery() -> bool {
    false
}

#[cfg(all(feature = "server", not(any(target_vendor = "apple", target_os = "linux", windows))))]
fn metered() -> bool {
    false
}

#[cfg(all(feature = "server", not(any(target_vendor = "apple", target_os = "linux", windows))))]
fn watch_power() {}

#[cfg(all(feature = "server", not(any(target_vendor = "apple", target_os = "linux", windows))))]
fn watch_network() {}

#[cfg(all(test, feature = "server"))]
mod power_tests {
    use super::{metered_value, supplies_on_battery};

    /// The network monitor answers once it starts: with the connection's cost, and a change to wake runs.
    #[cfg(any(target_vendor = "apple", windows))]
    #[test]
    fn the_system_tells_when_the_network_is_known() {
        let (told, heard) = std::sync::mpsc::channel();
        super::watch_conditions(move || _ = told.send(()));
        heard.recv_timeout(std::time::Duration::from_secs(5)).expect("the system told nothing");
    }

    /// A laptop unplugged, then plugged in. A desktop has no battery. A missing folder is not on battery.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "writes power supplies as the kernel lays them out")]
    fn the_kernels_supplies_say_whether_on_battery() {
        let dir = super::test_dir("power-supplies");
        let supply = |name: &str, files: &[(&str, &str)]| {
            let path = dir.join(name);
            std::fs::create_dir_all(&path).unwrap();
            for (file, value) in files {
                std::fs::write(path.join(file), format!("{value}\n")).unwrap();
            }
        };
        supply("BAT0", &[("type", "Battery"), ("status", "Discharging")]);
        supply("AC", &[("type", "Mains"), ("online", "0")]);
        assert!(supplies_on_battery(&dir));
        supply("AC", &[("type", "Mains"), ("online", "1")]);
        assert!(!supplies_on_battery(&dir));
        std::fs::remove_dir_all(dir.join("BAT0")).unwrap();
        supply("AC", &[("type", "Mains"), ("online", "0")]);
        assert!(!supplies_on_battery(&dir));
        assert!(!supplies_on_battery(&dir.join("none")));
    }

    #[test]
    fn networkmanager_says_whether_metered() {
        assert!(metered_value(1) && metered_value(3));
        assert!([0, 2, 4].into_iter().all(|v| !metered_value(v)));
    }
}
