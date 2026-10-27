//! Prometheus metrics.

use std::{collections::BTreeMap, sync::Arc};

use axum::{
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};
use jiff::Timestamp;
use strum::IntoEnumIterator;

use super::{App, VERSION, blocking};
use crate::{NAME, log::Level, request::StepKey, rustic::newest_summary};

/// A metric: its name, after our prefix unless it is a standard one, and as `Display` its help.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum::IntoStaticStr, crate::i18n::Fluent)]
#[strum(serialize_all = "snake_case")]
enum Metric {
    ProcessStartTimeSeconds,
    BuildInfo,
    #[fluent(transparent = health())]
    CollectionStatus,
    CollectionRunning,
    Issues,
    NewestSnapshotTimestampSeconds,
    BehindSinceTimestampSeconds,
    #[fluent(transparent = health())]
    RepositoryStatus,
    Snapshots,
    StoredBytes,
    SnapshotFiles,
    SnapshotProcessedBytes,
    SnapshotAddedBytes,
    SnapshotDurationSeconds,
    LastSuccessTimestampSeconds,
    ConsecutiveFailures,
}

/// The statuses' help: each level's number and word, `0 healthy, 1 degraded, 2 failing`.
fn health() -> String {
    Level::iter().map(|l| format!("{} {}", l as u8, l.word())).collect::<Vec<_>>().join(", ")
}

impl Metric {
    fn name(self) -> String {
        let name: &str = self.into();
        if self == Self::ProcessStartTimeSeconds { name.to_owned() } else { format!("{NAME}_{name}") }
    }
}

/// The collections' status as numbers, in Prometheus' text format, with times in Unix seconds.
pub(super) async fn metrics(State(app): State<Arc<App>>) -> Response {
    blocking(move || {
        let engine = app.engine();
        let mut families: BTreeMap<Metric, Vec<String>> = BTreeMap::new();
        let mut add = |metric: Metric, labels: &[(&str, &str)], value: f64| {
            let labels: Vec<String> = labels
                .iter()
                .map(|(k, v)| format!("{k}=\"{}\"", v.replace('\\', r"\\").replace('"', "\\\"").replace('\n', r"\n")))
                .collect();
            let labels = if labels.is_empty() { String::new() } else { format!("{{{}}}", labels.join(",")) };
            families.entry(metric).or_default().push(format!("{}{labels} {value}", metric.name()));
        };
        let seconds = |t: Timestamp| t.as_duration().as_secs_f64();
        add(Metric::ProcessStartTimeSeconds, &[], seconds(app.started));
        add(Metric::BuildInfo, &[("version", VERSION)], 1.0);
        for s in engine.status() {
            let (collection, title) = (s.primary.name.as_str(), s.title.as_str());
            let one = [("collection", title)];
            add(Metric::CollectionStatus, &one, f64::from(s.level as u8));
            add(Metric::CollectionRunning, &one, f64::from(u8::from(!s.running.is_empty())));
            let mut kinds: BTreeMap<&str, u32> = BTreeMap::new();
            for issue in &s.issues {
                *kinds.entry(issue.into()).or_default() += 1;
            }
            for (kind, count) in kinds {
                add(Metric::Issues, &[one[0], ("kind", kind)], f64::from(count));
            }
            for r in std::iter::once(&s.primary).chain(&s.targets) {
                let labels = [("collection", title), ("repository", r.name.as_str())];
                let (c, n) = (collection.to_owned(), r.name.clone());
                let steps = [
                    (StepKey::Snapshot(c.clone()), s.snapshot.as_ref().filter(|_| n == collection)),
                    (StepKey::Copy(c.clone(), n.clone()), r.copy.as_ref()),
                    (StepKey::Forget(c, n.clone()), r.forget.as_ref()),
                    (StepKey::Prune(n.clone()), r.prune.as_ref()),
                    (StepKey::Check(n), r.check.as_ref()),
                ];
                for (key, last) in steps.iter().filter_map(|(k, l)| Some((k, (*l)?))) {
                    let labels = [labels[0], labels[1], ("step", key.into())];
                    if let Some(t) = last.last_ok.filter(|_| !matches!(key, StepKey::Snapshot(_))) {
                        add(Metric::LastSuccessTimestampSeconds, &labels, seconds(t));
                    }
                    add(Metric::ConsecutiveFailures, &labels, f64::from(last.failures));
                }
                if let Some(t) = r.latest_snapshot {
                    add(Metric::NewestSnapshotTimestampSeconds, &labels, seconds(t));
                }
                if let Some(t) = r.lag.as_ref().and_then(|l| l.behind_since) {
                    add(Metric::BehindSinceTimestampSeconds, &labels, seconds(t));
                }
                add(Metric::RepositoryStatus, &labels, f64::from(r.status.level() as u8));
                if let Some(Ok(info)) = engine.store.inventory(collection, &r.name).map(|l| l.result) {
                    add(Metric::Snapshots, &labels, info.snapshots.len() as f64);
                    if let Some(stored) = info.stored {
                        add(Metric::StoredBytes, &labels, stored as f64);
                    }
                    if let Some(sum) = newest_summary(&info.snapshots) {
                        add(Metric::SnapshotFiles, &labels, sum.total_files_processed as f64);
                        add(Metric::SnapshotProcessedBytes, &labels, sum.total_bytes_processed as f64);
                        add(Metric::SnapshotAddedBytes, &labels, sum.data_added_packed as f64);
                        add(Metric::SnapshotDurationSeconds, &labels, sum.backup_duration);
                    }
                }
            }
        }
        let mut text = String::new();
        for (metric, samples) in families {
            let (name, help) = (metric.name(), metric);
            text += &format!("# HELP {name} {help}\n# TYPE {name} gauge\n");
            text.extend(samples.iter().map(|s| format!("{s}\n")));
        }
        Ok(([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], text).into_response())
    })
    .await
}
