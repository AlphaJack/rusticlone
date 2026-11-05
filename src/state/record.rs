//! What is stored of a run: its record, its steps' results and its log's lines. Each is a stored shape
//! that every later release must read back.

use anyhow::Result;
use jiff::Timestamp;
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};

use crate::{
    log::{Level, Message},
    request::{Request, RunKind, Trigger},
    result::{self, Status, StepResult},
    rustic::{Call, Origin},
    shape::{self, Versioned},
};

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
    pub(super) fn note(text: String) -> Self {
        Self { at: None, origin: Origin::default(), text }
    }

    pub(super) fn json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// `line` written in the shape of `version`.
    pub(super) fn parse(line: &str, version: usize) -> Self {
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
pub(super) enum LogNote {
    /// Past `LOG_CAP`.
    LeftOut { lines: u64 },
    /// Read from its end, with the size before.
    Cut { size: String },
}

/// A run of a collection and its report. Its log is a separate file. It keeps what is shown, not the
/// types that made it, so that a later version can still show it. Its request is stored beside it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    /// When it was requested, in milliseconds. Also its position in the queue.
    pub id: i64,
    /// The collection it runs.
    pub collection: String,
    /// `prune primary`, `check primary 3/10`.
    pub label: String,
    /// What it does.
    pub kind: RunKind,
    /// The repository it acts on, if one.
    pub scope: Option<String>,
    /// What started it.
    pub trigger: Trigger,
    /// The process that runs it.
    pub pid: u32,
    /// When it started. While queued: when it was requested.
    pub started: Timestamp,
    /// Unset while it runs, or if the process stopped before its end.
    pub finished: Option<Timestamp>,
    /// How far it got.
    pub status: Status,
    /// Results that no step produced, such as a request that could not be planned.
    pub results: Vec<ResultRecord>,
    /// The rustic calls it made, in the order they started.
    pub calls: Vec<Call>,
    /// Its planned steps, and how far each got.
    pub steps: Vec<StepRecord>,
}

/// The resources a step holds and waits for, by the names the tables show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holds {
    pub using: Vec<String>,
    pub waiting_for: Option<String>,
}

/// A result as `--json` prints it: its kind and values, its level and its text. Unlike a
/// [`StepResult`], it reads back whatever kinds results had when it was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
pub(super) fn stored<T: DeserializeOwned>(text: &str) -> Option<T> {
    let mut value = serde_json::from_str(text).ok()?;
    shape::migrate::<RunRecord>(&mut value);
    serde_json::from_value(value).ok()
}

/// A stored run read back with its request, to queue or wait for it.
#[derive(Debug, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        result.within(&std::iter::once(self.label.as_str()).chain(self.repo.as_deref()).collect::<Vec<_>>())
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The first shape's requests had tags in Rust's case: a test restore's snapshot and a re-create are read in
    /// this version's.
    #[test]
    fn old_request_tags_read_as_kebab_case() {
        let migrated = |mut request: serde_json::Value| {
            kebab_request(&mut request);
            request
        };
        assert_eq!(
            migrated(json!({"TestRestore": [null, {"Newest": null}]})),
            json!({"test-restore": [null, {"newest": null}]})
        );
        assert_eq!(
            migrated(json!({"Recreate": {"repo": "Docs", "from": null}})),
            json!({"re-create": {"repo": "Docs", "from": null}})
        );
    }

    /// A stored result keeps its level.
    #[test]
    fn a_stored_result_keeps_its_level() {
        let failed = StepResult::Error { error: "unreachable".into() };
        assert_eq!(Message::level(&ResultRecord::from(&failed)), Level::Failure);
    }
}
