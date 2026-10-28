//! A dry run changes no file: not the sources, the repositories or the state, whatever the command. Only
//! lock files may appear: a dry run waits for other runs, as any run does.

use std::{collections::BTreeMap, fs, path::Path, time::SystemTime};

use crate::common::{Sandbox, docs_with_remote};

/// Each file below the sources, the repositories and the state, but its locks, with its size and
/// modification time.
fn files(sb: &Sandbox) -> BTreeMap<String, (u64, SystemTime)> {
    fn walk(dir: &Path, locks: &Path, into: &mut BTreeMap<String, (u64, SystemTime)>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            // Not the entry's metadata: on Windows, that is the folder listing's copy, which NTFS updates
            // late.
            let path = entry.path();
            let meta = fs::metadata(&path).unwrap();
            if path == locks {
                continue;
            }
            into.insert(path.display().to_string(), (meta.len(), meta.modified().unwrap()));
            if meta.is_dir() {
                walk(&path, locks, into);
            }
        }
    }
    let (locks, mut files) = (sb.path("store/locks"), BTreeMap::new());
    for dir in ["source", "local", "remote", "store"] {
        walk(&sb.path(dir), &locks, &mut files);
    }
    files
}

#[test]
fn dry_runs_change_no_file() {
    let sb = docs_with_remote("[forget]\nkeep-last = 1\n");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.write("docs/b.txt", "b");
    sb.run(&["backup", "--all"]).assert_exit(0);
    // Every step has work to do: a new file to back up, snapshots to forget and prune.
    sb.write("docs/c.txt", "c");
    sb.common_env(&[("BACKUP_CRON", "* * * * *"), ("PRUNE_CRON", "* * * * *"), ("CHECK_CRON", "* * * * *")]);
    let before = files(&sb);
    let to = sb.path("restored").display().to_string();
    let commands: [&[&str]; 10] = [
        &["due", "--all"],
        &["backup", "--all"],
        &["copy", "--all"],
        &["prune", "--all"],
        &["check", "--all"],
        &["restore", "--all"],
        &["restore", "--all", "--missing"],
        &["restore", "--all", "--to", &to],
        &["recreate", "--repo", "Docs-remote"],
        &["collections", "--refresh"],
    ];
    for command in commands {
        let run = sb.run(&[command, &["--dry-run"]].concat());
        assert_ne!(run.code, 1, "{command:?}\n{}", run.output);
        let after = files(&sb);
        let changed: Vec<&String> =
            before.keys().chain(after.keys()).filter(|f| before.get(*f) != after.get(*f)).collect();
        assert!(changed.is_empty(), "{command:?} changed {changed:?}\n{}", run.output);
    }
    // The findings, not what was done.
    let backup = sb.run(&["backup", "--dry-run", "--all"]);
    backup.assert_result("would-back-up", serde_json::json!({})).assert_no_output("rustic backup --json");
}

/// A dry run creates no repository, though rustic's own dry run would, and no state.
#[test]
fn a_dry_run_creates_nothing() {
    let sb = docs_with_remote("");
    let run = sb.run(&["backup", "--dry-run", "--all"]);
    run.assert_result("would-create-primary", serde_json::json!({}));
    run.assert_result("would-copy", serde_json::json!({"target": "Docs-remote", "create": true}));
    for dir in ["local/Docs", "remote/Docs", "store"] {
        assert!(!sb.path(dir).exists(), "{dir} exists\n{}", run.output);
    }
}

/// With restore choices, a dry run only says what would be restored where, from the listings, without
/// reading the snapshots. Without them, it reads them as a restore would, and leaves no folder behind for a
/// source that is gone, though rustic's own dry run creates it.
#[test]
fn a_dry_run_restore_previews_or_reads() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.remove("source/docs");
    let preview = sb.run(&["restore", "--dry-run", "--all", "--missing"]);
    preview.assert_exit(0).assert_result("would-restore", serde_json::json!({})).assert_no_output("rustic dump");
    let read = sb.run(&["restore", "--dry-run", "--all"]);
    read.assert_exit(0).assert_result("test-restored", serde_json::json!({}));
    assert!(!sb.path("source/docs").exists(), "a dry run created the source");
}
