//! Each collection's runs, from the time they are requested: their records, which also form the queue, and
//! their logs as JSON lines.

#![expect(clippy::disallowed_methods, reason = "writes run logs")]

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::{self, File, OpenOptions, TryLockError},
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

use anyhow::Result;
use jiff::Timestamp;
use redb::{ReadableTable, WriteTransaction};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};

use super::{
    Error, Kept, RUNS, Shown, Store, UNFINISHED,
    locks::{Mode, run_lock},
    reparse,
};
use crate::{
    clock, fmt,
    log::{Level, Message},
    request::{Request, RunKind, Trigger},
    result::{self, Status, StepResult},
    rustic::{Call, Ctx, Origin},
    shape::{self, Versioned},
};

const MAX_RUNS: usize = 100;
/// How much of a log's end is read back.
const LOG_TAIL: u64 = 512 << 10;
/// Beyond this size, a log keeps only its last lines, so that a verbose rustic cannot fill the disk.
const LOG_CAP: u64 = 8 << 20;
const LOG_CAP_LINES: usize = 2000;

/// A line of a run's log. A log is one JSON object per line, after a first line with the lines' `version`.
/// Lines in another form are kept whole as `text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub at: Option<Timestamp>,
    #[serde(flatten)]
    pub origin: Origin,
    pub text: String,
}

impl Versioned for LogLine {
    const MIGRATIONS: &'static [shape::Migration] = &[];
}

impl LogLine {
    fn note(text: String) -> Self {
        Self { at: None, origin: Origin::default(), text }
    }

    fn json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// `line` written in the shape of `version`.
    fn parse(line: &str, version: usize) -> Self {
        let parsed = || {
            let mut value = serde_json::from_str(line).ok()?;
            shape::migrate_from::<Self>(&mut value, version);
            serde_json::from_value(value).ok()
        };
        parsed().unwrap_or_else(|| Self::note(line.to_owned()))
    }
}

/// What a log says of its own lines.
#[derive(crate::i18n::Fluent)]
enum LogNote {
    /// Past [`LOG_CAP`].
    LeftOut { lines: u64 },
    /// Read from its end, with the size before.
    Cut { size: String },
}

/// A run of a collection and its report. Its log is a separate file. It keeps what is shown, not the
/// types that made it, so that a later version can still show it. Its request is stored beside it.
#[derive(Clone, Serialize, Deserialize)]
pub struct RunRecord {
    /// When it was requested, in milliseconds. Also its position in the queue.
    pub id: i64,
    pub collection: String,
    /// `prune primary`, `check primary 3/10`.
    pub label: String,
    pub kind: RunKind,
    /// The repository it acts on, if one.
    pub scope: Option<String>,
    pub trigger: Trigger,
    pub pid: u32,
    /// When it started. While queued: when it was requested.
    pub started: Timestamp,
    /// Unset while it runs, or if the process stopped before its end.
    pub finished: Option<Timestamp>,
    pub status: Status,
    /// Results that no step produced, such as a request that could not be planned.
    pub results: Vec<ResultRecord>,
    pub calls: Vec<Call>,
    /// Its planned steps, and how far each got.
    pub steps: Vec<StepRecord>,
}

/// The resources a step holds and waits for, by the names the tables show.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Holds {
    pub using: Vec<String>,
    pub waiting_for: Option<String>,
}

/// A result as `--json` prints it: its kind and values, its level and its text. Unlike a
/// [`StepResult`], it reads back whatever kinds results had when it was written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResultRecord {
    pub level: Level,
    /// The repository, target, path or step it concerns, shown before [`Self::text`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub text: String,
    #[serde(flatten)]
    pub values: serde_json::Map<String, serde_json::Value>,
}

impl From<&StepResult> for ResultRecord {
    fn from(result: &StepResult) -> Self {
        let values = match serde_json::to_value(result) {
            Ok(serde_json::Value::Object(values)) => values,
            _ => serde_json::Map::new(),
        };
        let (subject, text) = (result.subject().map(Into::into), result::Detail(result).to_string());
        Self { level: result.level(), subject, text, values }
    }
}

impl ResultRecord {
    /// Its text where `named` are already shown: without its subject if it is one of them.
    pub fn within(&self, named: &[impl AsRef<str>]) -> String {
        if self.subject.as_deref().is_some_and(|s| named.iter().any(|n| n.as_ref() == s)) {
            self.text.clone()
        } else {
            self.to_string()
        }
    }
}

/// `subject: text`.
impl std::fmt::Display for ResultRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.subject {
            Some(subject) => write!(f, "{subject}: {}", self.text),
            None => f.write_str(&self.text),
        }
    }
}

impl Message for ResultRecord {
    fn level(&self) -> Level {
        self.level
    }
}

impl Versioned for RunRecord {
    const MIGRATIONS: &'static [shape::Migration] = &[
        shape::Migration { up: |v| _ = v.get_mut("request").map(kebab_request), read_by_previous: false },
        shape::Migration { up: |v| _ = v.get_mut("request").map(listed_repositories), read_by_previous: false },
    ];
}

/// A prune's or check's repository as the one of a list.
fn listed_repositories(request: &mut serde_json::Value) {
    let one = |v: &&mut serde_json::Value| v.is_string() || v.get(0).is_some_and(serde_json::Value::is_string);
    for kind in ["prune", "check"] {
        if let Some(held) = request.get_mut(kind).filter(one) {
            *held = serde_json::Value::Array(vec![held.take()]);
        }
    }
}

/// A request's tags in kebab-case, and a check's reading or a test restore's snapshot with it.
fn kebab_request(request: &mut serde_json::Value) {
    let tag = request.as_object().and_then(|m| m.keys().next().cloned());
    let Some(held) = shape::kebab(request) else { return };
    match tag.as_deref() {
        Some("Check") => _ = held.get_mut(1).and_then(shape::kebab).and_then(shape::kebab),
        Some("TestRestore") => _ = held.get_mut(1).and_then(shape::kebab),
        Some("Recreate") => _ = shape::retag(request, |_| "re-create".into()),
        _ => {}
    }
}

/// A stored run, as a [`RunRecord`] or [`Queued`], in this version's shape.
fn stored<T: DeserializeOwned>(text: &str) -> Option<T> {
    let mut value = serde_json::from_str(text).ok()?;
    shape::migrate::<RunRecord>(&mut value);
    serde_json::from_value(value).ok()
}

/// A stored run read back with its request, to queue or wait for it.
#[derive(Deserialize)]
pub struct Queued {
    #[serde(flatten)]
    pub record: RunRecord,
    /// `None` if another version wrote it in another shape.
    #[serde(default, deserialize_with = "lenient")]
    pub request: Option<Request>,
}

/// Reads a value that another version wrote in another shape as `None`.
fn lenient<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Option<T>, D::Error> {
    Ok(serde_json::from_value(serde_json::Value::deserialize(d)?).ok())
}

/// One step of a run, and its results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRecord {
    /// `copy to usb`.
    pub label: String,
    /// The repository it works on, which its label names, so that its results leave it out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The indices of the earlier steps that must do their job for it to run.
    #[serde(default)]
    pub needs: Vec<usize>,
    pub status: Status,
    pub started: Option<Timestamp>,
    pub finished: Option<Timestamp>,
    pub results: Vec<ResultRecord>,
    /// While it runs.
    #[serde(default)]
    pub holds: Holds,
    /// Seconds spent waiting for repositories and links held by other runs. Not part of its duration.
    #[serde(default)]
    pub waited: u64,
}

impl StepRecord {
    pub fn queued(label: String, needs: Vec<usize>) -> Self {
        Self {
            label,
            repo: None,
            needs,
            status: Status::Queued,
            started: None,
            finished: None,
            results: Vec::new(),
            holds: Holds::default(),
            waited: 0,
        }
    }
}

impl StepRecord {
    /// A result of it, without the subject that its row names: its label or repository.
    pub fn result(&self, result: &ResultRecord) -> String {
        result.within(&[self.label.as_str()].into_iter().chain(self.repo.as_deref()).collect::<Vec<_>>())
    }
}

impl RunRecord {
    /// All its results: its own, then its steps'.
    pub fn all_results(&self) -> impl Iterator<Item = &ResultRecord> {
        self.results.iter().chain(self.steps.iter().flat_map(|s| &s.results))
    }

    /// Seconds its steps spent waiting for resources held by other runs. Waiting for an earlier run of
    /// the same request happens before it starts.
    pub fn waited(&self) -> u64 {
        self.steps.iter().map(|s| s.waited).sum()
    }

    /// What its running step holds and waits for.
    pub fn holds(&self) -> Holds {
        self.steps.iter().find(|s| s.status == Status::Running).map(|s| s.holds.clone()).unwrap_or_default()
    }

    /// Shows what `request` does.
    pub(crate) fn describe(&mut self, request: &Request) {
        (self.label, self.kind) = (request.label(&self.collection), request.kind());
        self.scope = request.scope(&self.collection).map(str::to_owned);
    }
}

/// A run's record and its log `runs/<collection>/<id>.log`, written as the run goes, so that any process can
/// follow it and a crash leaves it behind. Its lock, held until drop, shows that it is queued or
/// running. Dropped while queued, it leaves the queue, unless the process is stopping: then the
/// server queues it again at its next start.
pub(crate) struct RunLog {
    store: Store,
    pub record: RunRecord,
    pub request: Request,
    _held: Option<File>,
    /// Its record is written, so the run is visible.
    saved: bool,
    log: Option<File>,
    written: u64,
    /// Past [`LOG_CAP`]: the last lines, and how many lines came.
    tail: VecDeque<String>,
    past_cap: u64,
}

impl Drop for RunLog {
    fn drop(&mut self) {
        if self.record.status == Status::Queued && !clock::interrupted() {
            _ = self.store.dequeue(&self.record.collection, self.record.id);
        }
    }
}

impl RunLog {
    /// A new run of `collection`, starting now. Nothing is written until it logs.
    pub(crate) fn new(store: &Store, collection: &str, request: Request, trigger: Trigger) -> Self {
        let id = store.now().as_millisecond();
        Self::with_id(store, collection, id, request, trigger, Status::Running)
    }

    /// Queues `request`, visible at once, with a unique id.
    pub(crate) fn queued(store: &Store, collection: &str, request: Request, trigger: Trigger) -> Result<Self> {
        let mut run = store.write(Shown::Of(collection), |w| {
            let table = w.open_table(RUNS)?;
            let last = table.range(of(collection))?.next_back().transpose()?;
            let id = store.now().as_millisecond().max(last.map_or(i64::MIN, |(key, _)| key.value().1 + 1));
            let run = Self::with_id(store, collection, id, request, trigger, Status::Queued);
            drop(table);
            put_run(w, &run.record, &run.request)?;
            Ok(run)
        })?;
        run.saved = true;
        Ok(run)
    }

    fn with_id(store: &Store, collection: &str, id: i64, request: Request, trigger: Trigger, status: Status) -> Self {
        let held = store.lock_file(&run_lock(collection, id)).ok().filter(|f| f.try_lock().is_ok());
        let mut record = RunRecord {
            id,
            collection: collection.to_owned(),
            label: String::new(),
            kind: request.kind(),
            scope: None,
            trigger,
            pid: std::process::id(),
            started: store.now(),
            finished: None,
            status,
            results: Vec::new(),
            calls: Vec::new(),
            steps: Vec::new(),
        };
        record.describe(&request);
        let store = store.clone();
        Self {
            store,
            record,
            request,
            _held: held,
            saved: false,
            log: None,
            written: 0,
            tail: VecDeque::new(),
            past_cap: 0,
        }
    }

    /// Replaces the request, as a check that took its data subset.
    pub(crate) fn set_request(&mut self, request: Request) {
        self.record.describe(&request);
        self.request = request;
    }

    fn path(&self) -> PathBuf {
        self.store.log_path(&self.record.collection, self.record.id)
    }

    pub(crate) fn start(&mut self) {
        (self.record.status, self.record.started) = (Status::Running, self.store.now());
        self.resave();
    }

    /// The first line also writes the record, so a run that logs nothing leaves no trace.
    pub(crate) fn line(&mut self, origin: Origin, line: &str) {
        // Written at once, so that a follower never reads half of it.
        let line = LogLine { at: Some(self.store.now()), origin, text: line.to_owned() }.json() + "\n";
        if self.log.is_none() && !self.store.is_dry_run() {
            let path = self.path();
            if let Some(parent) = path.parent() {
                _ = fs::create_dir_all(parent).map(|()| self.save());
            }
            self.log = OpenOptions::new().create(true).append(true).open(path).ok();
            if let Some(log) = self.log.as_mut().filter(|l| l.metadata().is_ok_and(|m| m.len() == 0)) {
                _ = writeln!(log, "{}", serde_json::json!({ "version": LogLine::MIGRATIONS.len() }));
            }
        }
        if self.written >= LOG_CAP {
            self.past_cap += 1;
            self.tail.push_back(line.clone());
            if self.tail.len() > LOG_CAP_LINES {
                self.tail.pop_front();
            }
        } else if let Some(log) = &mut self.log {
            self.written += line.len() as u64;
            _ = log.write_all(line.as_bytes());
        }
    }

    pub(crate) fn waited(&mut self, step: Option<usize>, secs: u64) {
        if let Some(s) = step.and_then(|s| self.record.steps.get_mut(s)) {
            s.waited += secs;
        }
    }

    /// Saved immediately if the run is already visible. How long each step waited is counted here.
    pub(crate) fn steps(&mut self, steps: &[StepRecord]) {
        let waited: Vec<u64> = self.record.steps.iter().map(|s| s.waited).collect();
        self.record.steps = steps.to_vec();
        self.record.steps.iter_mut().zip(waited).for_each(|(s, w)| s.waited = w);
        self.resave();
    }

    /// Saved immediately if the run is already visible, so that every process can show it.
    pub(crate) fn calls(&mut self, calls: &[Call]) {
        self.record.calls = calls.to_vec();
        self.resave();
    }

    /// What `step` holds and waits for. Saved immediately, so that every process can show it, and the
    /// runs waiting for it name it: a run that holds locks is visible before it logs.
    /// Returns whether they changed: a run waiting for a lock is told so again at every change of the state,
    /// which its own save would be.
    pub(crate) fn holds(&mut self, step: Option<usize>, holds: Holds) -> bool {
        let Some(s) = step.and_then(|s| self.record.steps.get_mut(s)).filter(|s| s.holds != holds) else {
            return false;
        };
        let holding = !holds.using.is_empty();
        s.holds = holds;
        if holding && !self.store.is_dry_run() {
            self.save();
        } else {
            self.resave();
        }
        true
    }

    fn save(&mut self) {
        self.saved = self.store.save_run(&self.record, &self.request).is_ok();
    }

    fn resave(&mut self) {
        if self.saved {
            self.save();
        }
    }

    /// Records the outcome, and drops the oldest runs.
    pub(crate) fn finish(&mut self, status: Status, results: &[StepResult], calls: Vec<Call>) -> Result<()> {
        if let Some(log) = &mut self.log
            && self.past_cap > 0
        {
            writeln!(
                log,
                "{}",
                LogLine::note(LogNote::LeftOut { lines: self.past_cap - self.tail.len() as u64 }.to_string()).json()
            )?;
            self.tail.iter().try_for_each(|line| log.write_all(line.as_bytes()))?;
        }
        let results = results.iter().map(ResultRecord::from).collect();
        (self.record.finished, self.record.status, self.record.results, self.record.calls) =
            (Some(self.store.now()), status, results, calls);
        self.store.save_run(&self.record, &self.request)?;
        self.store.trim_runs(&self.record.collection)
    }
}

/// The keys of `collection`'s runs.
fn of(collection: &str) -> std::ops::RangeInclusive<(&str, i64)> {
    (collection, i64::MIN)..=(collection, i64::MAX)
}

/// A stored run, parsed only if it changed since it was last read.
fn parse_run(kept: &mut BTreeMap<i64, Kept<RunRecord>>, id: i64, text: &str) -> Option<RunRecord> {
    let run = kept.entry(id).or_default();
    *run = reparse(run.take(), text, stored);
    Some(run.as_ref()?.1.clone())
}

/// Writes the record, and adds it to or removes it from the queue.
fn put_run(w: &WriteTransaction, record: &RunRecord, request: &Request) -> Result<()> {
    let key = (record.collection.as_str(), record.id);
    let mut row = shape::to_value(record)?;
    row["request"] = serde_json::to_value(request)?;
    w.open_table(RUNS)?.insert(key, row.to_string().as_str())?;
    let mut unfinished = w.open_table(UNFINISHED)?;
    if record.status.unfinished() {
        unfinished.insert(key, ())?
    } else {
        unfinished.remove(key)?
    };
    Ok(())
}

impl Store {
    fn log_path(&self, collection: &str, id: i64) -> PathBuf {
        self.dir.join("runs").join(collection).join(format!("{id}.log"))
    }

    fn save_run(&self, record: &RunRecord, request: &Request) -> Result<()> {
        self.write(Shown::Of(&record.collection), |w| put_run(w, record, request))
    }

    /// Removes a queued run. Returns whether it was queued.
    pub fn dequeue(&self, collection: &str, id: i64) -> Result<bool> {
        self.write(Shown::Of(collection), |w| {
            let mut table = w.open_table(RUNS)?;
            let queued = table
                .get((collection, id))?
                .and_then(|r| stored::<RunRecord>(r.value()))
                .is_some_and(|r| r.status == Status::Queued);
            if queued {
                table.remove((collection, id))?;
                w.open_table(UNFINISHED)?.remove((collection, id))?;
            }
            Ok(queued)
        })
    }

    /// Whether its process still holds its lock. The lock of an ended process is removed, once.
    pub fn alive(&self, run: &RunRecord) -> bool {
        let path = self.lock_path(&run_lock(&run.collection, run.id));
        match OpenOptions::new().write(true).open(&path).map(|f| f.try_lock()) {
            Ok(Err(TryLockError::WouldBlock)) => true,
            Ok(_) => {
                _ = fs::remove_file(path);
                false
            }
            Err(_) => false,
        }
    }

    /// The live runs, of any collection and process, whose running step holds the resource `name`.
    pub fn holding(&self, name: &str) -> Vec<RunRecord> {
        let holds = |r: &RunRecord| r.holds().using.iter().any(|u| u == name);
        self.unfinished(None).into_iter().filter(|r| holds(r) && self.alive(r)).collect()
    }

    /// Waits until `run` ends, or its process does.
    pub fn wait_for(&self, cx: &Ctx, run: &RunRecord, waiting: impl Fn(&str)) -> Result<()> {
        let lock = run_lock(&run.collection, run.id);
        self.lock(cx, &[(&lock, Mode::Exclusive)], waiting, || Ok(None)).map(drop)
    }

    /// The lock of the queued run of `request` that came before `run`, if its process is still waiting.
    /// Fails once `run` is no longer queued (cancelled).
    pub fn ahead(&self, run: &RunRecord, request: &Request) -> Result<Option<String>> {
        let queued: Vec<Queued> =
            self.queue(Some(&run.collection)).into_iter().filter(|q| q.record.status == Status::Queued).collect();
        if !queued.iter().any(|q| q.record.id == run.id) {
            return Err(Error::Dequeued.into());
        }
        let same = |q: &Queued| q.request.as_ref().is_some_and(|r| r.key() == request.key());
        let ahead = queued.iter().rev().find(|q| q.record.id < run.id && same(q) && self.alive(&q.record));
        Ok(ahead.map(|q| run_lock(&q.record.collection, q.record.id)))
    }

    /// Removes queued runs whose process ended, and returns those the web UI requested, to queue them
    /// again.
    pub fn orphans(&self) -> Result<Vec<Queued>> {
        let queued = self.queue(None).into_iter().filter(|q| q.record.status == Status::Queued);
        let orphans: Vec<Queued> = queued.filter(|q| !self.alive(&q.record)).collect();
        self.write(Shown::All, |w| {
            let (mut runs, mut unfinished) = (w.open_table(RUNS)?, w.open_table(UNFINISHED)?);
            for key in orphans.iter().map(|q| (q.record.collection.as_str(), q.record.id)) {
                runs.remove(key)?;
                unfinished.remove(key)?;
            }
            Ok(())
        })?;
        Ok(orphans.into_iter().filter(|q| q.record.trigger == Trigger::Web).collect())
    }

    /// Keeps the collection's newest [`MAX_RUNS`] with their logs, plus every queued or running run, so that
    /// a long run is not dropped by the runs that end around it.
    fn trim_runs(&self, collection: &str) -> Result<()> {
        let old: Vec<i64> = self.write(Shown::Of(collection), |w| {
            let mut table = w.open_table(RUNS)?;
            let ended = |record: &str| stored::<RunRecord>(record).is_none_or(|r| !r.status.unfinished());
            let ids: Vec<i64> = table
                .range(of(collection))?
                .rev()
                .skip(MAX_RUNS)
                .map(|e| e.map(|(key, record)| ended(record.value()).then(|| key.value().1)))
                .filter_map(Result::transpose)
                .collect::<Result<_, _>>()?;
            ids.iter().try_for_each(|id| table.remove((collection, *id)).map(drop))?;
            Ok(ids)
        })?;
        for id in old {
            _ = fs::remove_file(self.log_path(collection, id));
        }
        Ok(())
    }

    /// The newest `limit`, and every unfinished one, newest first.
    pub fn runs(&self, collection: &str, limit: usize) -> Vec<RunRecord> {
        let runs = self.read(|r| {
            let (table, unfinished) = (r.open_table(RUNS)?, r.open_table(UNFINISHED)?);
            let mut ids = BTreeSet::new();
            for key in table.range(of(collection))?.rev().take(limit) {
                ids.insert(key?.0.value().1);
            }
            for key in unfinished.range(of(collection))? {
                ids.insert(key?.0.value().1);
            }
            // Runs removed since are older than the oldest left, but for dequeued ones, which are few.
            let oldest = table.range(of(collection))?.next().transpose()?.map(|(key, _)| key.value().1);
            let mut parsed = self.parsed.runs.lock();
            let kept = parsed.entry(collection.to_owned()).or_default();
            kept.retain(|id, _| oldest.is_some_and(|oldest| *id >= oldest));
            let mut runs = Vec::new();
            for id in ids.into_iter().rev() {
                runs.extend(table.get((collection, id))?.and_then(|run| parse_run(kept, id, run.value())));
            }
            Ok(runs)
        });
        runs.unwrap_or_default()
    }

    /// How many stored runs this version cannot read. They are not shown.
    #[cfg(feature = "server")]
    pub(crate) fn unreadable_runs(&self) -> usize {
        let count = self.read(|r| {
            let runs = r.open_table(RUNS)?;
            Ok(runs.iter()?.filter(|e| e.as_ref().is_ok_and(|e| stored::<RunRecord>(e.1.value()).is_none())).count())
        });
        count.unwrap_or_default()
    }

    /// Queued or running runs of `collection` (else of every collection), including those left by an ended process.
    /// Oldest first.
    pub fn unfinished(&self, collection: Option<&str>) -> Vec<RunRecord> {
        self.queue(collection).into_iter().map(|q| q.record).collect()
    }

    /// [`Self::unfinished`], with their requests.
    pub fn queue(&self, collection: Option<&str>) -> Vec<Queued> {
        let runs = self.read(|r| {
            let (unfinished, runs) = (r.open_table(UNFINISHED)?, r.open_table(RUNS)?);
            let keys = match collection {
                Some(collection) => unfinished.range(of(collection))?,
                None => unfinished.iter()?,
            };
            let record = |key: (&str, i64)| stored(runs.get(key).ok()??.value());
            Ok(keys.filter_map(|e| record(e.ok()?.0.value())).collect())
        });
        runs.unwrap_or_default()
    }

    pub fn run(&self, collection: &str, id: i64) -> Option<RunRecord> {
        let run = self.read(|r| {
            let table = r.open_table(RUNS)?;
            let Some(run) = table.get((collection, id))? else { return Ok(None) };
            Ok(parse_run(self.parsed.runs.lock().entry(collection.to_owned()).or_default(), id, run.value()))
        });
        run.ok().flatten()
    }

    /// Whole lines of a run's log from byte `from`, else its end, and the offset after them.
    pub fn run_log_tail(&self, collection: &str, id: i64, from: Option<u64>) -> (Vec<LogLine>, u64) {
        let Ok(mut file) = File::open(self.log_path(collection, id)) else { return (Vec::new(), from.unwrap_or(0)) };
        let mut first = String::new();
        _ = std::io::BufRead::read_line(&mut std::io::BufReader::new(&file), &mut first);
        let version = serde_json::from_str(&first).ok().as_ref().and_then(shape::version);
        let start = from.unwrap_or_else(|| file.metadata().map_or(0, |m| m.len().saturating_sub(LOG_TAIL)));
        let mut bytes = Vec::new();
        if file.seek(SeekFrom::Start(start)).and_then(|_| file.read_to_end(&mut bytes)).is_err() {
            return (Vec::new(), start);
        }
        // A line still being written is read next time.
        bytes.truncate(bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1));
        let text = String::from_utf8_lossy(&bytes);
        let header = usize::from(start == 0 && version.is_some());
        let parse = |line| LogLine::parse(line, version.unwrap_or_default());
        let mut lines: Vec<LogLine> = text.lines().skip(header).map(parse).collect();
        if from.is_none() && start > 0 && !lines.is_empty() {
            // The first line may be cut.
            lines[0] = LogLine::note(LogNote::Cut { size: fmt::human_size(start) }.to_string());
        }
        (lines, start + bytes.len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::{rustic::Reading, state::tests::store};

    /// Runs of each outcome, as this version stores them.
    fn runs_of_each_outcome(store: &Store) -> Vec<RunRecord> {
        type Planned<'a> = (&'a str, Status, Vec<usize>, Vec<StepResult>);
        let ended = |steps: &[Planned<'_>], status| {
            let mut run = RunLog::new(store, "Docs", Request::Backup, Trigger::Schedule);
            let step = |(label, status, needs, lines): &Planned<'_>| StepRecord {
                status: *status,
                results: lines.iter().map(ResultRecord::from).collect(),
                waited: 2,
                ..StepRecord::queued((*label).into(), needs.clone())
            };
            run.steps(&steps.iter().map(step).collect::<Vec<_>>());
            let (args, started) = ("-P Docs backup".into(), store.now());
            let call = Call { step: Some(0), args, bin: None, dir: None, started, secs: 1.0, exit: Some("0".into()) };
            run.finish(status, &[], vec![call]).unwrap();
            run.record.clone()
        };
        let copy_failed =
            StepResult::Failed { step: "copy to Docs-usb".into(), error: "unreachable".into(), stopped: false };
        let unmet = StepResult::Unmet { step: "forget on Docs".into(), needs: vec!["copy to Docs-usb".into()] };
        let skipped = StepResult::Skipped { items: vec![("/docs/a".into(), "permission denied".into())], count: 1 };
        let check = Request::Check(vec![("Docs".into(), Some(Reading::Subset { n: 1, of: 10 }))]);
        vec![
            ended(&[("snapshot", Status::Succeeded, Vec::new(), vec![skipped])], Status::Succeeded),
            ended(
                &[
                    ("copy to Docs-usb", Status::Failed, Vec::new(), vec![copy_failed]),
                    ("forget on Docs", Status::Skipped, vec![0], vec![unmet]),
                ],
                Status::Failed,
            ),
            ended(&[("snapshot", Status::Cancelled, Vec::new(), Vec::new())], Status::Cancelled),
            RunLog::queued(store, "Docs", check, Trigger::Web).unwrap().record.clone(),
        ]
    }

    /// Recorded once in `tests/fixtures/runs.jsonl`. Deleting it records this version's runs again, and
    /// drops the shapes that earlier versions stored.
    #[test]
    fn runs_stored_by_earlier_versions_are_still_read() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/runs.jsonl");
        if !path.exists() {
            let store = store("runs-fixture");
            let runs: Vec<String> =
                runs_of_each_outcome(&store).iter().map(|r| shape::to_value(r).unwrap().to_string()).collect();
            fs::write(&path, runs.join("\n") + "\n").unwrap();
            fs::remove_dir_all(&store.dir).unwrap();
        }
        for line in fs::read_to_string(&path).unwrap().lines() {
            assert!(
                stored::<Queued>(line).is_some_and(|q| !q.record.label.is_empty() && q.request.is_some()),
                "{line}"
            );
            // A request in a shape this version does not know still shows the run.
            let mut changed: serde_json::Value = serde_json::from_str(line).unwrap();
            changed["request"] = serde_json::json!({ "Unknown": 1 });
            assert!(stored::<Queued>(&changed.to_string()).is_some_and(|q| q.request.is_none()));
        }
    }

    #[test]
    fn runs_are_written_as_they_go_and_the_oldest_dropped() {
        let store = store("runs-test");
        // A run that logs nothing leaves no trace.
        RunLog::new(&store, "Docs", Request::Backup, Trigger::Schedule);
        assert!(store.runs("Docs", usize::MAX).is_empty());

        let mut run = RunLog::new(&store, "Docs", Request::Backup, Trigger::Cli);
        run.line(Origin::default(), "first");
        let listed = store.runs("Docs", usize::MAX);
        assert!(listed.len() == 1 && listed[0].finished.is_none(), "known as soon as it logs");
        let (lines, end) = store.run_log_tail("Docs", run.record.id, None);
        let first = &lines[0];
        let size = fs::metadata(store.log_path("Docs", run.record.id)).unwrap().len();
        assert!(
            lines.len() == 1 && first.at.is_some() && first.origin == Origin::default() && end == size,
            "{lines:?}"
        );
        // Each line keeps its step and rustic call.
        let origin = Origin { step: Some(1), call: Some(0) };
        run.line(origin, "second");
        let (lines, _) = store.run_log_tail("Docs", run.record.id, Some(end));
        assert!(lines.len() == 1 && lines[0].origin == origin && lines[0].text == "second", "read on: {lines:?}");
        assert_eq!(store.unfinished(Some("Docs")).len(), 1);
        run.finish(Status::Succeeded, &[StepResult::RunningAlready], Vec::new()).unwrap();
        assert!(store.runs("Docs", 1)[0].finished.is_some() && store.unfinished(None).is_empty());

        // A running check is kept, however many runs end after it.
        let mut check = RunLog::new(&store, "Docs", Request::Check(vec![("Docs".into(), None)]), Trigger::Schedule);
        check.record.id = 1_999_999_999_999;
        check.line(Origin::default(), "reading");
        for i in 0..i64::try_from(MAX_RUNS).unwrap() {
            let mut run = RunLog::new(&store, "Docs", Request::Copy("Docs".into()), Trigger::Web);
            run.record.id = 2_000_000_000_000 + i;
            run.line(Origin::default(), "x");
            run.finish(Status::Succeeded, &[], Vec::new()).unwrap();
        }
        let runs = store.runs("Docs", usize::MAX);
        assert!(runs.len() == MAX_RUNS + 1 && runs.iter().all(|r| r.kind != RunKind::Backup), "the oldest went");
        assert!(runs.iter().any(|r| r.kind == RunKind::Check && r.finished.is_none()), "the check stays");
        let newest: Vec<RunKind> = store.runs("Docs", 1).iter().map(|r| r.kind).collect();
        assert_eq!(newest, [RunKind::Copy, RunKind::Check], "the newest, and those not over");
        // Telling a run again what it holds saves nothing, else a run waiting for a lock would wake itself.
        let using = Holds { using: vec!["Docs".into()], waiting_for: None };
        check.steps(&[StepRecord::queued("check".into(), Vec::new())]);
        assert!(check.holds(Some(0), using.clone()) && !check.holds(Some(0), using));
        assert_eq!(fs::read_dir(store.dir.join("runs/Docs")).unwrap().count(), MAX_RUNS + 1);
        fs::remove_dir_all(&store.dir).unwrap();
    }

    #[test]
    fn queued_runs_take_their_turn() {
        let a = store("queue-test");
        let b = Store::at(a.dir.clone()).unwrap();
        // Requested at the same time by two processes: they run one after the other.
        let first = RunLog::queued(&a, "Docs", Request::Backup, Trigger::Cli).unwrap();
        let second = RunLog::queued(&b, "Docs", Request::Backup, Trigger::Web).unwrap();
        let other = RunLog::queued(&b, "Docs", Request::Prune(vec!["Docs".into()]), Trigger::Web).unwrap();
        assert!(second.record.id > first.record.id);
        assert_eq!(a.unfinished(Some("Docs")).len(), 3);
        assert!(
            a.ahead(&first.record, &first.request).unwrap().is_none()
                && b.ahead(&second.record, &second.request).unwrap().is_some()
        );
        assert!(b.ahead(&other.record, &other.request).unwrap().is_none(), "another request doesn't wait");
        // Cancelled: it leaves the queue, and the next run's turn comes.
        assert!(a.dequeue("Docs", first.record.id).unwrap());
        assert!(
            a.ahead(&first.record, &first.request).is_err()
                && b.ahead(&second.record, &second.request).unwrap().is_none()
        );
        // Left by an ended process: those from the web UI are queued again.
        for mut run in [first, second, other] {
            #[expect(clippy::used_underscore_binding, reason = "releases the lock as if the process ended")]
            {
                run._held = None;
            }
            std::mem::forget(run);
        }
        let mut orphans: Vec<RunKind> = a.orphans().unwrap().iter().map(|q| q.record.kind).collect();
        orphans.sort_by_key(|o| <&str>::from(*o));
        assert_eq!(orphans, [RunKind::Backup, RunKind::Prune]);
        assert!(a.runs("Docs", usize::MAX).is_empty() && a.unfinished(None).is_empty());
        fs::remove_dir_all(&a.dir).unwrap();
    }

    #[test]
    fn a_long_log_keeps_its_start_and_end() {
        // A fixed time, so that each line has the same length.
        let store = store("log-cap-test").with_clock(clock::Fixed::at("2026-10-16T12:00:00Z"));
        let mut run = RunLog::new(&store, "Docs", Request::Backup, Trigger::Cli);
        // 1 KiB as a JSON line.
        let line = "x".repeat(960);
        for _ in 0..(LOG_CAP >> 10) + 5000 {
            run.line(Origin::default(), &line);
        }
        run.line(Origin::default(), "the end");
        run.finish(Status::Succeeded, &[], Vec::new()).unwrap();
        let size = fs::metadata(store.log_path("Docs", run.record.id)).unwrap().len();
        assert!(size < LOG_CAP + (LOG_CAP_LINES as u64 + 1) * 1024, "{size}");
        let (tail, _) = store.run_log_tail("Docs", run.record.id, None);
        assert_eq!(tail.last().unwrap().text, "the end");
        let file = fs::read_to_string(store.log_path("Docs", run.record.id)).unwrap();
        let left_out = LogNote::LeftOut { lines: 5001 - LOG_CAP_LINES as u64 }.to_string();
        assert!(file.contains(&left_out));
        fs::remove_dir_all(&store.dir).unwrap();
    }
}
