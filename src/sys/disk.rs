//! What holds a path: its disk, its mount and whether anything is there.

use std::path::PathBuf;

use super::os::{device, network_fs, solid_state};

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
    let Some([mounts, fstab]) = super::os::mount_tables() else { return Presence::Missing };
    let read = |file: &std::path::Path| std::fs::read_to_string(file).unwrap_or_default();
    if unmounted(path, &read(&mounts), &read(&fstab)) { Presence::Unmounted } else { Presence::Missing }
}

/// Whether `fstab` mounts a device at `path` or above it, other than `/`, that `mounts` lacks. The innermost such
/// mount point counts.
fn unmounted(path: &std::path::Path, mounts: &str, fstab: &str) -> bool {
    let mounted = points(mounts);
    let expected = points(fstab).into_iter().filter(|m| m != std::path::Path::new("/") && path.starts_with(m));
    expected.max_by_key(|m| m.components().count()).is_some_and(|mount| !mounted.contains(&mount))
}

/// The mount points a table such as `/proc/mounts` or `/etc/fstab` lists, which write a space as `\040`.
fn points(table: &str) -> Vec<PathBuf> {
    let lines = table.lines().filter(|l| !l.trim_start().starts_with('#'));
    lines.filter_map(|l| l.split_whitespace().nth(1)).map(|p| p.replace("\\040", " ").into()).collect()
}

/// Whether `path` may be on a spinning disk, where concurrent runs slow each other down. True unless
/// the system reports a solid-state disk: btrfs, tmpfs and shares are unknown.
#[cfg_attr(test, mutants::skip)] // Asks the disk, which a test has no say in.
pub(crate) fn spinning(path: &std::path::Path) -> bool {
    solid_state(path) != Some(true)
}

/// The mount point of the file system that holds `path`: its topmost ancestor on the same device.
pub(crate) fn mount_point(path: &std::path::Path) -> Option<&std::path::Path> {
    let dev = device(path)?;
    path.ancestors().filter(|p| device(p) == Some(dev)).last()
}

/// Whether `path` is on a network or FUSE file system (NFS, SMB, rclone, sshfs). Rustic treats it
/// as local, but its listings may lag what was just written.
#[expect(clippy::disallowed_methods, reason = "checks the file system of `path`")]
#[cfg_attr(test, mutants::skip)] // Asks the file system, and no test runs on a share.
pub(crate) fn on_network_mount(path: &std::path::Path) -> bool {
    path.ancestors().find(|p| p.exists()).is_some_and(network_fs)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// A table's mount points, without its comments, with `\040` as a space.
    #[test]
    fn a_mount_table_lists_its_points() {
        let table = "# comment\n/dev/sda1 / ext4 rw 0 0\n  # indented\nserver:/share /mnt/My\\040Disk nfs rw 0 0\n";
        assert_eq!(points(table), [PathBuf::from("/"), PathBuf::from("/mnt/My Disk")]);
    }

    /// A path is unmounted where `fstab` mounts a device above it that is not mounted. The innermost mount point
    /// counts, and `/` is always there.
    #[test]
    fn a_path_below_a_mount_that_is_not_there_is_unmounted() {
        let fstab = "/dev/a / ext4 rw 0 0\n/dev/b /data ext4 rw 0 0\n/dev/c /data/usb ext4 rw 0 0\n";
        let path = Path::new("/data/usb/docs");
        assert!(unmounted(path, "/dev/a / ext4 rw 0 0\n/dev/b /data ext4 rw 0 0\n", fstab), "/data/usb is missing");
        assert!(!unmounted(path, "/dev/c /data/usb ext4 rw 0 0\n", fstab), "the innermost is there");
        assert!(!unmounted(Path::new("/home/me"), "", fstab), "only / is above it");
        assert!(!unmounted(Path::new("/database"), "", fstab), "a name that starts like a mount point is not below it");
    }

    /// A folder's mount point is the topmost of its ancestors on its device.
    #[test]
    fn a_folder_s_mount_point_is_on_its_device() {
        let dir = crate::sys::test_dir("mount-point");
        let mount = mount_point(&dir).unwrap();
        assert!(dir.starts_with(mount) && device(mount) == device(&dir));
        assert!(mount.parent().is_none_or(|above| device(above) != device(&dir)), "the topmost on it");
    }
}
