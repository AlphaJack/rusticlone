//! When steps are due, and when the agenda wakes.

use super::*;

/// A backup never made is due now, so nothing is due next. After one, the schedule's next time is.
#[test]
fn the_next_run_is_after_now() {
    use crate::request::StepKey;
    let engine = engine_with("next-due", &Replay::new(), &docs_with(&[(&crate::settings::BACKUP, "@daily")]));
    let collection = &engine.collections()[0];
    assert_eq!(engine.next_due(collection), None, "due now");
    attempted(&engine, StepKey::Snapshot("Docs".into()), &backed_up(), false);
    assert_eq!(engine.next_due(collection), Some((Request::Backup, at("2026-10-17T00:00:00Z"))));
}

/// Maintenance never run is due a schedule after the schedule was first seen, even if no process was
/// running at that time.
#[test]
fn maintenance_is_due_after_its_schedule_was_first_seen() {
    let engine = engine_with("first-seen", &Replay::new(), &docs_with(&[(&crate::settings::CHECK, "@daily")]));
    let collection = &engine.collections()[0];
    assert_eq!(engine.next_due(collection).map(|(_, at)| at), Some(at("2026-10-17T00:00:00Z")));
    engine.store.wait(std::time::Duration::from_hours(13), &|| false);
    assert_eq!(engine.due_next(collection, &[]).unwrap(), Some(Request::Check(vec![("Docs".into(), None)])));
}

/// With a backup schedule, a repository is stale once the schedule fired twice since its last backup.
/// Without one, after a week.
#[test]
fn a_repository_is_stale_after_two_scheduled_backups_or_a_week() {
    use crate::request::StepKey;
    let stale_at = |test: &str, docs: &str| {
        let engine = engine_with(test, &Replay::new(), docs);
        attempted(&engine, StepKey::Snapshot("Docs".into()), &backed_up(), false);
        let status = engine.status_of(&engine.collections()[0]);
        let (_, changes) = crate::clock::next_change(|| engine.status_of(&engine.collections()[0]));
        (status.primary.stale_at, changes)
    };
    let daily = docs_with(&[(&crate::settings::BACKUP, "@daily")]);
    assert_eq!(stale_at("stale-scheduled", &daily).0, Some(at("2026-10-18T00:00:00Z")));
    let week = at("2026-10-23T12:00:00Z");
    assert_eq!(stale_at("stale-unscheduled", DOCS), (Some(week), Some(week)), "and the status changes then");
}

/// A scheduled first failure is reported once its retry is late, and the agenda wakes then. Not after.
#[cfg(feature = "server")]
#[test]
fn the_agenda_wakes_when_a_pending_failure_is_reported() {
    use crate::request::StepKey;
    let engine = engine("agenda-pending", &Replay::new());
    let key = StepKey::Check("Docs".into());
    attempted(&engine, key.clone(), &failed(), true);
    let until = engine.store.load().last_step(&key).unwrap().pending_until().unwrap();
    let (_, changes) = crate::clock::next_change(|| engine.status_of(&engine.collections()[0]));
    assert_eq!(changes, Some(until), "the status changes then");
    assert_eq!(engine.agenda(engine.store.now()).1, Some(until));
    assert_eq!(engine.agenda(until).1, None);
    engine.store.wait(until.duration_since(engine.store.now()).unsigned_abs(), &|| false);
    let (_, changes) = crate::clock::next_change(|| engine.status_of(&engine.collections()[0]));
    assert_eq!(changes, None, "reported: nothing changes any more");
}

/// The agenda wakes when a repository becomes stale, and not after.
#[cfg(feature = "server")]
#[test]
fn the_agenda_wakes_when_a_repository_becomes_stale() {
    use crate::request::StepKey;
    let engine = engine("agenda-stale", &Replay::new());
    attempted(&engine, StepKey::Snapshot("Docs".into()), &backed_up(), false);
    let week = at("2026-10-23T12:00:00Z");
    assert_eq!(engine.agenda(engine.store.now()).1, Some(week));
    assert_eq!(engine.agenda(week).1, None);
    engine.store.wait(week.duration_since(engine.store.now()).unsigned_abs(), &|| false);
    let (_, changes) = crate::clock::next_change(|| engine.status_of(&engine.collections()[0]));
    assert_eq!(changes, None, "stale: nothing changes any more");
}

/// What happens next is after now: a collection that turns stale right now is not next, it is stale already.
#[cfg(feature = "server")]
#[test]
fn the_next_time_is_after_now() {
    use crate::{request::StepKey, state::Event};
    let engine = engine("agenda-after-now", &Replay::new());
    let now = engine.store.now();
    // Backed up a week ago: stale now, without a schedule.
    let started = now - jiff::SignedDuration::from_hours(7 * 24);
    let key = StepKey::Snapshot("Docs".into());
    engine
        .store
        .apply(&[Event::Attempted { key, started, at: started, result: (&backed_up()).into(), by: None }])
        .unwrap();
    let (_, next) = engine.agenda(now);
    assert_eq!(next, None);
}

/// A repository recorded as missing has nothing to maintain until found again, and a local one only once it was
/// created. Neither looks at the disk.
#[test]
fn maintenance_waits_for_a_repository_that_exists_as_the_state_says() {
    use crate::{request::StepKey, state::Event};
    let local =
        docs_with(&[(&crate::settings::CHECK, "@daily")]).replace("rest:http://backup/docs", "/nonexistent/docs");
    let engine = engine_with("maintain-existing", &Replay::new(), &local);
    let collections = engine.collections();
    let collection = &collections[0];
    assert!(engine.next_due(collection).is_some(), "first seen now");
    engine.store.wait(std::time::Duration::from_hours(36), &|| false);
    let check = Request::Check(vec![("Docs".into(), None)]);
    let due = |engine: &Engine| engine.due_next(collection, &[]).unwrap() == Some(check.clone());
    assert!(!due(&engine), "a local repository never created");
    attempted(&engine, StepKey::Snapshot("Docs".into()), &backed_up(), false);
    assert!(engine.next_due(collection).is_some(), "its schedule first seen once created");
    engine.store.wait(std::time::Duration::from_hours(36), &|| false);
    assert!(due(&engine), "created");
    engine.store.apply(&[Event::Missing { repo: "Docs".into(), missing: true }]).unwrap();
    assert!(!due(&engine), "missing");
    engine.store.apply(&[Event::Missing { repo: "Docs".into(), missing: false }]).unwrap();
    assert!(due(&engine), "found again");
}
