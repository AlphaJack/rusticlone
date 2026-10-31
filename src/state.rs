//! Bookkeeping in a redb database, shared by every process of the user. The repositories
//! are the source of truth: if the database is lost, every scheduled step runs once and lag is
//! measured again. Run logs are text files next to it, in `runs/<collection>/`.

#![expect(clippy::disallowed_methods, reason = "opens the state database")]

mod event;
mod locks;
mod runs;
mod step;

pub use event::Event;
#[cfg(test)]
pub(crate) use event::Hold;
pub(crate) use event::{Behind, Ledger, Read};
pub(crate) use locks::{Mode, RepoLocks};
pub(crate) use runs::LogLine;
pub(crate) use runs::ResultRecord;
pub use runs::RunRecord;
pub(crate) use runs::{Holds, Queued, RunLog, StepRecord};
use step::retry_at;
pub(crate) use step::{Checks, Lag, LastStep, Outcome, RunStep};

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    sync::Arc,
};

use anyhow::Result;
use jiff::Timestamp;
use parking_lot::Mutex;
use redb::{
    ConcurrencyMode, Database, DatabaseError, ReadOnlyDatabase, ReadTransaction, ReadableDatabase, ReadableTable,
    StorageError, TableDefinition, WriteTransaction,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::log::Level;
use crate::{
    clock::{self, Clock},
    log::Logged,
    rustic::{Snapshot, Stop},
    settings,
    shape::{self, Versioned},
};

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "state-error")]
pub(crate) enum Error {
    #[fluent(path = path.display())]
    CannotOpen {
        path: PathBuf,
        source: DatabaseError,
    },
    Dequeued,
    Stopped {
        stop: Stop,
    },
    #[fluent(path = path.display())]
    Newer {
        path: PathBuf,
    },
}

/// A repository's snapshots of a collection, and its size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoInfo {
    pub snapshots: Vec<Snapshot>,
    /// `None` when only a run listed the repository.
    pub stored: Option<u64>,
    pub data: Option<u64>,
}

/// The repository's snapshots and size at its last listing, or why the listing failed. Runs list
/// repositories too, which keeps the snapshots current but not the size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inventory {
    pub listed: Timestamp,
    pub result: Result<RepoInfo, String>,
    /// Measured before a run changed the repository.
    pub size_outdated: bool,
}

/// Declares each table, and [`create_tables`] for all of them.
macro_rules! tables {
    ($($(#[$doc:meta])* $name:ident: $key:ty => $value:ty = $file:literal;)*) => {
        $($(#[$doc])* const $name: TableDefinition<'static, $key, $value> = TableDefinition::new($file);)*

        /// Creates every table, so that reads never find one missing.
        fn create_tables(w: &WriteTransaction) -> Result<()> {
            $(w.open_table($name)?;)*
            Ok(())
        }

        /// Copies every table that `from` has into `to`.
        fn copy_tables(from: &ReadTransaction, to: &WriteTransaction) -> Result<()> {
            $(
                let mut into = to.open_table($name)?;
                if let Ok(table) = from.open_table($name) {
                    for entry in table.iter()? {
                        let (key, value) = entry?;
                        into.insert(key.value(), value.value())?;
                    }
                }
            )*
            Ok(())
        }
    };
}

tables! {
    /// For each kind of data in the state, how many of its migrations a version must know to read it.
    READABLE_FROM: &str => u64 = "readable_from";
    /// The [`Ledger`], as a single entry.
    LEDGER: () => &str = "ledger";
    /// Queued, running and ended runs, by collection and id.
    RUNS: (&str, i64) => &str = "runs";
    /// Queued and running runs, so that reading the queue does not read every run.
    UNFINISHED: (&str, i64) => () = "unfinished";
    /// By collection and repository.
    INVENTORIES: (&str, &str) => &str = "inventories";
    /// Web UI sessions: the session hash, its expiry, and the hash of the token used to log in.
    SESSIONS: &str => (i64, &str) = "logins";
    /// The [`Notified`] status, by collection.
    NOTIFIED: &str => &str = "notified";
    /// The id in each repository's `config`, by [`crate::profile::RusticProfile::repo_key`].
    REPO_IDS: &str => &str = "repo_ids";
    /// The paths rustic records for sources that hooks create, by profile and `sources`, as JSON.
    HOOKED_PATHS: (&str, &str) => &str = "hooked_paths";
    /// How many writes changed each collection, and all of them (`None`), so that the server re-renders only
    /// the pages that show a change, also when another process made it.
    CHANGES: Option<&str> => u64 = "changes";
}

/// What a write changes of what the web UI shows.
#[derive(Clone, Copy)]
enum Shown<'a> {
    Nothing,
    Of(&'a str),
    All,
}

/// Each kind of data the state keeps: how many migrations this version knows, and how many a version must
/// know to read what this one writes.
fn kinds() -> [(&'static str, u64, u64); 3] {
    fn kind<T: Versioned>(name: &'static str) -> (&'static str, u64, u64) {
        (name, T::MIGRATIONS.len() as u64, shape::readable_from::<T>() as u64)
    }
    [kind::<Ledger>("ledger"), kind::<RunRecord>("runs"), kind::<LogLine>("logs")]
}

/// Creates every table. Fails on a state that a newer version wrote and this one cannot read, else records
/// what reading it now takes: data from a newer version can remain.
fn prepare(w: &WriteTransaction, path: &std::path::Path) -> Result<()> {
    create_tables(w)?;
    let mut table = w.open_table(READABLE_FROM)?;
    for (name, known, readable_from) in kinds() {
        let stored = table.get(name)?.map(|v| v.value());
        if stored.is_some_and(|s| s > known) {
            Err(Error::Newer { path: path.to_owned() })?;
        }
        if stored < Some(readable_from) {
            table.insert(name, readable_from)?;
        }
    }
    Ok(())
}

fn json<T: DeserializeOwned>(text: &str) -> Option<T> {
    serde_json::from_str(text).ok()
}

fn to_json(value: &impl Serialize) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn at(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(ms).unwrap_or_default()
}

/// The state: the ledger, the runs, the listings and the sessions, in one database.
#[derive(Clone)]
pub struct Store {
    /// The directory that holds the database.
    pub dir: PathBuf,
    db: Arc<Database>,
    clock: Arc<dyn Clock>,
    /// A dry run, on a copy of the state in memory.
    dry_run: bool,
    /// The temporary directory of a dry run without a state, removed with the last store.
    _scratch: Option<Arc<Scratch>>,
    parsed: Arc<Parsed>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").field("dir", &self.dir).field("dry_run", &self.dry_run).finish_non_exhaustive()
    }
}

/// What tells the server that another process changed the state, and what.
#[cfg(feature = "server")]
#[derive(Default)]
pub(crate) struct Changes {
    /// How many writes changed each collection, and all of them (`None`).
    pub writes: BTreeMap<Option<String>, u64>,
    /// The queued and running runs, by collection and id.
    pub unfinished: BTreeSet<(String, i64)>,
}

/// A value and the stored text it was parsed from.
type Kept<V> = Option<(Box<str>, V)>;

/// What was last read of the ledger and of each collection's runs. The server reads them on every render,
/// mostly unchanged.
#[derive(Default)]
struct Parsed {
    ledger: Mutex<Kept<Ledger>>,
    runs: Mutex<BTreeMap<String, BTreeMap<i64, Kept<RunRecord>>>>,
}

/// `kept` if it was parsed from `text`, else `text` parsed.
fn reparse<V>(kept: Kept<V>, text: &str, parse: impl FnOnce(&str) -> Option<V>) -> Kept<V> {
    match kept {
        Some((was, value)) if *was == *text => Some((was, value)),
        _ => Some((text.into(), parse(text)?)),
    }
}

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        _ = fs::remove_dir_all(&self.0);
    }
}

/// Windows serializes the reads and writes on a file handle: a thread waiting there for redb's lock on the
/// state, which another process holds, keeps this process's other threads from reading it, and that process
/// may be waiting for them. So on Windows each process uses the state from one thread at a time.
#[cfg(windows)]
fn one_at_a_time() -> parking_lot::ReentrantMutexGuard<'static, ()> {
    static STATE: parking_lot::ReentrantMutex<()> = parking_lot::const_reentrant_mutex(());
    STATE.lock()
}

impl Store {
    /// The state in the state directory.
    pub fn open() -> Result<Self> {
        Self::at(settings::state_dir()?)
    }

    /// The store of a dry run: a copy of the state in memory, read without writing to the state, which a
    /// database open for writing would. Its locks are the state's, so that it waits for other runs. Without
    /// a state yet, they are in a temporary directory, so that the dry run creates no state.
    pub(crate) fn open_dry_run() -> Result<Self> {
        #[cfg(windows)]
        let _one = one_at_a_time();
        let dir = settings::state_dir()?;
        let path = dir.join("state.redb");
        // Shared, as the server does, when another process has it open: reading it changes no data.
        let open = || -> Result<Box<dyn ReadableDatabase>, DatabaseError> {
            match ReadOnlyDatabase::open(&path) {
                Err(DatabaseError::DatabaseAlreadyOpen) => {
                    Ok(Box::new(Database::builder().set_concurrency_mode(ConcurrencyMode::MultiWriter).open(&path)?))
                }
                state => Ok(Box::new(state?)),
            }
        };
        let state = path.exists().then(open).transpose();
        let state = state.map_err(|source| Error::CannotOpen { path: path.clone(), source })?;
        let scratch = match state {
            Some(_) => None,
            None => {
                let dir = std::env::temp_dir().join(format!("{}-dry-run-{}", crate::NAME, std::process::id()));
                fs::create_dir_all(dir.join("locks"))?;
                Some(Arc::new(Scratch(dir)))
            }
        };
        let db = Database::builder().create_with_backend(redb::backends::InMemoryBackend::new())?;
        let dir = scratch.as_ref().map_or(dir, |s| s.0.clone());
        let store = Self {
            dir,
            db: Arc::new(db),
            clock: Arc::new(clock::System),
            dry_run: true,
            _scratch: scratch,
            parsed: Arc::default(),
        };
        store.write(Shown::Nothing, create_tables)?;
        if let Some(state) = state {
            let read = state.begin_read()?;
            store.write(Shown::Nothing, |w| copy_tables(&read, w).and_then(|()| prepare(w, &path)))?;
        }
        Ok(store)
    }

    #[cfg(test)]
    pub(crate) fn with_clock(self, clock: Arc<dyn Clock>) -> Self {
        Self { clock, ..self }
    }

    /// The time now, by the store's clock.
    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    /// Waits for `for_`, or until `stop` returns true.
    pub fn wait(&self, for_: std::time::Duration, stop: &dyn Fn() -> bool) {
        self.clock.wait(for_, stop);
    }

    /// The state in `dir`.
    pub fn at(dir: PathBuf) -> Result<Self> {
        #[cfg(windows)]
        let _one = one_at_a_time();
        fs::create_dir_all(dir.join("locks"))?;
        let path = dir.join("state.redb");
        let create = || Database::builder().set_concurrency_mode(ConcurrencyMode::MultiWriter).create(&path);
        let db = match create() {
            Err(e @ (DatabaseError::Storage(StorageError::Corrupted(_)) | DatabaseError::UpgradeRequired(_))) => {
                let kept = path.with_extension("redb.unreadable");
                crate::log::event(
                    None,
                    &Logged::StateAfresh { path: path.display().to_string(), error: e.to_string() },
                );
                fs::rename(&path, kept)?;
                create()
            }
            db => db,
        }
        .map_err(|source| Error::CannotOpen { path: path.clone(), source })?;
        let store = Self {
            dir,
            db: Arc::new(db),
            clock: Arc::new(clock::System),
            dry_run: false,
            _scratch: None,
            parsed: Arc::default(),
        };
        store.write(Shown::Nothing, |w| prepare(w, &path))?;
        Ok(store)
    }

    fn read<R>(&self, f: impl FnOnce(&ReadTransaction) -> Result<R>) -> Result<R> {
        #[cfg(windows)]
        let _one = one_at_a_time();
        f(&self.db.begin_read()?)
    }

    pub(crate) fn is_dry_run(&self) -> bool {
        self.dry_run
    }

    /// Other processes see all of the transaction or none of it.
    fn write<R>(&self, shown: Shown<'_>, f: impl FnOnce(&WriteTransaction) -> Result<R>) -> Result<R> {
        #[cfg(windows)]
        let _one = one_at_a_time();
        let w = self.db.begin_write()?;
        let result = f(&w)?;
        let key = match shown {
            Shown::Nothing => None,
            Shown::Of(collection) => Some(Some(collection)),
            Shown::All => Some(None),
        };
        if let Some(key) = key {
            let mut changes = w.open_table(CHANGES)?;
            let count = changes.get(key)?.map_or(0, |c| c.value());
            changes.insert(key, count + 1)?;
        }
        w.commit()?;
        Ok(result)
    }

    #[cfg(feature = "server")]
    pub(crate) fn changes(&self) -> Changes {
        let changes = self.read(|r| {
            let (writes, unfinished) = (r.open_table(CHANGES)?, r.open_table(UNFINISHED)?);
            let writes = writes.iter()?.map(|e| e.map(|(k, v)| (k.value().map(str::to_owned), v.value())));
            let unfinished = unfinished.iter()?.map(|e| e.map(|(k, _)| (k.value().0.to_owned(), k.value().1)));
            Ok(Changes { writes: writes.collect::<Result<_, _>>()?, unfinished: unfinished.collect::<Result<_, _>>()? })
        });
        changes.unwrap_or_default()
    }

    /// Parsed only if it changed since the last read. An unreadable ledger is rebuilt from the repositories.
    fn ledger(&self, table: &impl ReadableTable<(), &'static str>) -> Result<Ledger> {
        let Some(stored) = table.get(())? else { return Ok(Ledger::default()) };
        let mut kept = self.parsed.ledger.lock();
        *kept = reparse(kept.take(), stored.value(), Ledger::from_stored);
        Ok(kept.as_ref().map(|(_, ledger)| ledger.clone()).unwrap_or_default())
    }

    /// The ledger. Empty if unreadable.
    pub fn load(&self) -> Ledger {
        self.read(|r| self.ledger(&r.open_table(LEDGER)?)).unwrap_or_else(|e| {
            crate::log::event(None, &Logged::StateUnread { error: format!("{e:#}") });
            Ledger::default()
        })
    }

    /// Folds `events` into the ledger in one transaction. Writes nothing if they change nothing, because
    /// the server reloads on every write.
    pub fn apply(&self, events: &[Event]) -> Result<()> {
        self.fold(events).map(drop)
    }

    /// [`Self::apply`], returning the new ledger.
    fn fold(&self, events: &[Event]) -> Result<Ledger> {
        let fold = |mut state: Ledger| {
            for event in events {
                state.apply(event);
            }
            state
        };
        let old = self.read(|r| self.ledger(&r.open_table(LEDGER)?))?;
        if fold(old.clone()) == old {
            return Ok(old);
        }
        self.write(Shown::All, |w| {
            // Fold again: another process may have changed the ledger since the read.
            let mut table = w.open_table(LEDGER)?;
            let new = fold(self.ledger(&table)?);
            let stored = new.to_stored()?;
            table.insert((), stored.as_str())?;
            *self.parsed.ledger.lock() = Some((stored.into(), new.clone()));
            Ok(new)
        })
    }

    /// Takes the data subset (of `of`) for the next check of `repo`, in one transaction, so that the
    /// next check in any process takes a different one.
    pub fn take_subset(&self, repo: &str, of: u32) -> Result<u32> {
        let state = self.fold(&[Event::CheckTaken { repo: repo.to_owned(), of }])?;
        Ok(state.checks.get(repo).map_or(1, |c| c.taken))
    }

    /// The last known id of the repository with this key.
    pub fn repo_id(&self, key: &str) -> Option<String> {
        self.read(|r| Ok(r.open_table(REPO_IDS)?.get(key)?.map(|id| id.value().to_owned()))).ok().flatten()
    }

    /// `None`: unknown, as after the repository was created again.
    pub fn set_repo_id(&self, key: &str, id: Option<&str>) -> Result<()> {
        self.write(Shown::Nothing, |w| {
            let mut table = w.open_table(REPO_IDS)?;
            match id {
                Some(id) => table.insert(key, id).map(drop)?,
                None => table.remove(key).map(drop)?,
            }
            Ok(())
        })
    }

    /// The paths rustic recorded for `sources` of `profile`, which a hook creates.
    pub fn hooked_paths(&self, profile: &str, sources: &[String]) -> Option<Vec<String>> {
        let key = to_json(&sources).ok()?;
        self.read(|r| Ok(r.open_table(HOOKED_PATHS)?.get((profile, key.as_str()))?.and_then(|p| json(p.value()))))
            .ok()
            .flatten()
    }

    /// `None`: unknown, as when rustic recorded others.
    pub fn set_hooked_paths(&self, profile: &str, sources: &[String], paths: Option<&[String]>) -> Result<()> {
        let key = to_json(&sources)?;
        self.write(Shown::Nothing, |w| {
            let mut table = w.open_table(HOOKED_PATHS)?;
            match paths {
                Some(paths) => table.insert((profile, key.as_str()), to_json(&paths)?.as_str()).map(drop)?,
                None => table.remove((profile, key.as_str())).map(drop)?,
            }
            Ok(())
        })
    }

    /// The repository's last listing for the collection, if any.
    pub fn inventory(&self, collection: &str, repo: &str) -> Option<Inventory> {
        self.read(|r| Ok(r.open_table(INVENTORIES)?.get((collection, repo))?.and_then(|l| json(l.value()))))
            .ok()
            .flatten()
    }

    /// The snapshots of the last listing. Empty if it failed, or if the repository was never listed.
    pub fn snapshots(&self, collection: &str, repo: &str) -> Vec<Snapshot> {
        self.inventory(collection, repo).and_then(|l| l.result.ok()).map(|i| i.snapshots).unwrap_or_default()
    }

    /// Replaces the repository's listing for the collection.
    pub fn save_inventory(&self, collection: &str, repo: &str, inventory: &Inventory) -> Result<()> {
        self.write(Shown::Of(collection), |w| {
            Ok(w.open_table(INVENTORIES)?.insert((collection, repo), to_json(inventory)?.as_str()).map(drop)?)
        })
    }

    fn update_inventory(
        &self,
        collection: &str,
        repo: &str,
        f: impl FnOnce(Option<Inventory>) -> Option<Inventory>,
    ) -> Result<()> {
        self.write(Shown::Of(collection), |w| {
            let mut table = w.open_table(INVENTORIES)?;
            let old = table.get((collection, repo))?.and_then(|l| json(l.value()));
            if let Some(inventory) = f(old) {
                table.insert((collection, repo), to_json(&inventory)?.as_str())?;
            }
            Ok(())
        })
    }

    /// Stores a run's listing. The stored size is marked outdated if the snapshots changed, unless the
    /// change kept the data (`forget`).
    pub fn relisted(&self, collection: &str, repo: &str, snapshots: &[Snapshot], same_data: bool) {
        let result = self.update_inventory(collection, repo, |old| {
            let measured = old.as_ref().and_then(|l| l.result.as_ref().ok());
            let ids = |snapshots: &[Snapshot]| snapshots.iter().map(|s| s.id.clone()).collect::<BTreeSet<_>>();
            let changed = measured.is_none_or(|i| ids(&i.snapshots) != ids(snapshots));
            Some(Inventory {
                listed: self.now(),
                result: Ok(RepoInfo {
                    snapshots: snapshots.to_vec(),
                    stored: measured.and_then(|i| i.stored),
                    data: measured.and_then(|i| i.data),
                }),
                size_outdated: (changed && !same_data) || old.is_some_and(|l| l.size_outdated),
            })
        });
        if let Err(e) = result {
            crate::log::event(
                Some(collection),
                &Logged::ListingNotKept { repo: repo.to_owned(), error: format!("{e:#}") },
            );
        }
    }

    /// Records the repository's size, measured now.
    pub fn measured(&self, collection: &str, repo: &str, stored: u64, data: u64) {
        _ = self.update_inventory(collection, repo, |inventory| {
            let mut inventory = inventory?;
            let info = inventory.result.as_mut().ok()?;
            (info.stored, info.data, inventory.size_outdated) = (Some(stored), Some(data), false);
            Some(inventory)
        });
    }

    /// Adds a session valid until `expires`, and removes expired ones.
    pub fn add_session(&self, hash: &str, expires: Timestamp, token: &str) -> Result<()> {
        let now = self.now().as_millisecond();
        self.write(Shown::Nothing, |w| {
            let mut table = w.open_table(SESSIONS)?;
            table.retain(|_, (expires, _)| expires > now)?;
            table.insert(hash, (expires.as_millisecond(), token))?;
            Ok(())
        })
    }

    /// The token hash of an unexpired session.
    pub fn session(&self, hash: &str) -> Option<String> {
        let session =
            self.read(|r| Ok(r.open_table(SESSIONS)?.get(hash)?.map(|e| (e.value().0, e.value().1.to_owned()))));
        session.ok().flatten().filter(|&(ms, _)| at(ms) > self.now()).map(|(_, token)| token)
    }

    /// Removes the session.
    pub fn remove_session(&self, hash: &str) -> Result<()> {
        self.write(Shown::Nothing, |w| Ok(w.open_table(SESSIONS)?.remove(hash).map(drop)?))
    }

    pub(crate) fn notified(&self, collection: &str) -> Notified {
        self.read(|r| notified(&r.open_table(NOTIFIED)?, collection)).unwrap_or_default()
    }

    /// Replaces the collection's [`Notified`] with what `f` makes of it, in one transaction, so that other
    /// processes see each attempt. Returns the new one if it changed. Writes nothing otherwise, like
    /// [`Self::apply`].
    pub(crate) fn update_notified(
        &self,
        collection: &str,
        f: impl Fn(&Notified) -> Notified,
    ) -> Result<Option<Notified>> {
        let changed = |old: Notified| Some(f(&old)).filter(|new| *new != old);
        if changed(self.notified(collection)).is_none() {
            return Ok(None);
        }
        self.write(Shown::Of(collection), |w| {
            let mut table = w.open_table(NOTIFIED)?;
            // Again: another process may have changed it since the read.
            let new = changed(notified(&table, collection)?);
            if let Some(new) = &new {
                table.insert(collection, to_json(new)?.as_str())?;
            }
            Ok(new)
        })
    }
}

/// The status last notified for a collection, and the failed sends of a newer one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Notified {
    /// Delivered, or below the minimum level to notify.
    pub level: Level,
    /// When `level` was delivered.
    pub since: Option<Timestamp>,
    /// Failed sends in a row.
    pub failures: u32,
    pub last_attempt: Option<Timestamp>,
    /// Why the last send failed.
    pub error: Option<String>,
}

impl Notified {
    /// After a failed send, when to send again.
    pub(crate) fn retry_at(&self) -> Option<Timestamp> {
        retry_at(self.last_attempt.filter(|_| self.failures > 0)?, self.failures)
    }
}

fn notified(table: &impl ReadableTable<&'static str, &'static str>, collection: &str) -> Result<Notified> {
    Ok(table.get(collection)?.and_then(|n| json(n.value())).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn store(name: &str) -> Store {
        Store::at(crate::sys::test_dir(name)).unwrap()
    }

    #[test]
    fn checks_take_the_data_subsets_in_turn() {
        let a = store("subsets-test");
        let b = Store::at(a.dir.clone()).unwrap();
        // Alternate the handles, as two processes would.
        let taken: Vec<u32> = (0..4).map(|i| [&a, &b][i % 2].take_subset("Docs", 3).unwrap()).collect();
        assert_eq!(taken, [1, 2, 3, 1]);
        assert_eq!(a.take_subset("Docs", 1).unwrap(), 1, "a single subset is always the first");
        fs::remove_dir_all(&a.dir).unwrap();
    }

    #[test]
    fn a_state_written_by_a_newer_version_is_not_opened() {
        let a = store("newer-test");
        let known = Ledger::MIGRATIONS.len() as u64;
        a.write(Shown::Nothing, |w| Ok(w.open_table(READABLE_FROM)?.insert("ledger", known + 1).map(drop)?)).unwrap();
        let err = Store::at(a.dir.clone()).err().unwrap();
        assert!(matches!(err.downcast_ref(), Some(Error::Newer { .. })), "{err:#}");
        fs::remove_dir_all(&a.dir).unwrap();
    }

    #[cfg(feature = "server")]
    #[test]
    fn writes_count_the_changes_pages_show() {
        let a = store("changes-test");
        let b = Store::at(a.dir.clone()).unwrap();
        a.add_session("hash", a.now() + jiff::SignedDuration::from_hours(1), "token").unwrap();
        assert!(b.changes().writes.is_empty(), "sessions are not shown");
        a.apply(&[Event::Newest { repo: "Docs".into(), at: a.now() }]).unwrap();
        a.update_notified("Docs", |_| Notified { level: Level::Failure, ..Notified::default() }).unwrap();
        let writes = b.changes().writes;
        assert_eq!((writes[&None], writes[&Some("Docs".into())]), (1, 1));
        fs::remove_dir_all(&a.dir).unwrap();
    }

    #[test]
    fn two_handles_share_the_state() {
        let a = store("shared-test");
        let b = Store::at(a.dir.clone()).unwrap();
        let at = Timestamp::from_second(1_800_000_000).unwrap();
        a.apply(&[Event::Newest { repo: "Docs".into(), at }]).unwrap();
        assert_eq!(b.load().latest["Docs"].as_second(), 1_800_000_000);
        {
            let failure = |_: &Notified| Notified { level: Level::Failure, ..Notified::default() };
            assert!(b.update_notified("Docs", failure).unwrap().is_some());
            assert!(a.update_notified("Docs", failure).unwrap().is_none(), "already notified");
            assert_eq!(a.notified("Docs").level, Level::Failure);
        }
        fs::remove_dir_all(&a.dir).unwrap();
    }
}
