//! Linux's answers, from sysfs and `statfs`.

use std::path::Path;

pub(super) use super::unix::read_hostname;
pub(crate) use super::unix::*;

/// macOS's alone.
pub(crate) fn needs_disk_access(_: &str) -> bool {
    false
}

/// From sysfs, which also reports the disks under LVM and dm-crypt.
pub(super) fn solid_state(path: &Path) -> Option<bool> {
    let dev = device(path)?;
    solid_state_in(Path::new("/sys"), libc::major(dev), libc::minor(dev))
}

/// `sys` is where sysfs is mounted. A partition has no queue of its own: its disk's applies.
#[expect(clippy::disallowed_methods, reason = "reads disk properties from sysfs")]
fn solid_state_in(sys: &Path, major: u32, minor: u32) -> Option<bool> {
    let block = std::fs::canonicalize(sys.join(format!("dev/block/{major}:{minor}"))).ok()?;
    let rotational = block.ancestors().take(2).find_map(|d| std::fs::read_to_string(d.join("queue/rotational")).ok());
    rotational.map(|r| r.trim() == "0")
}

#[expect(unsafe_code, reason = "statfs fills a zeroed struct for a NUL-terminated path")]
pub(super) fn network_fs(path: &Path) -> bool {
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

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink};

    use super::solid_state_in;

    #[test]
    #[expect(clippy::disallowed_methods, reason = "creates a fake /sys")]
    fn a_disk_is_solid_state_if_the_kernel_says_so() {
        let sys = crate::sys::test_dir("sys");
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
