//! Runs, their queue and their locks.

use super::*;

/// A request joins the same one queued, but not a copy to one target the copy to all, nor a check
/// another, which reads the next data subset.
#[test]
fn a_request_joins_the_same_one_queued() {
    let engine = engine("join", &Replay::new());
    let collections = engine.collections();
    let ask = |request: Request| engine.ask(&collections[0], &request, Trigger::Cli).unwrap();
    let first = ask(Request::Backup);
    assert!(matches!(ask(Request::Backup), Asked::Joined(r) if r.id == first.record().id));
    let to_usb = ask(Request::Copy("Docs-usb".into()));
    assert!(matches!(ask(Request::Copy("Docs".into())), Asked::Queued(_)));
    assert!(matches!(ask(Request::Copy("Docs-usb".into())), Asked::Joined(r) if r.id == to_usb.record().id));
    let checks = [ask(Request::Check(vec![("Docs".into(), None)])), ask(Request::Check(vec![("Docs".into(), None)]))];
    assert!(checks.iter().all(|c| matches!(c, Asked::Queued(_))));
}

/// A queued run waits for the running ones that hold its locks, and for the same request queued before
/// it. Not for other requests queued, even with a shared lock, nor for those queued after it.
#[test]
fn a_queued_run_waits_for_its_locks_and_the_same_request_before_it() {
    use crate::result::{Doing, Status};
    let engine = engine("queued-wait", &Replay::new());
    let collections = engine.collections();
    let ask = |request: &Request| match engine.ask(&collections[0], request, Trigger::Cli).unwrap() {
        Asked::Queued(queued) => queued,
        Asked::Joined(_) => panic!("joined"),
    };
    let waits_for = |own: &crate::state::RunLog| -> Vec<Option<Status>> {
        let Doing::WaitingFor { by, .. } =
            engine.queued_wait(&engine.store.queue(Some("Docs")), &own.record, &own.request)
        else {
            panic!("not waiting")
        };
        by.into_iter().map(|h| h.status).collect()
    };
    let check = Request::Check(vec![("Docs".into(), None)]);
    let mut prune = ask(&Request::Prune(vec!["Docs".into()]));
    let mut backup = ask(&Request::Backup);
    let (copy, first, second) = (ask(&Request::Copy("Docs".into())), ask(&check), ask(&check));
    assert!(waits_for(&copy).is_empty(), "a backup queued before, which also copies");
    assert!(waits_for(&first).is_empty(), "a prune queued before, and a check after");
    assert_eq!(waits_for(&second), [Some(Status::Queued)]);
    backup.start();
    prune.start();
    assert_eq!(waits_for(&copy), [Some(Status::Running)], "the backup, not the prune");
}

/// A scheduled run starts at once: it is neither queued nor joins the same one queued. A restore asked twice runs
/// twice.
#[test]
fn a_scheduled_run_starts_without_a_queue() {
    use crate::{request::RestoreOptions, result::Status};
    let engine = engine("ask-scheduled", &Replay::new());
    let collections = engine.collections();
    let ask = |request: Request, trigger| engine.ask(&collections[0], &request, trigger).unwrap();
    let queued = ask(Request::Backup, Trigger::Cli);
    let scheduled = ask(Request::Backup, Trigger::Schedule);
    assert!(
        matches!(&scheduled, Asked::Queued(run) if run.record.status == Status::Running && run.record.id != queued.record().id)
    );
    let restore = || ask(Request::Restore(RestoreOptions::default()), Trigger::Cli);
    let first = restore();
    assert!(matches!(restore(), Asked::Queued(run) if run.record.id != first.record().id));
}

/// A result that steps running at once report alike is kept once. Failures are counted. A step that does nothing
/// more shows no activity.
#[test]
fn a_run_keeps_each_result_once_and_counts_failures() {
    use crate::{live::Guard, result::Doing, state::RunLog};
    let engine = engine("run-results", &Replay::new());
    let collections = engine.collections();
    let runs = Runs::new(|_| {});
    let guard = Guard::start(&runs, RunLog::new(&engine.store, "Docs", Request::Backup, Trigger::Cli));
    let run = crate::engine::Run::new(&collections[0], Some(guard.live.clone()));
    run.push_once(failed());
    run.push_once(failed());
    run.push_once(backed_up());
    assert_eq!(run.results_of(None), [failed(), backed_up()]);
    run.push(failed());
    assert_eq!(failures(&run), 2);
    run.activity(Some(Doing::WaitingForListing { repo: "Docs".into() }), false);
    assert_eq!(guard.live.activities().len(), 1);
    run.idle();
    assert!(guard.live.activities().is_empty());
}

/// What a step holds shows while it holds it, and no longer once it lets go.
#[test]
fn a_step_shows_what_it_holds_until_it_lets_go() {
    use crate::{live::Guard, result::Status, state::RunLog};
    let engine = engine("held", &Replay::new());
    let collections = engine.collections();
    let runs = Runs::new(|_| {});
    let guard = Guard::start(&runs, RunLog::new(&engine.store, "Docs", Request::Backup, Trigger::Cli));
    guard
        .live
        .steps(&[StepRecord { status: Status::Running, ..StepRecord::queued("snapshot to Docs".into(), Vec::new()) }]);
    let run = crate::engine::Run::new(&collections[0], Some(guard.live.clone()));
    let step = run.for_step(0);
    let using = || guard.live.record().steps[0].holds.using.clone();
    let held = engine.lock(&step, false, &[(collections[0].primary, crate::engine::holds::Use::Read)]).unwrap();
    assert_eq!(using(), ["download", "repo:Docs"]);
    drop(held);
    assert_eq!(using(), Vec::<String>::new());
}

/// A lock that fails is an error. Only a queued run that was removed from the queue meanwhile is dequeued.
#[test]
fn a_lock_that_fails_is_an_error() {
    use crate::{
        result::{RunResults, StepResult},
        rustic::Ctx,
    };
    let engine = engine("lock-fails", &Replay::new());
    let collections = engine.collections();
    // A file where the locks go.
    let locks = engine.store.dir.join("locks");
    std::fs::remove_dir_all(&locks).unwrap();
    std::fs::write(&locks, "").unwrap();
    let (cx, mut r) = (Ctx::default(), RunResults::new("Docs"));
    assert!(engine.lock_for(&collections[0], &Request::Backup, &cx, &mut r, None).is_none());
    assert!(matches!(r.results[..], [StepResult::Error { .. }]), "{:?}", r.results);
}
