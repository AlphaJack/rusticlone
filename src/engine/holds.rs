//! What runs hold, across collections and processes: their request, the repositories they use, and the
//! resources of this computer those need.

use std::cell::RefCell;

use anyhow::Result;

use super::{Collection, Engine, Run, RunResults};
use crate::request::Request;
use crate::result::{Doing, Holder};
use crate::{
    live::Live,
    profile::RusticProfile,
    result::{Status, StepResult},
    rustic::Ctx,
    state::{Holds, Mode, Queued, RepoLocks, RunRecord},
    sys,
};

impl Engine {
    /// The repository's id, read once per collection key. Until the repository exists, the key itself.
    fn repo_id(&self, run: &Run<'_>, p: &RusticProfile) -> String {
        if let Some(id) = self.store.repo_id(p.repo_key()) {
            return id;
        }
        match self.rustic.repo_id(&run.cx, p) {
            Ok(id) => {
                _ = self.store.set_repo_id(p.repo_key(), Some(&id));
                id
            }
            Err(_) => p.repo_key().to_owned(),
        }
    }

    /// Takes what a run needs, across collections and processes, named as the runs table shows them:
    /// - each repository, by its id: shared for reads and additions, exclusive for removals (rustic has
    ///   no lock of its own for this);
    /// - this computer's link to it, one run at a time: the disk of a local repository that may be on a
    ///   spinning disk, or the upload or download link of a remote one (or of a network mount);
    /// - the CPU, for runs that compress or repack.
    pub(super) fn lock<'r>(&self, run: &'r Run<'_>, cpu: bool, uses: &[(&RusticProfile, Use)]) -> Result<Held<'r>> {
        let mut keys: Vec<(String, Mode, String)> = Vec::new();
        if cpu {
            keys.push(("cpu".into(), Mode::Exclusive, "cpu".into()));
        }
        for &(p, used) in uses {
            let mode = if matches!(used, Use::Forget | Use::Prune) { Mode::Exclusive } else { Mode::Shared };
            keys.push((format!("repo:{}", self.repo_id(run, p)), mode, format!("repo:{}", p.name)));
            let links: &[&str] = match (used, p.on_disk()) {
                (Use::Forget, _) => &[],
                (_, Some(local)) if sys::spinning(local) => {
                    let disk = sys::device(local).map_or_else(|| local.display().to_string(), |d| d.to_string());
                    let mount = sys::mount_point(local).unwrap_or(local);
                    keys.push((format!("disk:{disk}"), Mode::Exclusive, format!("disk:{}", mount.display())));
                    &[]
                }
                (_, Some(_)) => &[],
                (Use::Read, None) => &["download"],
                (Use::Add, None) => &["upload"],
                (Use::Prune, None) => &["upload", "download"],
            };
            keys.extend(links.iter().map(|l| ((*l).to_owned(), Mode::Exclusive, (*l).to_owned())));
        }
        keys.sort_by(|a, b| a.0.cmp(&b.0));
        keys.dedup_by(|a, b| a.0 == b.0);
        let held: Vec<(&str, Mode)> = keys.iter().map(|(k, m, ..)| (k.as_str(), *m)).collect();
        let (last, since) = (RefCell::new(None), self.store.now());
        let (live, step) = (run.live.as_deref(), run.cx.step());
        let locks = self.store.lock(
            &run.cx,
            &held,
            |busy| {
                let Some((.., name)) = keys.iter().find(|(k, ..)| k == busy) else { return };
                // Every lock that other runs hold, not only the one it waits for first.
                let held = keys.iter().flat_map(|(.., n)| self.store.holding(n).into_iter().map(move |r| (r, n)));
                let what = Doing::WaitingFor { lock: name.clone(), by: holders(held, false) };
                if !wait_changed(&last, run.collection.title, &what) {
                    return;
                }
                run.activity(Some(what), false);
                if let Some(l) = live {
                    l.set_holds(step, Holds { using: Vec::new(), waiting_for: Some(name.clone()) });
                }
            },
            || Ok(None),
        );
        run.idle();
        if let Some(l) = live {
            let using = if locks.is_ok() { keys.into_iter().map(|(.., name)| name).collect() } else { Vec::new() };
            l.set_holds(step, Holds { using, waiting_for: None });
        }
        if last.borrow().is_some()
            && let Some(l) = &run.live
        {
            l.waited(step, u64::try_from(self.store.now().duration_since(since).as_secs()).unwrap_or_default());
        }
        Ok(Held { _locks: locks?, live, step })
    }

    /// Takes the request's own locks through `cx`, so that the same request never runs twice at once in
    /// any process, and no copy runs beside a backup, which copies too. With `queued`, the run waits for
    /// its turn; otherwise it is skipped if the same request runs. `None` if it cannot run now; `r`
    /// then says why.
    pub(super) fn lock_for<'c>(
        &self,
        collection: &Collection<'_>,
        request: &Request,
        cx: &'c Ctx,
        r: &mut RunResults,
        queued: Option<&RunRecord>,
    ) -> Option<RepoLocks<'c>> {
        let held = request_locks(collection.name(), request);
        let keys: Vec<(&str, Mode)> = held.iter().map(|k| (k.as_str(), Mode::Exclusive)).collect();
        let last = RefCell::new(None);
        let waiting = |own: &RunRecord| {
            let doing = self.queued_wait(&self.store.queue(Some(collection.name())), own, request);
            if wait_changed(&last, collection.title, &doing) {
                self.runs.wait(collection.name(), own.id, Some(doing));
            }
        };
        let lock = match queued {
            None => self.store.try_lock(cx, &keys),
            Some(run) => {
                let lock = self.store.lock(cx, &keys, |_| waiting(run), || self.store.ahead(run, request));
                self.runs.wait(collection.name(), run.id, None);
                lock.map(Some)
            }
        };
        match lock {
            Ok(None) => r.push(StepResult::RunningAlready),
            Err(e) if queued.is_some_and(|run| self.store.run(collection.name(), run.id).is_none()) => {
                r.push(StepResult::Dequeued { error: format!("{e:#}") });
            }
            Err(e) => r.push(StepResult::Error { error: format!("{e:#}") }),
            Ok(lock) => return lock,
        }
        None
    }

    /// Whether the same request runs, here or in another process. A new request for it would be skipped.
    #[cfg(feature = "server")]
    pub(crate) fn runs_already(&self, collection: &Collection<'_>, request: &Request) -> bool {
        let held = request_locks(collection.name(), request);
        let keys: Vec<(&str, Mode)> = held.iter().map(|k| (k.as_str(), Mode::Exclusive)).collect();
        matches!(self.store.try_lock(&Ctx::default(), &keys), Ok(None))
    }
}

impl Engine {
    /// Waits for `run`, which a request [joined](super::Asked::Joined), and logs its results as this one's.
    pub(crate) fn follow(&self, collection: &Collection<'_>, run: &RunRecord) -> RunResults {
        let (mut r, last) = (RunResults::new(collection.title), RefCell::new(None));
        let waiting = |_: &str| {
            // Ended: its lock is released a moment after its record.
            let Some(now) = self.store.run(collection.name(), run.id).filter(|r| r.status.unfinished()) else {
                return;
            };
            let doing = Doing::WaitingFor { by: holders([(now, &run.label)], true), lock: run.label.clone() };
            if wait_changed(&last, collection.title, &doing) {
                self.runs.wait(collection.name(), run.id, Some(doing));
            }
        };
        let waited = self.store.wait_for(&Ctx::default(), run, waiting);
        self.runs.wait(collection.name(), run.id, None);
        let ended = self.store.run(collection.name(), run.id).filter(|r| !r.status.unfinished());
        match (waited, ended) {
            (Err(e), _) => r.push(StepResult::Error { error: format!("{e:#}") }),
            (Ok(()), Some(done)) => {
                done.all_results().for_each(|result| crate::log::event(Some(collection.title), result));
                let (trigger, pid, level) = (done.trigger, done.pid, done.status.level());
                r.push(StepResult::Joined { label: done.label, trigger, pid, level });
            }
            (Ok(()), None) => r.push(StepResult::Dequeued { error: crate::state::Error::Dequeued.to_string() }),
        }
        r
    }

    /// What the queued run `own` of `request` waits for: the runs in `queue`, its collection's, that hold
    /// its locks, and those of the same request queued before it.
    pub(crate) fn queued_wait(&self, queue: &[Queued], own: &RunRecord, request: &Request) -> Doing {
        let held = request_locks(&own.collection, request);
        let ahead = queue.iter().filter(|q| {
            let Some(r) = &q.request else { return false };
            let shared = || request_locks(&own.collection, r).iter().any(|k| held.contains(k));
            let (status, id) = (q.record.status, q.record.id);
            let ahead = (status == Status::Running && shared())
                || (status == Status::Queued && id < own.id && r.key() == request.key());
            ahead && self.store.alive(&q.record)
        });
        let lock = request.label(&own.collection);
        Doing::WaitingFor { by: holders(ahead.map(|q| (q.record.clone(), &lock)), true), lock }
    }
}

/// The runs holding each lock, by process and trigger, and with `status`, by their status too.
fn holders<'a>(held: impl IntoIterator<Item = (RunRecord, &'a String)>, status: bool) -> Vec<Holder> {
    let mut by: Vec<Holder> = Vec::new();
    for (run, lock) in held {
        let status = status.then_some(run.status);
        match by.iter_mut().find(|h| (h.trigger, h.pid, h.status) == (run.trigger, run.pid, status)) {
            Some(h) if h.locks.contains(lock) => {}
            Some(h) => h.locks.push(lock.clone()),
            None => by.push(Holder { trigger: run.trigger, pid: run.pid, locks: vec![lock.clone()], status }),
        }
    }
    by
}

/// Whether what a run waits for changed since `last`. Logged if so, unless a terminal shows it with the run.
fn wait_changed(last: &RefCell<Option<Doing>>, collection: &str, doing: &Doing) -> bool {
    // A holder that ended keeps its lock a moment after its record.
    let ending = last.borrow().is_some() && matches!(doing, Doing::WaitingFor { by, .. } if by.is_empty());
    if ending || last.borrow().as_ref() == Some(doing) {
        return false;
    }
    if !crate::log::is_live() {
        crate::log::event(Some(collection), doing);
    }
    *last.borrow_mut() = Some(doing.clone());
    true
}

/// What [`Engine::lock`] took, shown as held until dropped.
pub(super) struct Held<'r> {
    _locks: RepoLocks<'r>,
    live: Option<&'r Live>,
    step: Option<usize>,
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        if let Some(l) = self.live {
            l.set_holds(self.step, Holds::default());
        }
    }
}

/// The request's own lock, and for a backup also the copy's.
fn request_locks(collection: &str, request: &Request) -> Vec<String> {
    let copy = (*request == Request::Backup).then(|| Request::Copy(collection.to_owned()));
    std::iter::once(request).chain(&copy).map(|r| format!("run:{collection}:{}", r.key())).collect()
}

/// How a run uses a repository: which lock mode it needs, and which link of this computer.
#[derive(Clone, Copy)]
pub(super) enum Use {
    /// Reads data: a check, a restore, or the source of a copy.
    Read,
    /// Adds a snapshot: a backup, or the target of a copy.
    Add,
    /// Removes snapshots: exclusive, but touches only metadata.
    Forget,
    /// Repacks: exclusive, and reads and writes data.
    Prune,
}
