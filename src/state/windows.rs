//! Windows's way with the state's files.

use std::fs::File;

/// Windows releases a lock with the process that took it, so rustic cannot inherit one: none to pass.
pub(super) fn fds(_: &[File]) -> Vec<i32> {
    Vec::new()
}

/// Windows serializes the reads and writes on a file handle: a thread waiting there for redb's lock on the
/// state, which another process holds, keeps this process's other threads from reading it, and that process
/// may be waiting for them. So on Windows each process uses the state from one thread at a time.
pub(super) fn one_at_a_time() -> parking_lot::ReentrantMutexGuard<'static, ()> {
    static STATE: parking_lot::ReentrantMutex<()> = parking_lot::ReentrantMutex::new(());
    STATE.lock()
}

/// Windows lets no one read a held lock file, so its name stays hidden while it is taken.
#[cfg(test)]
pub(super) const HELD_LOCK_READABLE: bool = false;
