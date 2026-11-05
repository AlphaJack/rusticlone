//! Notifications: when they are sent, and what they say.

use super::*;

/// A notification that cannot be sent is attempted again once its backoff has passed, not before.
#[test]
fn a_failed_notification_is_sent_again_after_its_backoff() {
    use crate::{request::StepKey, result::StepResult, state::Event};
    let docs = format!("{DOCS}[global.env]\n{} = \"json://127.0.0.1:1\"\n", crate::settings::NOTIFY.key);
    let engine = engine_with("notify-backoff", &Replay::new(), &docs);
    let at = engine.store.now();
    let result = StepResult::Error { error: "repository unreachable".into() };
    let failed =
        Event::Attempted { key: StepKey::Snapshot("Docs".into()), started: at, at, result: (&result).into(), by: None };
    engine.store.apply(&[failed]).unwrap();
    let collection = &engine.collections()[0];
    let retry = engine.notify(collection).unwrap();
    assert_eq!(engine.store.notified("Docs").unwrap().failures, 1);
    assert_eq!(engine.notify(collection), Some(retry), "not before its backoff");
    assert_eq!(engine.store.notified("Docs").unwrap().failures, 1);
    engine.store.wait(retry.duration_since(at).unsigned_abs(), &|| false);
    assert!(engine.notify(collection) > Some(retry), "sent again, and failed again");
    assert_eq!(engine.store.notified("Docs").unwrap().failures, 2);
}

/// A recovery says how long the level notified before it lasted, from when that level was delivered, and
/// how many other collections notified at the same URLs are not healthy.
#[test]
fn a_recovery_notification_says_how_long_the_previous_level_lasted() {
    use crate::{request::StepKey, result::StepResult, state::Event, state::Notified};
    let elsewhere = "[repository]\nrepository = \"rest:http://backup/music\"\n[[backup.snapshots]]\nsources = [\"/music\"]\n[global.env]\nRUSTICLONE_NOTIFY_URLS = \"ntfy://ntfy.sh/music\"\n";
    let photos =
        "[repository]\nrepository = \"rest:http://backup/photos\"\n[[backup.snapshots]]\nsources = [\"/photos\"]\n";
    let pics = photos.replace("photos", "pics");
    let profiles = [("Docs", DOCS), ("Docs-usb", USB), ("Music", elsewhere), ("Photos", photos), ("Pics", &pics)];
    let engine = engine_of("notify-recovery", &Replay::new(), &profiles);
    let (key, at) = (StepKey::Snapshot("Docs".into()), engine.store.now());
    let result = StepResult::Error { error: "repository unreachable".into() };
    let failed = |name: &str| Event::Attempted {
        key: StepKey::Snapshot(name.into()),
        started: at,
        at,
        result: (&result).into(),
        by: None,
    };
    engine.store.apply(&["Docs", "Music", "Photos", "Pics"].map(failed)).unwrap();
    let failing = Notified { level: Level::Failure, since: Some(at), ..Notified::default() };
    engine.store.update_notified("Docs", |_| failing.clone()).unwrap();
    let collection = &engine.collections()[0];
    let host = crate::sys::hostname().unwrap();
    let text = engine.message(collection, &engine.status_of(collection), &failing);
    assert_eq!(text, format!("{host}: Docs is now failing\n🔴 Repository unreachable\n"), "no age unless recovered");
    engine.store.wait(std::time::Duration::from_hours(51), &|| false);
    let (at, result) =
        (engine.store.now(), StepResult::BackedUp { repo: "Docs".into(), snapshots: vec![], new_repository: false });
    engine.store.apply(&[Event::Attempted { key, started: at, at, result: (&result).into(), by: None }]).unwrap();
    let text = engine.message(collection, &engine.status_of(collection), &failing);
    // Those notified at the same URLs: not Music.
    let others = "🔴 2 other collections failing";
    assert_eq!(text, format!("{host}: Docs is now healthy\n🟢 Was failing for 2 days\n{others}\n"));
    // Healthy before too: nothing lasted.
    let healthy = Notified { level: Level::Success, since: Some(at), ..Notified::default() };
    assert_eq!(
        engine.message(collection, &engine.status_of(collection), &healthy),
        format!("{host}: Docs is now healthy\n")
    );
}

/// A level already notified is not sent again. Sends at the same level keep when it started.
#[test]
fn a_level_is_notified_once_from_when_it_started() {
    use std::io::{Read, Write};

    use crate::state::Notified;
    let receiver = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("json://{}", receiver.local_addr().unwrap());
    let answering = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut socket, _) = receiver.accept().unwrap();
            let mut request = [0; 4096];
            _ = socket.read(&mut request).unwrap();
            write!(socket, "HTTP/1.1 204 Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        }
    });
    let docs = format!("{DOCS}[global.env]\n{} = \"{url}\"\n", crate::settings::NOTIFY.key);
    let engine = engine_with("notify-once", &Replay::new(), &docs);
    let collection = &engine.collections()[0];
    // Notified at this level, with a failure left from an earlier one: nothing is sent, which would fail.
    let refused = format!("{DOCS}[global.env]\n{} = \"json://127.0.0.1:1\"\n", crate::settings::NOTIFY.key);
    let unsent = engine_with("notify-unsent", &Replay::new(), &refused);
    let level = unsent.status_of(&unsent.collections()[0]).level;
    let left = Notified { level, failures: 2, error: Some("refused".into()), ..Notified::default() };
    unsent.store.update_notified("Docs", |_| left.clone()).unwrap();
    unsent.notify(&unsent.collections()[0]);
    let notified = unsent.store.notified("Docs").unwrap();
    assert_eq!((notified.level, notified.failures, notified.error), (level, 0, None));
    engine.notify_now(collection, &mut |_| {}).unwrap();
    let first = engine.store.notified("Docs").unwrap().since;
    engine.store.wait(std::time::Duration::from_hours(1), &|| false);
    engine.notify_now(collection, &mut |_| {}).unwrap();
    assert_eq!(engine.store.notified("Docs").unwrap().since, first);
    answering.join().unwrap();
}

/// A notification error shows only while sends fail.
#[test]
fn an_undelivered_notification_shows_while_it_fails() {
    let engine =
        engine_with("undelivered", &Replay::new(), &docs_with(&[(&crate::settings::NOTIFY, "json://127.0.0.1:1")]));
    let collection = &engine.collections()[0];
    let undelivered = |failures| {
        let notified = crate::state::Notified { failures, error: Some("refused".into()), ..Default::default() };
        engine.store.update_notified("Docs", |_| notified.clone()).unwrap();
        engine
            .status_of(collection)
            .issues
            .iter()
            .any(|i| matches!(i, crate::engine::report::Issue::Undelivered { .. }))
    };
    assert!(!undelivered(0));
    assert!(undelivered(1));
}
