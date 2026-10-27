//! The [`Ledger`] and its changes. Every change is an [`Event`], applied by [`Ledger::apply`]; the
//! store writes only what an event changed.

use std::collections::{BTreeMap, BTreeSet};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::{Checks, Lag, LastStep, Outcome, ResultRecord, RunStep};
use crate::{
    request::StepKey,
    rustic::SnapshotId,
    shape::{self, Versioned},
};

/// Holds what is shown, as the ledger does, so that events read back as it does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Event {
    /// A step that started at `started` ended at `at`.
    Attempted {
        key: StepKey,
        started: Timestamp,
        at: Timestamp,
        result: Outcome,
        #[serde(default)]
        by: Option<RunStep>,
    },
    /// What a successful backup left out, such as unreadable items. Cleared by a backup without
    /// warnings.
    Warned {
        key: StepKey,
        warnings: Vec<ResultRecord>,
    },
    /// The user dismissed a step's failure, cancellation or warning.
    Dismissed {
        key: StepKey,
    },
    /// New schedules. A step never done is due at its first scheduled time after `at`.
    FirstSeen {
        keys: Vec<StepKey>,
        at: Timestamp,
    },
    /// A schedule was removed. If set again, it starts from scratch.
    Unscheduled {
        key: StepKey,
    },
    /// A backup ran, so the collection's targets may lack snapshots.
    Pending {
        collection: String,
        targets: Vec<String>,
    },
    /// A target has every snapshot of the collection's primary.
    Copied {
        collection: String,
        target: String,
    },
    Newest {
        repo: String,
        at: Timestamp,
    },
    /// How far a target is behind its primary, measured at `at`.
    Lag {
        collection: String,
        target: String,
        at: Timestamp,
        behind: Behind,
    },
    /// A check of a repository took a data subset (of `of`): an owed one first, else the next one.
    CheckTaken {
        repo: String,
        of: u32,
    },
    CheckRead {
        repo: String,
        n: u32,
        read: Read,
    },
    /// Snapshots of the collection that a target's own forget removed. Copying them again would only remove them
    /// again. Snapshots older than the oldest one kept do not matter.
    Dropped {
        collection: String,
        target: String,
        removed: Vec<(SnapshotId, Timestamp)>,
        oldest_kept: Option<Timestamp>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Read {
    Through,
    Failed,
    /// Stopped or never run: owed.
    Not,
}

/// What a target lacks of its primary's snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Behind {
    /// Listed: behind since its oldest missing snapshot, if any.
    Listed(Option<Timestamp>),
    /// Not listed: behind since the known time, else since the primary's newest snapshot.
    Unlisted(Option<Timestamp>),
}

/// What is known of every collection: all events applied so far, read in one piece.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    pub last_steps: BTreeMap<StepKey, LastStep>,
    pub checks: BTreeMap<String, Checks>,
    /// By collection and target.
    #[serde(with = "entries")]
    pub lag: BTreeMap<(String, String), Lag>,
    /// Profiles and targets with snapshots to copy.
    pub pending: BTreeSet<(String, String)>,
    /// The newest snapshot seen, by repository.
    pub latest: BTreeMap<String, Timestamp>,
    /// When each step's schedule was first seen. A step never done runs at its first scheduled time
    /// after that, like a persistent systemd timer.
    pub scheduled: BTreeMap<StepKey, Timestamp>,
    /// Snapshots each target's forget removed, by collection and target.
    #[serde(with = "entries")]
    pub dropped: BTreeMap<(String, String), Vec<(SnapshotId, Timestamp)>>,
}

impl Versioned for Event {
    const MIGRATIONS: &'static [shape::Migration] = &[shape::Migration {
        up: |v| ["behind", "read"].into_iter().for_each(|key| _ = v.get_mut(key).and_then(shape::kebab)),
        read_by_previous: false,
    }];
}

impl Versioned for Ledger {
    const MIGRATIONS: &'static [shape::Migration] = &[];
}

impl Ledger {
    /// A stored ledger in this version's shape. `None` if unreadable.
    pub(super) fn from_stored(text: &str) -> Option<Self> {
        shape::read(serde_json::from_str(text).ok()?)
    }

    pub(super) fn to_stored(&self) -> anyhow::Result<String> {
        Ok(shape::to_value(self)?.to_string())
    }

    pub fn last_step(&self, key: &StepKey) -> Option<&LastStep> {
        self.last_steps.get(key)
    }

    /// Whether any step ever ran here: collections were configured before.
    pub fn ran_before(&self) -> bool {
        !self.last_steps.is_empty()
    }

    /// Whether the collection has snapshots that `target` may lack.
    pub fn pending(&self, collection: &str, target: &str) -> bool {
        self.pending.contains(&(collection.to_owned(), target.to_owned()))
    }

    pub fn succeeded(&self, key: &StepKey) -> bool {
        self.last_step(key).is_some_and(|l| l.last_ok.is_some())
    }

    pub fn lag(&self, collection: &str, target: &str) -> Option<&Lag> {
        self.lag.get(&(collection.to_owned(), target.to_owned()))
    }

    pub fn apply(&mut self, event: &Event) {
        match event.clone() {
            Event::Attempted { key, started, at, result, by } => {
                // Reading another subset does not clear the failure of a failed one.
                let failing =
                    matches!(&key, StepKey::Check(repo) if self.checks.get(repo).is_some_and(|c| !c.failed.is_empty()));
                let record = self.last_steps.entry(key).or_default();
                record.by = by;
                let succeeded = result.status.done();
                if succeeded && failing {
                    (record.last_attempt, record.note) = (Some(at), Some(result.record));
                } else {
                    if succeeded {
                        record.ok_started = Some(started);
                    }
                    record.record(at, &result);
                }
            }
            Event::Warned { key, warnings } => {
                self.last_steps.entry(key).or_default().warnings = warnings;
            }
            Event::Dismissed { key } => {
                // Its subsets are read again anyway.
                if let StepKey::Check(repo) = &key {
                    self.checks.entry(repo.clone()).or_default().failed.clear();
                }
                let record = self.last_steps.entry(key).or_default();
                (record.failures, record.last_error, record.cancelled) = (0, None, false);
                record.warnings.clear();
            }
            Event::FirstSeen { keys, at } => {
                for key in keys {
                    self.scheduled.entry(key).or_insert(at);
                }
            }
            Event::Unscheduled { key } => _ = self.scheduled.remove(&key),
            Event::Pending { collection, targets } => {
                self.pending.extend(targets.into_iter().map(|t| (collection.clone(), t)));
            }
            Event::Copied { collection, target } => _ = self.pending.remove(&(collection, target)),
            Event::Newest { repo, at } => _ = self.latest.insert(repo, at),
            Event::Lag { collection, target, at, behind } => {
                let pair = (collection, target);
                let behind_since = match behind {
                    Behind::Listed(since) => since,
                    Behind::Unlisted(since) => self.lag.get(&pair).and_then(|l| l.behind_since).or(since),
                };
                let listed = matches!(behind, Behind::Listed(_));
                self.lag.insert(pair, Lag { checked_at: at, behind_since, listed });
            }
            Event::CheckTaken { repo, of } => {
                let c = self.checks.entry(repo).or_default();
                // With fewer subsets, the ones beyond the new count no longer exist.
                c.owed.retain(|n| *n <= of);
                c.failed.retain(|n| *n <= of);
                c.taken = match c.owed.pop_first() {
                    Some(n) => n,
                    None => {
                        let taken = Some(c.next).filter(|n| (1..=of).contains(n)).unwrap_or(1);
                        c.next = taken % of + 1;
                        taken
                    }
                };
            }
            Event::CheckRead { repo, n, read } => {
                let c = self.checks.entry(repo).or_default();
                match read {
                    Read::Through => _ = (c.owed.remove(&n), c.failed.remove(&n)),
                    Read::Failed => _ = (c.owed.insert(n), c.failed.insert(n)),
                    Read::Not => _ = c.owed.insert(n),
                }
            }
            Event::Dropped { collection, target, removed, oldest_kept } => {
                let all = self.dropped.entry((collection, target)).or_default();
                all.extend(removed);
                all.retain(|(_, time)| oldest_kept.is_none_or(|kept| *time >= kept));
                all.sort();
                all.dedup();
            }
        }
    }
}

/// Serializes a map with tuple keys as a list of entries, because JSON keys must be strings.
mod entries {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S: Serializer, K: Serialize, V: Serialize>(
        map: &BTreeMap<K, V>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        s.collect_seq(map)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>, K: Deserialize<'de> + Ord, V: Deserialize<'de>>(
        d: D,
    ) -> Result<BTreeMap<K, V>, D::Error> {
        Vec::<(K, V)>::deserialize(d).map(BTreeMap::from_iter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    use crate::{result::StepResult, rustic::Reading};

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    /// A failed subset is read again before the next one, and the check fails until that subset passes,
    /// whatever other subsets do.
    #[test]
    fn failed_subset_is_checked_again_first() {
        let (repo, key, at) = ("Docs".to_owned(), StepKey::Check("Docs".into()), ts("2026-10-16T03:00:00Z"));
        let mut state = Ledger::default();
        let take = |state: &mut Ledger| {
            state.apply(&Event::CheckTaken { repo: repo.clone(), of: 10 });
            state.checks[&repo].taken
        };
        let (first, second) = (take(&mut state), take(&mut state));
        assert_eq!((first, second), (1, 2));
        let read = |n, read| Event::CheckRead { repo: repo.clone(), n, read };
        let verified = |n| StepResult::Verified { repo: "Docs".into(), reading: Reading::Subset { n, of: 10 } };
        let attempted = |result: StepResult| Event::Attempted {
            key: key.clone(),
            started: at,
            at,
            result: (&result).into(),
            by: None,
        };
        state.apply(&read(1, Read::Failed));
        state.apply(&attempted(StepResult::Error { error: "check 1/10 failed".into() }));
        state.apply(&read(2, Read::Through));
        state.apply(&attempted(verified(2)));
        assert_eq!(state.last_step(&key).unwrap().failures, 1, "subset 1 still fails");
        // Stopped before reading: owed, not failed.
        assert_eq!(take(&mut state), 1);
        assert_eq!(take(&mut state), 3);
        state.apply(&read(3, Read::Not));
        assert_eq!((take(&mut state), take(&mut state)), (3, 4));
        state.apply(&read(1, Read::Through));
        state.apply(&attempted(verified(1)));
        assert_eq!(state.last_step(&key).unwrap().failures, 0);
    }

    /// A backup, a failed copy, a successful copy, and a forget on the target.
    fn a_day() -> Vec<Event> {
        let (backup, copy) = (StepKey::Snapshot("Docs".into()), StepKey::Copy("Docs".into(), "usb".into()));
        let (collection, target) = ("Docs".to_owned(), "usb".to_owned());
        let id = |hex: &str| hex.repeat(8).parse::<SnapshotId>().unwrap();
        [
            Event::FirstSeen { keys: vec![backup.clone()], at: ts("2026-10-15T12:00:00Z") },
            Event::Attempted {
                key: backup.clone(),
                started: ts("2026-10-16T03:00:00Z"),
                at: ts("2026-10-16T03:10:00Z"),
                result: (&StepResult::BackedUp {
                    repo: "Docs".into(),
                    snapshots: vec!["22222222".into()],
                    new_repository: false,
                })
                    .into(),
                by: None,
            },
            Event::Warned {
                key: backup,
                warnings: vec![(&StepResult::Skipped { items: vec![("/a".into(), "denied".into())], count: 1 }).into()],
            },
            Event::Pending { collection: collection.clone(), targets: vec![target.clone()] },
            Event::Newest { repo: collection.clone(), at: ts("2026-10-16T03:00:00Z") },
            Event::Attempted {
                key: copy.clone(),
                started: ts("2026-10-16T03:10:00Z"),
                at: ts("2026-10-16T03:11:00Z"),
                result: (&StepResult::Failed {
                    step: "copy to usb".into(),
                    error: "unreachable".into(),
                    stopped: false,
                })
                    .into(),
                by: None,
            },
            // Not listed: behind since the known time, else since the newest snapshot.
            Event::Lag {
                collection: collection.clone(),
                target: target.clone(),
                at: ts("2026-10-16T03:11:00Z"),
                behind: Behind::Unlisted(Some(ts("2026-10-16T03:00:00Z"))),
            },
            Event::Attempted {
                key: copy.clone(),
                started: ts("2026-10-16T03:20:00Z"),
                at: ts("2026-10-16T03:21:00Z"),
                result: (&StepResult::Copied { target: "usb".into(), snapshots: 1 }).into(),
                by: None,
            },
            Event::Copied { collection: collection.clone(), target: target.clone() },
            Event::Lag {
                collection: collection.clone(),
                target: target.clone(),
                at: ts("2026-10-16T03:21:00Z"),
                behind: Behind::Listed(None),
            },
            Event::Dropped {
                collection,
                target,
                removed: vec![(id("1"), ts("2026-10-15T03:00:00Z")), (id("0"), ts("2026-10-01T03:00:00Z"))],
                oldest_kept: Some(ts("2026-10-10T03:00:00Z")),
            },
            Event::Dismissed { key: copy },
        ]
        .into()
    }

    fn a_day_of_events() -> Ledger {
        let mut state = Ledger::default();
        for event in &a_day() {
            state.apply(event);
        }
        state
    }

    #[test]
    fn ledger_folds_a_day_of_events() {
        insta::assert_debug_snapshot!(a_day_of_events());
    }

    /// Recorded once in `tests/fixtures/ledger.json`. A change that fails this needs a migration in
    /// [`Versioned::MIGRATIONS`]. Deleting the file records this version's ledger again.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "records and reads the fixture")]
    fn ledgers_stored_by_earlier_versions_are_still_read() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ledger.json");
        if !path.exists() {
            std::fs::write(&path, a_day_of_events().to_stored().unwrap()).unwrap();
        }
        let stored = std::fs::read_to_string(&path).unwrap();
        assert!(Ledger::from_stored(&stored).is_some_and(|l| l != Ledger::default()), "cannot read {stored}");
    }

    /// Recorded once in `tests/fixtures/events.json`: events read back, as ledgers do, so that another
    /// process can fold them. Deleting the file records this version's events again.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "records and reads the fixture")]
    fn events_stored_by_earlier_versions_are_still_read() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/events.json");
        if !path.exists() {
            let events: Vec<Value> = a_day().iter().map(|e| shape::to_value(e).unwrap()).collect();
            std::fs::write(&path, serde_json::to_string(&events).unwrap()).unwrap();
        }
        let stored: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for event in stored {
            assert!(shape::read::<Event>(event.clone()).is_some(), "cannot read {event}");
        }
    }
}
