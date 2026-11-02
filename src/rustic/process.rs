//! Rustic's process, and the processes it starts.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

#[cfg(feature = "server")]
pub(crate) use imp::tie_child;
pub(super) use imp::work;
pub(crate) use imp::{inherit_locks, kill, own_group, quiet, tie_spawned};

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// Written to disk: macOS counts the disk's bytes only.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "writes a file to produce I/O")]
    fn work_is_counted() {
        let before = work(std::process::id()).unwrap();
        let path = std::env::temp_dir().join(format!("{}-work-{}", crate::NAME, std::process::id()));
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(&vec![1; 1 << 20]).unwrap();
        file.sync_all().unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(work(std::process::id()).unwrap() > before);
    }
}

/// Ends `child`, and what it starts, with this process, as Windows needs: see `tie`.
#[cfg(feature = "server")]
pub(crate) fn tie_child(child: &tokio::process::Child) {
    #[cfg(windows)]
    if let Some(handle) = child.raw_handle() {
        tie(handle);
    }
    #[cfg(not(windows))]
    let _ = child;
}
