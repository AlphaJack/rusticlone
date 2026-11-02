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

/// Whether `path` may be on a spinning disk, where concurrent runs slow each other down. True unless
/// the system reports a solid-state disk: btrfs, tmpfs and shares are unknown.
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
pub(crate) fn on_network_mount(path: &std::path::Path) -> bool {
    path.ancestors().find(|p| p.exists()).is_some_and(network_fs)
}
