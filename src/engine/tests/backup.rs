//! Backups: creating the primary, holding back, and learning where hooked sources are.

use super::*;

/// A primary that cannot be listed is not created again once a backup into it succeeded, even before its newest
/// snapshot was ever recorded.
#[test]
fn a_primary_backed_up_before_is_not_created_again() {
    use crate::request::StepKey;
    let replay = Replay::new();
    let engine = engine("backed-up-before", &replay);
    attempted(&engine, StepKey::Snapshot("Docs".into()), &backed_up(), false);
    let collections = engine.collections();
    asked(&engine, &collections[0], &Request::Backup);
    let calls = replay.calls();
    assert!(!calls.iter().any(|c| c.starts_with("-P Docs backup")), "{calls:?}");
}

/// A backup is of the whole collection: one of a single repository is refused.
#[test]
fn a_backup_of_one_repository_is_refused() {
    use crate::request::{RestoreOptions, RunKind};
    let engine = engine("backup-repo", &Replay::new());
    let collections = engine.collections();
    let usb = collections[0].repo("Docs-usb");
    assert!(collections[0].requests(RunKind::Backup, usb, &RestoreOptions::default()).is_err());
    assert!(collections[0].requests(RunKind::Backup, None, &RestoreOptions::default()).is_ok());
}

/// A source that a hook creates learns from the one snapshot that no other source explains. With more of them than
/// such sources, the order would not tell which is whose: none learns.
#[test]
fn a_hooked_source_learns_from_the_one_snapshot_left() {
    let config = "[repository]\nrepository = '/backup'\npassword = 'x'\n\
        [[backup.snapshots]]\nsources = ['docs']\nas-path = '/docs'\n\
        [[backup.snapshots]]\nsources = ['not yet']\nhooks = { run-before = ['mount'] }\n";
    let profile = profile::parse("Docs", config);
    let snapshot = |id: u32, path: &str| -> crate::rustic::Snapshot {
        serde_json::from_value(serde_json::json!({
            "id": format!("{id:08x}"), "time": "2026-10-16T03:00:00Z", "paths": [path], "hostname": "laptop"
        }))
        .unwrap()
    };
    let (docs, mounted, other) = (snapshot(1, "/docs"), snapshot(2, "/mnt/docs"), snapshot(3, "/mnt/other"));
    let paired = |snapshots: &[crate::rustic::Snapshot]| -> Vec<Vec<String>> {
        crate::engine::backup::learned(&profile.sources, snapshots)
            .into_iter()
            .map(|(s, sn)| [s.sources.clone(), sn.paths.clone()].concat())
            .collect()
    };
    assert_eq!(paired(&[docs.clone(), mounted.clone()]), [["not yet", "/mnt/docs"]]);
    assert!(paired(&[docs.clone(), mounted, other]).is_empty(), "two left for one source");
    let unhooked = profile::parse(
        "Docs",
        "[repository]\nrepository = '/backup'\npassword = 'x'\n[[backup.snapshots]]\nsources = ['/home/me/docs']\n",
    );
    assert!(crate::engine::backup::learned(&unhooked.sources, &[docs]).is_empty(), "no source to learn");
}

/// A new computer's hooked sources learn from the newest snapshot of as many unexplained sources, in the order rustic
/// made them. With fewer sources than wanted, none learns.
#[test]
fn hooked_sources_pick_the_newest_unexplained_snapshots() {
    let snapshot = |id: u32, time: &str, path: &str| -> crate::rustic::Snapshot {
        serde_json::from_value(serde_json::json!({ "id": format!("{id:08x}"), "time": time, "paths": [path] })).unwrap()
    };
    let snapshots = [
        snapshot(1, "2026-10-14T03:00:00Z", "/mnt/old"),
        snapshot(2, "2026-10-16T03:00:00Z", "/mnt/b"),
        snapshot(3, "2026-10-15T03:00:00Z", "/mnt/a"),
        snapshot(4, "2026-10-13T03:00:00Z", "/mnt/a"),
    ];
    let ids = |wanted| {
        crate::engine::snapshots::picks(&snapshots, wanted)
            .map(|p| p.iter().map(|s| s.id.short().to_owned()).collect::<Vec<_>>())
    };
    assert_eq!(ids(2), Some(vec!["00000003".to_owned(), "00000002".to_owned()]));
    assert_eq!(ids(4), None, "three sources for four");
}

/// A first backup after a reinstall is held back for the sources that have snapshots of this computer, and that it
/// would not back up or would save empty. Another computer's snapshot, or another source's, does not count.
#[test]
fn a_backup_is_held_back_for_the_sources_it_would_empty() {
    use crate::rustic::{BackupPlan, PlannedSnapshot};
    let config = "[repository]\nrepository = '/backup'\npassword = 'x'\n[backup]\nhost = 'laptop'\n\
        [[backup.snapshots]]\nsources = ['docs']\nas-path = '/docs'\n\
        [[backup.snapshots]]\nsources = ['mail']\nas-path = '/mail'\n";
    let profile = profile::parse("Docs", config);
    let snapshot = |host: &str, path: &str| -> crate::rustic::Snapshot {
        serde_json::from_value(serde_json::json!({
            "id": "00000001", "time": "2026-10-16T03:00:00Z", "paths": [path], "hostname": host
        }))
        .unwrap()
    };
    let paths = |kept: &[&profile::Sources]| kept.iter().flat_map(|s| s.paths()).collect::<Vec<_>>();
    let kept =
        crate::engine::backup::kept(&profile.sources, &[snapshot("laptop", "/docs"), snapshot("desktop", "/mail")]);
    assert_eq!(paths(&kept), ["/docs"]);
    assert!(crate::engine::backup::kept(&profile.sources, &[snapshot("laptop", "/music")]).is_empty());
    let both =
        crate::engine::backup::kept(&profile.sources, &[snapshot("laptop", "/docs"), snapshot("laptop", "/mail")]);
    let planned = |path: &str, files| PlannedSnapshot { paths: vec![path.into()], files, bytes: 0 };
    let plan = BackupPlan { snapshots: vec![planned("/docs", 3), planned("/mail", 0)], not_backed_up: Vec::new() };
    assert_eq!(crate::engine::backup::emptied(&both, &plan), ["/mail"]);
    let failed = BackupPlan { snapshots: Vec::new(), not_backed_up: vec![("docs,other".into(), "gone".into())] };
    assert_eq!(crate::engine::backup::emptied(&both, &failed), ["/docs"]);
}
