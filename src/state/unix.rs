//! Unix's way with the state's files.

use std::fs::File;

/// The locks' descriptors, which rustic inherits so that they stay held while it runs.
pub(super) fn fds(files: &[File]) -> Vec<i32> {
    files.iter().map(std::os::fd::AsRawFd::as_raw_fd).collect()
}

/// Unix serializes nothing on a file handle: threads use the state at once.
pub(super) struct Concurrent;

pub(super) fn one_at_a_time() -> Concurrent {
    Concurrent
}

/// Unix lets another reader see a held lock file, so its name shows.
#[cfg(test)]
pub(super) const HELD_LOCK_READABLE: bool = true;
