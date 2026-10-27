//! File locks shared across processes: for runs, requests, repositories and the resources they use.

#![expect(clippy::disallowed_methods, reason = "creates lock files")]

use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions, TryLockError},
    path::PathBuf,
    thread::{self, JoinHandle},
};

use anyhow::Result;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use super::{Error, Store};
use crate::{clock, rustic::Ctx, sys};

/// Held while a run is queued or running, so that other processes know its process is alive.
pub(super) fn run_lock(collection: &str, id: i64) -> String {
    format!("run:{collection}:{id}")
}

/// Shared with runs that read from or add to the repository, or exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Shared,
    Exclusive,
}

/// Released on drop. The run's rustic processes inherit them (see [`crate::rustic::Ctx::held`]), so that a killed
/// process cannot leave rustic running without its locks.
pub struct RepoLocks<'a> {
    files: Vec<File>,
    cx: &'a Ctx,
}

#[cfg(unix)]
fn fds(files: &[File]) -> Vec<i32> {
    files.iter().map(std::os::fd::AsRawFd::as_raw_fd).collect()
}

#[cfg(not(unix))]
fn fds(_: &[File]) -> Vec<i32> {
    Vec::new()
}

/// Waits for the lock, then releases it. Runs in its own thread, because the wait cannot be
/// interrupted.
fn lock_blocking(file: &File, mode: Mode) -> std::io::Result<()> {
    if mode == Mode::Shared { file.lock_shared() } else { file.lock() }?;
    file.unlock()
}

impl<'a> RepoLocks<'a> {
    fn new(cx: &'a Ctx, held: Vec<(&str, File)>) -> Self {
        let files: Vec<File> = held.into_iter().map(|(_, file)| file).collect();
        cx.hold(&fds(&files));
        Self { files, cx }
    }
}

impl Drop for RepoLocks<'_> {
    fn drop(&mut self) {
        self.cx.release(&fds(&self.files));
    }
}

impl Store {
    /// Takes all `keys`. The first key is taken only after earlier requests for it (`behind` returns the
    /// lock of the run ahead, or fails to end the wait). While one key is busy, no other key is held, so
    /// that two runs never wait for each other. `waiting` receives the busy key. The wait ends when the
    /// busy lock is freed or the state changes, and fails when the run or the process stops.
    pub fn lock<'a>(
        &self,
        cx: &'a Ctx,
        keys: &[(&str, Mode)],
        waiting: impl Fn(&str),
        behind: impl Fn() -> Result<Option<String>>,
    ) -> Result<RepoLocks<'a>> {
        let (mut held, mut freed, mut watcher) = (Vec::new(), BTreeMap::new(), None);
        loop {
            let seen = clock::wakes();
            let (shown, busy) = match (keys.first(), behind()?) {
                (Some((first, _)), Some(ahead)) => (*first, (ahead, Mode::Exclusive)),
                _ => match self.lock_rest(keys, &mut held)? {
                    Some((busy, mode)) => (busy, (busy.to_owned(), mode)),
                    None => break,
                },
            };
            held.clear();
            waiting(shown);
            if let Some(stop) = cx.stop() {
                return Err(Error::Stopped { stop }.into());
            }
            // Another process may cancel the run by removing its record.
            watcher = watcher.or_else(|| self.watch_state());
            if freed.get(&busy.0).is_none_or(JoinHandle::is_finished) {
                let file = self.lock_file(&busy.0)?;
                freed.insert(busy.0, thread::spawn(move || lock_blocking(&file, busy.1).map(|()| clock::wake())));
            }
            clock::wait(seen);
        }
        Ok(RepoLocks::new(cx, held))
    }

    /// Wakes this process's waits when another process changes the state.
    fn watch_state(&self) -> Option<RecommendedWatcher> {
        let mut watcher = notify::recommended_watcher(|event: notify::Result<notify::Event>| {
            if event.is_ok_and(|e| !matches!(e.kind, EventKind::Access(_))) {
                clock::wake();
            }
        })
        .ok()?;
        watcher.watch(&self.dir.join("state.redb"), RecursiveMode::NonRecursive).ok()?;
        Some(watcher)
    }

    /// `None` if a key is busy.
    pub fn try_lock<'a>(&self, cx: &'a Ctx, keys: &[(&str, Mode)]) -> Result<Option<RepoLocks<'a>>> {
        let mut held = Vec::new();
        Ok(self.lock_rest(keys, &mut held)?.is_none().then(|| RepoLocks::new(cx, held)))
    }

    /// Takes the keys not held yet, without waiting. Returns the first busy one.
    fn lock_rest<'k>(
        &self,
        keys: &[(&'k str, Mode)],
        held: &mut Vec<(&'k str, File)>,
    ) -> Result<Option<(&'k str, Mode)>> {
        for &(key, mode) in keys {
            if held.iter().any(|(h, _)| *h == key) {
                continue;
            }
            let file = self.lock_file(key)?;
            match if mode == Mode::Shared { file.try_lock_shared() } else { file.try_lock() } {
                Ok(()) => held.push((key, file)),
                Err(TryLockError::WouldBlock) => return Ok(Some((key, mode))),
                Err(TryLockError::Error(e)) => return Err(e.into()),
            }
        }
        Ok(None)
    }

    pub(super) fn lock_path(&self, key: &str) -> PathBuf {
        self.dir.join("locks").join(format!("{:016x}.lock", sys::fnv1a(key.as_bytes())))
    }

    pub(super) fn lock_file(&self, key: &str) -> std::io::Result<File> {
        OpenOptions::new().create(true).truncate(false).write(true).open(self.lock_path(key))
    }
}
