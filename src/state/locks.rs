//! File locks shared across processes: for runs, requests, repositories and the resources they use.

use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions, TryLockError},
    io::Write,
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
    /// Held along with other shared locks.
    Shared,
    /// Held alone.
    Exclusive,
}

/// Released on drop. The run's rustic processes inherit them (see [`crate::rustic::Ctx::held`]), so that a killed
/// process cannot leave rustic running without its locks.
#[derive(Debug)]
pub struct RepoLocks<'a> {
    files: Vec<File>,
    cx: &'a Ctx,
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
        cx.hold(&super::os::fds(&files));
        Self { files, cx }
    }
}

impl Drop for RepoLocks<'_> {
    fn drop(&mut self) {
        self.cx.release(&super::os::fds(&self.files));
    }
}

impl Store {
    /// Takes all `keys`, each with its mode and the name it is shown with. The first key is taken only after earlier requests for it (`behind` returns the
    /// lock of the run ahead, or fails to end the wait). While one key is busy, no other key is held, so
    /// that two runs never wait for each other. `waiting` receives the busy key. The wait ends when the
    /// busy lock is freed or the state changes, and fails when the run or the process stops.
    pub fn lock<'a>(
        &self,
        cx: &'a Ctx,
        keys: &[(&str, Mode, &str)],
        waiting: impl Fn(&str),
        behind: impl Fn() -> Result<Option<String>>,
    ) -> Result<RepoLocks<'a>> {
        let (mut held, mut freed, mut watcher) = (Vec::new(), BTreeMap::new(), None);
        loop {
            let seen = clock::WAKES.seen();
            let (shown, busy) = match (keys.first(), behind()?) {
                (Some((first, ..)), Some(ahead)) => (*first, (ahead, Mode::Exclusive)),
                _ => match self.lock_rest(keys, &mut held)? {
                    Some((busy, mode)) => (busy, (busy.to_owned(), mode)),
                    None => break,
                },
            };
            held.clear();
            // Watched before `waiting` reads who holds the lock: a holder that saves its record after the read
            // wakes this wait, which reads it again. Another process may also cancel the run by removing its record.
            watcher = watcher.or_else(|| self.watch_state());
            waiting(shown);
            if let Some(stop) = cx.stop() {
                return Err(Error::Stopped { stop }.into());
            }
            if freed.get(&busy.0).is_none_or(JoinHandle::is_finished) {
                let file = self.lock_file(&busy.0)?;
                freed.insert(busy.0, thread::spawn(move || lock_blocking(&file, busy.1).map(|()| clock::WAKES.wake())));
            }
            clock::WAKES.wait(seen);
        }
        Ok(RepoLocks::new(cx, held))
    }

    /// Wakes this process's waits when another process changes the state.
    fn watch_state(&self) -> Option<RecommendedWatcher> {
        let mut watcher = notify::recommended_watcher(|event: notify::Result<notify::Event>| {
            if event.is_ok_and(|e| !matches!(e.kind, EventKind::Access(_))) {
                clock::WAKES.wake();
            }
        })
        .ok()?;
        watcher.watch(&self.dir.join("state.redb"), RecursiveMode::NonRecursive).ok()?;
        Some(watcher)
    }

    /// `None` if a key is busy.
    pub fn try_lock<'a>(&self, cx: &'a Ctx, keys: &[(&str, Mode, &str)]) -> Result<Option<RepoLocks<'a>>> {
        let mut held = Vec::new();
        Ok(self.lock_rest(keys, &mut held)?.is_none().then(|| RepoLocks::new(cx, held)))
    }

    /// Takes the keys not held yet, without waiting. Returns the first busy one.
    fn lock_rest<'k>(
        &self,
        keys: &[(&'k str, Mode, &str)],
        held: &mut Vec<(&'k str, File)>,
    ) -> Result<Option<(&'k str, Mode)>> {
        for &(key, mode, name) in keys {
            if held.iter().any(|(h, _)| *h == key) {
                continue;
            }
            let file = self.lock_file(key)?;
            match if mode == Mode::Shared { file.try_lock_shared() } else { file.try_lock() } {
                Ok(()) => {
                    named(&file, name);
                    held.push((key, file));
                }
                Err(TryLockError::WouldBlock) => return Ok(Some((key, mode))),
                Err(TryLockError::Error(e)) => return Err(e.into()),
            }
        }
        Ok(None)
    }

    /// Named after `key` as far as a file name can be, and told apart by its hash.
    pub(super) fn lock_path(&self, key: &str) -> PathBuf {
        self.dir.join("locks").join(format!("{}-{:016x}.lock", readable(key), sys::fnv1a(key.as_bytes())))
    }

    #[expect(clippy::disallowed_methods, reason = "creates lock files")]
    pub(super) fn lock_file(&self, key: &str) -> std::io::Result<File> {
        OpenOptions::new().create(true).truncate(false).write(true).open(self.lock_path(key))
    }
}

/// The start of `key`, with what no file system takes in a name as `-`.
fn readable(key: &str) -> String {
    const LONGEST: usize = 40;
    let safe = |c: char| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' };
    key.chars().map(safe).take(LONGEST).collect::<String>().trim_matches('-').to_owned()
}

/// Writes `name` into a lock file just taken, so that whoever finds it held can tell what it locks. Best
/// effort: the lock itself is what counts.
pub(super) fn named(file: &File, name: &str) {
    let mut file = file;
    _ = file.set_len(0).and_then(|()| file.write_all(name.as_bytes()));
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, time::Duration};

    use super::*;
    use crate::state::tests::store;

    /// A shared wait ends at once beside other shared locks, and only once an exclusive one is released.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "creates lock files")]
    fn a_shared_wait_ends_once_no_exclusive_lock_is_held() {
        let store = store("lock-blocking-test");
        let (held, waiting) = (store.lock_file("repo").unwrap(), store.lock_file("repo").unwrap());
        held.lock_shared().unwrap();
        lock_blocking(&waiting, Mode::Shared).unwrap();
        held.unlock().unwrap();
        held.lock().unwrap();
        let (done, ended) = mpsc::channel();
        let waiter = thread::spawn(move || done.send(lock_blocking(&waiting, Mode::Shared).is_ok()).unwrap());
        assert!(ended.recv_timeout(Duration::from_millis(100)).is_err(), "ended before the release");
        held.unlock().unwrap();
        assert!(ended.recv().unwrap());
        waiter.join().unwrap();
        std::fs::remove_dir_all(&store.dir).unwrap();
    }

    /// A run waits for the one ahead of it, showing the lock it waits for, and releases its locks once dropped.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "creates lock files")]
    fn a_run_waits_for_the_one_ahead_then_holds_its_locks() {
        let store = store("lock-ahead-test");
        let (cx, ahead) = (Ctx::default(), store.lock_file("run:Docs:1").unwrap());
        ahead.lock().unwrap();
        let (told, waiting) = mpsc::channel();
        let still = std::sync::atomic::AtomicBool::new(true);
        thread::scope(|scope| {
            let locking = scope.spawn(|| {
                let behind = || Ok(still.load(std::sync::atomic::Ordering::Relaxed).then(|| "run:Docs:1".to_owned()));
                store.lock(
                    &cx,
                    &[("Docs", Mode::Exclusive, "Docs")],
                    |busy| told.send(busy.to_owned()).unwrap(),
                    behind,
                )
            });
            assert_eq!(waiting.recv_timeout(Duration::from_secs(10)).expect("waits for the run ahead"), "Docs");
            still.store(false, std::sync::atomic::Ordering::Relaxed);
            ahead.unlock().unwrap();
            let locks = locking.join().unwrap().unwrap();
            // What this system's rustic inherits of one lock.
            let one = super::super::os::fds(&[store.lock_file("Docs").unwrap()]).len();
            assert_eq!(cx.held().len(), one);
            drop(locks);
            assert!(cx.held().is_empty());
        });
        std::fs::remove_dir_all(&store.dir).unwrap();
    }

    /// A lock file is named after its key, and holds the name of what it locks while taken.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "reads a lock file")]
    fn a_lock_file_is_named_after_its_key_and_holds_its_name() {
        let store = store("lock-names-test");
        let file = |key: &str| store.lock_path(key).file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(file("disk:64772"), format!("disk-64772-{:016x}.lock", sys::fnv1a(b"disk:64772")));
        assert!(file("run:My Docs/usb:backup").starts_with("run-My-Docs-usb-backup-"));
        assert_ne!(file("run:a b"), file("run:a/b"), "told apart by the hash");
        let cx = Ctx::default();
        let held = store.try_lock(&cx, &[("disk:64772", Mode::Exclusive, "disk:/backup")]).unwrap();
        assert!(held.is_some());
        // Windows lets no one read a held lock, so only the other systems show its name.
        #[cfg(not(windows))]
        assert_eq!(std::fs::read_to_string(store.lock_path("disk:64772")).unwrap(), "disk:/backup");
        drop(held);
        std::fs::remove_dir_all(&store.dir).unwrap();
    }
}
