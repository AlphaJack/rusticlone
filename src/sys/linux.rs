//! Linux's answers, from sysfs and `statfs`.

use std::path::{Path, PathBuf};

use etcetera::BaseStrategy;

pub(super) use super::unix::read_hostname;
pub(crate) use super::unix::*;

/// `app`'s configuration directory for this user, laid out as the `directories` crate that rustic uses lays
/// it out.
pub(crate) fn user_config_dir(base: &impl BaseStrategy, app: &str) -> PathBuf {
    base.config_dir().join(app)
}

/// Whether the process `pid` ended, for test directories to go.
#[cfg(test)]
#[expect(clippy::disallowed_methods, reason = "looks for the process in /proc")]
pub(crate) fn ended(pid: &str) -> bool {
    !Path::new("/proc").join(pid).exists()
}

/// macOS's alone.
pub(crate) fn needs_disk_access(_: &str) -> bool {
    false
}

/// From sysfs, which also reports the disks under LVM and dm-crypt.
#[cfg_attr(test, mutants::skip)] // The test machine's disk decides; `solid_state_in` is tested with a sysfs of its own.
pub(super) fn solid_state(path: &Path) -> Option<bool> {
    let dev = device(path)?;
    solid_state_in(Path::new("/sys"), libc::major(dev), libc::minor(dev))
}

/// `sys` is where sysfs is mounted. A partition has no queue of its own: its disk's applies.
#[expect(clippy::disallowed_methods, reason = "reads disk properties from sysfs")]
fn solid_state_in(sys: &Path, major: u32, minor: u32) -> Option<bool> {
    let block = std::fs::canonicalize(sys.join("dev").join("block").join(format!("{major}:{minor}"))).ok()?;
    let rotational =
        block.ancestors().take(2).find_map(|d| std::fs::read_to_string(d.join("queue").join("rotational")).ok());
    rotational.map(|r| r.trim() == "0")
}

#[cfg_attr(test, mutants::skip)] // Asks the kernel, and tests have no network mount.
#[expect(unsafe_code, reason = "statfs fills a zeroed struct for a NUL-terminated path")]
pub(super) fn network_fs(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    // SAFETY: a valid C string and a struct statfs fills.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &raw mut stat) } != 0 {
        return false;
    }
    network(stat.f_type.into())
}

/// Whether `f_type`, as `statfs` gives it, is a network file system's. It is a 32-bit magic number, in a field whose
/// integer type depends on the libc and the architecture: a signed one extends it.
fn network(f_type: i128) -> bool {
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
    let magic = f_type & i128::from(u32::MAX);
    NETWORK.iter().any(|&m| i128::from(m) == magic)
}

/// The files that Docker and Podman put in their containers.
#[cfg(any(feature = "desktop", feature = "self-update"))]
#[expect(clippy::unnecessary_wraps, reason = "other systems run no containers")]
pub(crate) fn container_markers() -> Option<[PathBuf; 2]> {
    Some([PathBuf::from("/.dockerenv"), Path::new("/run").join(".containerenv")])
}

/// What is mounted, and what `/etc/fstab` mounts at boot.
#[expect(clippy::unnecessary_wraps, reason = "other systems have no such tables")]
pub(crate) fn mount_tables() -> Option<[PathBuf; 2]> {
    Some([Path::new("/proc").join("self").join("mounts"), Path::new("/etc").join("fstab")])
}

/// Where `app` keeps its state for the system.
pub(crate) fn system_state_dir(app: &str) -> PathBuf {
    Path::new("/var").join("lib").join(app)
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink};

    use super::solid_state_in;

    #[test]
    #[expect(clippy::disallowed_methods, reason = "creates a fake /sys")]
    fn a_disk_is_solid_state_if_the_kernel_says_so() {
        let sys = crate::sys::test_dir("sys");
        let disk = |name: &str, rotational: &str| {
            fs::create_dir_all(sys.join("devices").join(name).join("queue")).unwrap();
            fs::write(sys.join("devices").join(name).join("queue").join("rotational"), rotational).unwrap();
        };
        disk("sda", "1\n");
        disk("dm-1", "0\n");
        fs::create_dir_all(sys.join("devices").join("sda").join("sda1")).unwrap();
        fs::create_dir_all(sys.join("dev").join("block")).unwrap();
        symlink(sys.join("devices").join("sda").join("sda1"), sys.join("dev").join("block").join("8:1")).unwrap();
        symlink(sys.join("devices").join("dm-1"), sys.join("dev").join("block").join("253:1")).unwrap();
        // A partition, an LVM volume, and a device the kernel has no disk for (tmpfs, btrfs).
        let found = [solid_state_in(&sys, 8, 1), solid_state_in(&sys, 253, 1), solid_state_in(&sys, 0, 26)];
        assert_eq!(found, [Some(false), Some(true), None]);
    }

    /// Network file systems by their magic number, also when a signed field extends it. Not a disk's.
    #[test]
    fn network_file_systems_are_known_by_their_number() {
        let cifs = i32::from_ne_bytes(0xff53_4d42_u32.to_ne_bytes());
        assert!(super::network(0x6969) && super::network(cifs.into()));
        assert!(!super::network(0xef53), "ext4");
    }

    /// The kind of disk the kernel gives for a folder's device.
    #[test]
    fn a_folders_disk_is_the_kernels() {
        let root = std::path::Path::new("/");
        let dev = crate::sys::device(root).unwrap();
        let kernel = solid_state_in(std::path::Path::new("/sys"), libc::major(dev), libc::minor(dev));
        assert_eq!(super::solid_state(root), kernel);
    }

    /// Where Linux keeps what is the system's, and what tells a container.
    #[test]
    fn linux_answers_its_own_way() {
        use std::path::Path;
        assert_eq!(super::system_state_dir("app"), Path::new("/var").join("lib").join("app"));
        let (proc, etc) = (Path::new("/proc"), Path::new("/etc"));
        assert_eq!(super::mount_tables(), Some([proc.join("self").join("mounts"), etc.join("fstab")]));
        #[cfg(any(feature = "desktop", feature = "self-update"))]
        assert_eq!(
            super::container_markers(),
            Some([std::path::PathBuf::from("/.dockerenv"), Path::new("/run").join(".containerenv")])
        );
    }

    /// As `/usr/bin` of a package. Root writes there anyway.
    #[cfg(any(feature = "server", feature = "self-update"))]
    #[test]
    fn a_folder_only_root_writes_is_not_writable() {
        if crate::sys::is_root() {
            return;
        }
        assert!(!crate::sys::writable(&std::path::Path::new("/usr").join("bin")));
    }
}
