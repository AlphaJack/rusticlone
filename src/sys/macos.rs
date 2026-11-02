//! macOS's answers, from `statfs`, `diskutil` and its privacy rules.

use std::path::Path;

use etcetera::BaseStrategy;

/// Whether a variable moved one of the XDG folders: only Linux reads them.
#[cfg(test)]
pub(crate) fn xdg_moved() -> bool {
    false
}

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

/// Asks `IOKit` whether the disk under `path` is an SSD. The answer sits on the storage device above the
/// partition or APFS volume, where `diskutil` reads it too.
#[expect(unsafe_code, reason = "statfs, IOKit and CoreFoundation are C functions")]
#[expect(clippy::disallowed_methods, reason = "finds the folder of `path` that exists")]
pub(super) fn solid_state(path: &Path) -> Option<bool> {
    use std::{
        ffi::{c_char, c_void},
        os::unix::ffi::OsStrExt,
    };
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOBSDNameMatching(main: u32, options: u32, name: *const c_char) -> *mut c_void;
        fn IOServiceGetMatchingService(main: u32, matching: *mut c_void) -> u32;
        fn IORegistryEntrySearchCFProperty(
            entry: u32,
            plane: *const c_char,
            key: *const c_void,
            allocator: *const c_void,
            options: u32,
        ) -> *const c_void;
        fn IOObjectRelease(object: u32) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(allocator: *const c_void, text: *const c_char, encoding: u32) -> *const c_void;
        fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
        fn CFEqual(a: *const c_void, b: *const c_void) -> bool;
        fn CFRelease(object: *const c_void);
    }
    /// `kCFStringEncodingUTF8`.
    const UTF8: u32 = 0x0800_0100;
    /// `kIORegistryIterateRecursively | kIORegistryIterateParents`: up from the volume to its device.
    const UP: u32 = 1 | 2;
    let existing = path.ancestors().find(|p| p.exists())?;
    let path = std::ffi::CString::new(existing.as_os_str().as_bytes()).ok()?;
    // SAFETY: a valid C string and a struct statfs fills.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &raw mut stat) } != 0 {
        return None;
    }
    // SAFETY: statfs NUL-terminates the device name, such as `/dev/disk3s1`.
    let from = unsafe { std::ffi::CStr::from_ptr(stat.f_mntfromname.as_ptr()) };
    let name = std::ffi::CString::new(from.to_bytes().strip_prefix(b"/dev/")?).ok()?;
    // SAFETY: every string, service and property is released once, the lookup consumes the matching
    // dictionary, and the dictionary is read while held.
    unsafe {
        let string = |text: &std::ffi::CStr| CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), UTF8);
        let (characteristics, medium, solid) =
            (string(c"Device Characteristics"), string(c"Medium Type"), string(c"Solid State"));
        let service = IOServiceGetMatchingService(0, IOBSDNameMatching(0, 0, name.as_ptr()));
        let found = (service != 0)
            .then(|| {
                IORegistryEntrySearchCFProperty(service, c"IOService".as_ptr(), characteristics, std::ptr::null(), UP)
            })
            .filter(|dict| !dict.is_null());
        let answer = found.map(|dict| {
            let value = CFDictionaryGetValue(dict, medium);
            let answer = (!value.is_null()).then(|| CFEqual(value, solid));
            CFRelease(dict);
            answer
        });
        if service != 0 {
            IOObjectRelease(service);
        }
        for s in [characteristics, medium, solid] {
            CFRelease(s);
        }
        answer.flatten()
    }
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

/// Linux's alone: containers run Linux.
#[cfg(any(feature = "desktop", feature = "self-update"))]
pub(crate) fn container_markers() -> Option<[std::path::PathBuf; 2]> {
    None
}

/// Linux's alone: macOS has no `/proc` to tell what is mounted.
pub(crate) fn mount_tables() -> Option<[std::path::PathBuf; 2]> {
    None
}

/// Where `app` keeps its state for the system.
pub(crate) fn system_state_dir(app: &str) -> std::path::PathBuf {
    Path::new("/Library").join("Application Support").join(app)
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

    /// `IOKit` and `diskutil` agree about the disk of `/`.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "compares with diskutil")]
    fn iokit_agrees_with_diskutil() {
        let info = std::process::Command::new("diskutil").args(["info", "-plist", "/"]).output().unwrap();
        let plist = String::from_utf8(info.stdout).unwrap();
        let said = plist.split_once("<key>SolidState</key>").map(|(_, v)| v.trim_start().starts_with("<true/>"));
        assert_eq!(solid_state(Path::new("/")), said, "{plist}");
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
