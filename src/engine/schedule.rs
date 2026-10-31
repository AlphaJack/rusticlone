//! What is due and when, from the collections' schedules and the state only. The schedule changes the
//! state must store are returned to the caller.

use anyhow::Result;
use jiff::{SignedDuration, Timestamp, tz::TimeZone};
use serde::Serialize;

use super::{Collection, Engine, behind};
use crate::request::{Request, RestoreOptions, RunKind, StepKey};
use crate::{
    profile::RusticProfile,
    settings,
    state::{Event, Ledger},
};

/// A schedule, as the status shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Schedule {
    pub cron: String,
    /// `None` for an invalid expression.
    pub next: Option<Timestamp>,
    /// Why the expression is invalid. Nothing runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A time the collection was backed up at least, and its last backup. Backups made by rustic directly count
/// once seen in the repository. But where unchanged sources make no snapshot, the newest snapshot only
/// dates the last change, and only the recorded runs date the last backup. Both are the time
/// the backup started, which is what the snapshot contains.
pub(super) fn backups(p: &RusticProfile, state: &Ledger) -> (Option<Timestamp>, Option<Timestamp>) {
    let recorded = state.last_step(&StepKey::Snapshot(p.name.clone())).and_then(|t| t.ok_started);
    let backed_up = recorded.max(state.latest.get(&p.name).copied());
    (backed_up, if p.keeps_unchanged() { backed_up } else { recorded })
}

/// A collection's runs in order, each with its due time if scheduled, and the schedules first seen now
/// or removed, for the state.
#[derive(Debug, Default, Serialize)]
pub(super) struct Due {
    pub requests: Vec<(Request, Option<Timestamp>)>,
    pub first_seen: Vec<StepKey>,
    pub unscheduled: Vec<StepKey>,
}

/// The restore of a collection from an emergency kit: every source, in place, only where nothing is.
pub(crate) fn kit_restore() -> RestoreOptions {
    RestoreOptions { missing: true, ..RestoreOptions::default() }
}

/// In order: a backup (with its copies), copies a target lacks, then prune and check of each
/// repository.
pub(super) fn due(collection: &Collection<'_>, state: &Ledger, now: Timestamp, tz: &TimeZone) -> Due {
    let mut due = Due::default();
    let p = collection.primary;
    // From an emergency kit: restored once, without overwriting anything, instead of backed up, unless the web UI
    // asked for that restore. Nothing backs it up or prunes it until a restore succeeds or someone backs it up.
    let held = state.restore_first.get(&p.name);
    if let Some(hold) = held {
        due.requests.push((Request::Restore(kit_restore()), hold.restore_due().then_some(now)));
    } else {
        let backup = due.at(&[p], RunKind::Backup, StepKey::Snapshot(p.name.clone()), state, now, tz);
        due.requests.push((Request::Backup, backup));
    }
    // Snapshots pending a copy, and those a target is known to lack (made by rustic directly), are copied
    // at once, or with backoff after a failure. Not to a target that cannot be listed: if it is
    // unmounted, the copy would create it in the mount point.
    let copy = collection.targets.iter().filter_map(|t| {
        let key = state.last_step(&StepKey::Copy(p.name.clone(), t.name.clone())).cloned().unwrap_or_default();
        let known = state.lag(&p.name, &t.name).is_some_and(|l| l.listed && l.behind_since.is_some());
        let lacking = state.pending(&p.name, &t.name) || known;
        if key.failures > 0 { key.retry_at() } else { (lacking && !key.cancelled).then_some(now) }
    });
    due.requests.push((Request::Copy(p.name.clone()), copy.min()));
    // Nothing to maintain until it exists.
    for repo in collection.repos().filter(|r| r.exists()) {
        let name = || repo.name.clone();
        for (request, key) in [
            (Request::Prune(vec![name()]), StepKey::Prune(name())),
            (Request::Check(vec![(name(), None)]), StepKey::Check(name())),
        ] {
            // The prune waits for the copy that brings the target up to date, and for the restore.
            if matches!(request, Request::Prune(_)) && (held.is_some() || behind(state, collection, repo)) {
                continue;
            }
            let at = due.at(&[repo, p], request.kind(), key, state, now, tz);
            due.requests.push((request, at));
        }
    }
    due
}

impl Due {
    /// The first scheduled time after the last success or cancellation, or the retry time after a
    /// failure. `None` if not scheduled.
    fn at(
        &mut self,
        chain: &[&RusticProfile],
        kind: RunKind,
        key: StepKey,
        state: &Ledger,
        now: Timestamp,
        tz: &TimeZone,
    ) -> Option<Timestamp> {
        let cron = match setting(chain, kind)?.schedule(chain) {
            Ok(Some(cron)) => cron,
            // The status reports the invalid schedule.
            Err(_) => return None,
            Ok(None) => {
                // If the schedule is set again later, it starts from scratch.
                if state.scheduled.contains_key(&key) {
                    self.unscheduled.push(key);
                }
                return None;
            }
        };
        let last = state.last_step(&key).cloned().unwrap_or_default();
        if last.failures > 0 {
            return last.retry_at();
        }
        let done = match &key {
            // A snapshot has the sources as of its start. A snapshot made since then, for example by rustic
            // itself, counts.
            StepKey::Snapshot(collection) => last.ok_started.max(state.latest.get(collection).copied()),
            // A check or prune that was running at the scheduled time did the work.
            StepKey::Copy(..) | StepKey::Forget(..) | StepKey::Prune(_) | StepKey::Check(_) => last.last_ok,
        };
        // Never done: a backup is due at once, because nothing has the sources yet. Maintenance is due
        // after its schedule was first seen, like a persistent systemd timer.
        let since = match if last.cancelled { last.last_attempt } else { done } {
            Some(last) => last,
            None if matches!(key, StepKey::Snapshot(_)) => return Some(now),
            None => state.scheduled.get(&key).copied().unwrap_or_else(|| {
                self.first_seen.push(key);
                now
            }),
        };
        cron.next_after(since, tz)
    }
}

/// `None` when not scheduled.
pub(super) fn of(chain: &[&RusticProfile], key: RunKind, now: Timestamp, tz: &TimeZone) -> Option<Schedule> {
    let setting = setting(chain, key)?;
    let (next, error) = match setting.schedule(chain) {
        Ok(None) => return None,
        Ok(Some(cron)) => (cron.next_after(now, tz), None),
        Err(e) => (None, Some(format!("{e:#}"))),
    };
    Some(Schedule { cron: setting.get(chain)?, next, error })
}

/// The setting that schedules `kind` on `chain`'s repository. One without a `[forget]` policy keeps every
/// snapshot, so it is never pruned.
fn setting(chain: &[&RusticProfile], kind: RunKind) -> Option<&'static settings::Setting> {
    kind.schedule().filter(|_| kind != RunKind::Prune || chain.first().is_some_and(|r| r.has_keep))
}

/// How long something can stay out of sync without a backup schedule.
const STALE_UNSCHEDULED: SignedDuration = SignedDuration::from_hours(7 * 24);

/// When something in sync since `since` becomes stale: when the backup schedule has fired twice (a
/// whole run was missed), or after a week without a schedule.
fn stale_after(collection: &RusticProfile, since: Timestamp, tz: &TimeZone) -> Option<Timestamp> {
    match settings::BACKUP.schedule(&[collection]) {
        Ok(Some(cron)) => cron.next_after(since, tz).and_then(|first| cron.next_after(first, tz)),
        _ => since.checked_add(STALE_UNSCHEDULED).ok(),
    }
}

impl Engine {
    /// When `r` is or will be stale. A target is as fresh as the primary, or as its oldest missing
    /// snapshot.
    pub(super) fn stale_at(
        &self,
        p: &RusticProfile,
        r: &RusticProfile,
        state: &Ledger,
        last_backup: Option<Timestamp>,
    ) -> Option<Timestamp> {
        let since = match r.name == p.name {
            true => last_backup,
            false => state.lag(&p.name, &r.name)?.behind_since.or(last_backup),
        };
        stale_after(p, since?, &self.tz)
    }

    /// [`due`], storing the schedule changes it returns.
    fn due_requests(
        &self,
        collection: &Collection<'_>,
        state: &Ledger,
        now: Timestamp,
    ) -> Result<Vec<(Request, Option<Timestamp>)>> {
        let due = due(collection, state, now, &self.tz);
        let mut events: Vec<Event> = due.unscheduled.into_iter().map(|key| Event::Unscheduled { key }).collect();
        if !due.first_seen.is_empty() {
            events.push(Event::FirstSeen { keys: due.first_seen, at: now });
        }
        self.store.apply(&events)?;
        Ok(due.requests)
    }

    /// The requests due now, and the next time something is due: a run, or a collection becoming stale or a failure
    /// being reported, which is notified.
    #[cfg(feature = "server")]
    pub(crate) fn agenda(&self, now: Timestamp) -> (Vec<(String, Request)>, Option<Timestamp>) {
        let state = self.store.load();
        // A failure is reported once its pending ends, unless a retry made it pass.
        let pending = state.last_steps.values().filter_map(crate::state::LastStep::pending_until);
        let (mut due, mut next) = (Vec::new(), pending.filter(|at| *at > now).min());
        for collection in self.collections() {
            let (_, last_backup) = backups(collection.primary, &state);
            let stale = collection.repos().filter_map(|r| self.stale_at(collection.primary, r, &state, last_backup));
            next = next.into_iter().chain(stale).filter(|at| *at > now).min();
            let requests = self.due_requests(&collection, &state, now).unwrap_or_else(|e| {
                crate::log::event(
                    Some(collection.name()),
                    &crate::log::Logged::NotScheduled { error: format!("{e:#}") },
                );
                Vec::new()
            });
            for (request, at) in requests {
                match at {
                    Some(at) if at <= now => due.push((collection.name().to_owned(), request)),
                    at => next = next.into_iter().chain(at).min(),
                }
            }
        }
        (due, next)
    }

    /// The request due next after now, and when: what `due` says when nothing is due yet.
    pub(crate) fn next_due(&self, collection: &Collection<'_>) -> Option<(Request, Timestamp)> {
        let now = self.store.now();
        let requests = self.due_requests(collection, &self.store.load(), now).ok()?;
        requests.into_iter().filter_map(|(r, at)| Some((r, at.filter(|at| *at > now)?))).min_by_key(|(_, at)| *at)
    }

    /// The next due request that this round has not run yet, in [`due`] order.
    pub(super) fn due_next(&self, collection: &Collection<'_>, done: &[Request]) -> Result<Option<Request>> {
        let now = self.store.now();
        let requests = self.due_requests(collection, &self.store.load(), now)?;
        Ok(requests.into_iter().find(|(r, at)| at.is_some_and(|at| at <= now) && !done.contains(r)).map(|(r, _)| r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        profile,
        state::{Hold, Lag, LastStep},
    };

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    /// A `[forget]` policy, without which a repository is never pruned.
    const KEEP: &str = "[forget]\nkeep-last = 3\n";

    /// `Docs`, copied to `Docs-usb`, its schedules in `env`, both forgetting.
    fn collections(env: &str) -> [RusticProfile; 2] {
        let docs = format!(
            "[global.env]\n{env}\n[repository]\nrepository = \"rest:http://backup/docs\"\n\
             [[backup.snapshots]]\nsources = [\"-\"]\nas-path = \"/docs\"\n[copy]\ntargets = [\"Docs-usb\"]\n{KEEP}"
        );
        [
            profile::parse("Docs", &docs),
            profile::parse("Docs-usb", &format!("[repository]\nrepository = \"rest:http://usb/docs\"\n{KEEP}")),
        ]
    }

    /// Each case: the schedules, the state; what is due at noon.
    #[test]
    fn due_by_example() {
        let backup = StepKey::Snapshot("Docs".into());
        let daily = &format!(r#"{} = "0 3 * * *""#, settings::BACKUP.key);
        let maintained = &format!(
            r#"{} = "0 4 * * *"
{} = "0 5 * * *""#,
            settings::PRUNE.key,
            settings::CHECK.key
        );
        let usb = ("Docs".to_owned(), "Docs-usb".to_owned());
        let failed = LastStep { failures: 2, last_attempt: Some(ts("2026-10-16T11:00:00Z")), ..LastStep::default() };
        let behind = Lag {
            checked_at: ts("2026-10-16T11:00:00Z"),
            behind_since: Some(ts("2026-10-16T09:00:00Z")),
            listed: true,
        };
        let cases: [(&str, &str, Ledger); 10] = [
            // Never backed up: due now; never checked nor pruned: at their next time.
            ("never_run", daily, Ledger::default()),
            ("never_maintained", maintained, Ledger::default()),
            (
                "check_seen_yesterday",
                maintained,
                Ledger {
                    scheduled: [(StepKey::Check("Docs".into()), ts("2026-10-15T12:00:00Z"))].into(),
                    ..Ledger::default()
                },
            ),
            (
                "backed_up_this_morning",
                daily,
                Ledger { latest: [("Docs".into(), ts("2026-10-16T03:00:00Z"))].into(), ..Ledger::default() },
            ),
            ("failing", daily, Ledger { last_steps: [(backup.clone(), failed)].into(), ..Ledger::default() }),
            (
                "schedule_removed",
                "",
                Ledger { scheduled: [(backup, ts("2026-10-15T12:00:00Z"))].into(), ..Ledger::default() },
            ),
            ("target_lacks_snapshots", "", Ledger { pending: [usb.clone()].into(), ..Ledger::default() }),
            // The target's prune waits for its copy.
            ("target_behind", maintained, Ledger { lag: [(usb, behind)].into(), ..Ledger::default() }),
            // From a kit: restored instead of backed up, and not pruned; once that failed, nothing until asked.
            (
                "restore_first",
                maintained,
                Ledger { restore_first: [("Docs".into(), Hold::default())].into(), ..Ledger::default() },
            ),
            (
                "restore_first_failed",
                maintained,
                Ledger {
                    restore_first: [("Docs".into(), Hold { asked: None, failed: Some(ts("2026-10-16T11:00:00Z")) })]
                        .into(),
                    ..Ledger::default()
                },
            ),
        ];
        for (name, env, state) in cases {
            let [docs, usb] = collections(env);
            let collection =
                Collection { primary: &docs, title: "Docs", targets: vec![&usb], missing_targets: Vec::new() };
            let due = due(&collection, &state, ts("2026-10-16T12:00:00Z"), &TimeZone::UTC);
            insta::assert_json_snapshot!(name, due);
        }
    }

    #[test]
    fn schedules_are_classic_cron() {
        let ts = |s: &str| s.parse::<Timestamp>().unwrap();
        let next =
            |expr: &str, after: &str, tz: &TimeZone| settings::cron(expr).unwrap().next_after(ts(after), tz).unwrap();
        let utc = TimeZone::UTC;
        assert_eq!(next("*/15 * * * *", "2026-01-01T00:00:00Z", &utc), ts("2026-01-01T00:15:00Z"));
        assert_eq!(next("0 3 * * *", "2026-01-01T03:00:00Z", &utc), ts("2026-01-02T03:00:00Z"));
        // 2026-10-18 is a Sunday.
        for weekly in ["@weekly", "0 0 * * sun", "0 0 * * 7"] {
            assert_eq!(next(weekly, "2026-10-14T12:00:00Z", &utc), ts("2026-10-18T00:00:00Z"));
        }
        // Day of month or day of week, when both are restricted.
        assert_eq!(next("0 0 13 * fri", "2026-10-14T12:00:00Z", &utc), ts("2026-10-16T00:00:00Z"));
        // 02:30 doesn't exist in Berlin that night: once, as the clocks jump to 03:00.
        let berlin = TimeZone::get("Europe/Berlin").unwrap();
        assert_eq!(next("30 2 * * *", "2026-03-28T23:00:00Z", &berlin), ts("2026-03-29T01:00:00Z"));
        // 02:30 comes twice there on 2026-10-25: the first time only.
        assert_eq!(next("30 2 * * *", "2026-10-25T00:30:00Z", &berlin), ts("2026-10-26T01:30:00Z"));
        assert_eq!(next("5/20 * * * *", "2026-01-01T00:30:00Z", &utc), ts("2026-01-01T00:45:00Z"));
        assert!(
            ["never", "", "* * * *", "0 0 * * * *", "60 * * * *", "x * * * *"]
                .iter()
                .all(|e| settings::cron(e).is_err())
        );
    }
}
