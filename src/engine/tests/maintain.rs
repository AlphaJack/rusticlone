//! Checks, and the subsets they owe.

use super::*;

/// Checks cycle through as many data subsets as set. Zero reads the structure only, and a sample takes none.
#[test]
fn checks_cycle_through_the_subsets_set() {
    use crate::settings::CHECK_SUBSETS;
    let subsets = |value: &str| {
        crate::engine::maintain::subsets(&[&profile::parse("Docs", &docs_with(&[(&CHECK_SUBSETS, value)]))])
    };
    assert_eq!([subsets("7"), subsets("1"), subsets("0"), subsets("5%")], [Some(7), Some(1), None, None]);
}

/// A check that will not run owes the data subset it took: the next check reads it again.
#[test]
fn a_check_that_does_not_run_owes_its_subset() {
    use crate::{rustic::Reading, settings::CHECK_SUBSETS};
    let engine = engine_with("owed-subset", &Replay::new(), &docs_with(&[(&CHECK_SUBSETS, "3")]));
    let collections = engine.collections();
    let check = Request::Check(vec![("Docs".into(), None)]);
    let claimed = |engine: &Engine| {
        let Request::Check(repos) = engine.claim(&collections[0], &check).unwrap() else {
            panic!("a check claims a check")
        };
        repos[0].1
    };
    let first = claimed(&engine);
    assert_eq!(first, Some(Reading::Subset { n: 1, of: 3 }));
    engine.unread(&collections[0], &Request::Check(vec![("Docs".into(), first)]));
    assert_eq!(claimed(&engine), first);
    assert_eq!(claimed(&engine), Some(Reading::Subset { n: 2, of: 3 }));
}

/// A check that fails owes its subset: the next check reads it again. Only one that fails on its own, not as its run
/// stops, counts as failed, until that subset passes.
#[cfg(feature = "server")]
#[test]
fn a_check_stopped_owes_its_subset() {
    use crate::{live::Guard, request::Step, rustic::Reading, state::RunLog};
    for stopped in [true, false] {
        let engine = engine(&format!("check-stopped-{stopped}"), &Replay::new());
        let collections = engine.collections();
        let runs = Runs::new(|_| {});
        let guard = Guard::start(&runs, RunLog::new(&engine.store, "Docs", Request::Backup, Trigger::Cli));
        if stopped {
            guard.live.cancel();
        }
        let run = crate::engine::Run::new(&collections[0], Some(guard.live.clone()));
        let check = Step::Check { repo: "Docs".into(), reading: Reading::Subset { n: 2, of: 5 } };
        engine.maintain(&run, collections[0].primary, &check);
        let checks = engine.store.load().checks["Docs"].clone();
        assert_eq!((checks.owed.contains(&2), checks.failed.contains(&2)), (true, !stopped), "stopped: {stopped}");
    }
}

/// A check that never started, because its run stopped first, owes its subset.
#[test]
fn a_check_never_started_owes_its_subset() {
    use crate::{request::Step, rustic::Reading};
    let engine = engine("check-never-started", &Replay::new());
    let collections = engine.collections();
    let run = crate::engine::Run::new(&collections[0], None);
    let check = Step::Check { repo: "Docs".into(), reading: Reading::Subset { n: 3, of: 5 } };
    engine.cancelled(&run, 0, &check, engine.store.now());
    let checks = engine.store.load().checks["Docs"].clone();
    assert_eq!((checks.owed.contains(&3), checks.failed.contains(&3)), (true, false));
}
