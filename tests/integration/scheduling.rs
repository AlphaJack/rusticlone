use std::fs::{self, File};

use serde_json::json;

use crate::common::{KEEP, Sandbox, docs_with_remote, global_env, objects, var};

const EVERY_MINUTE: &str = "* * * * *";

#[test]
fn nothing_is_scheduled_by_default() {
    let sb = docs_with_remote("");
    let run = sb.run(&["due", "--all"]);
    run.assert_exit(0).assert_silent();
    assert!(sb.snapshots("Docs").is_empty());
    let unscheduled =
        json!({"backup_schedule": null, "primary": {"name": "Docs", "prune_schedule": null, "check_schedule": null}});
    sb.run(&["collections"]).assert_json(&unscheduled);
}

#[test]
fn settings_a_profile_cannot_use_are_reported() {
    let sb = Sandbox::new();
    let profile_env = global_env(&[("BACKUP_CRON", "@daily"), ("STALL_TIMEOUT", "2h")]);
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-remote", "Docs-usb"], &profile_env);
    sb.target("Docs-remote", "Docs", &global_env(&[("BACKUP_CRON", "@hourly"), ("PRUNE_CRONN", "@weekly")]));
    // The same as the collection's: inherited from a shared collection, not set for the target.
    sb.target("Docs-usb", "usb/Docs", &global_env(&[("BACKUP_CRON", "@daily")]));
    let out = sb.run(&["collections"]);
    out.assert_exit(2)
        .assert_result("ignored", json!({"key": var("STALL_TIMEOUT"), "why": "environment-only"}))
        .assert_result("ignored", json!({"key": var("BACKUP_CRON"), "why": {"follows-profiles": ["Docs"]}}))
        .assert_result("ignored", json!({"key": var("PRUNE_CRONN"), "why": "unknown"}));
    let document = out.json();
    assert_eq!(objects(&document).iter().filter(|o| o["kind"] == "ignored").count(), 3, "{}", out.stdout);
    // Also a issue of the collection: a typo leaves a step unscheduled.
    let typo = json!({"repo": "Docs-remote", "setting": {"key": var("PRUNE_CRONN"), "why": "unknown"}});
    sb.run(&["collections"]).assert_result("setting-ignored", typo);
}

#[test]
fn shadowed_profile_is_reported() {
    let sb = docs_with_remote("");
    // The collection and its target in rustic's working directory, the target also in the user's directory.
    let (user, cwd) = (sb.profile_dir(), sb.path("cwd"));
    fs::rename(user.join("Docs.toml"), cwd.join("Docs.toml")).unwrap();
    fs::copy(user.join("Docs-remote.toml"), cwd.join("Docs-remote.toml")).unwrap();
    let (read, hidden) = (user.join("Docs-remote.toml"), cwd.join("Docs-remote.toml"));
    let (read, hidden) = (read.display().to_string(), hidden.display().to_string());
    sb.run(&["collections", "--collection", "Docs"])
        .assert_exit(2)
        .assert_result("hidden", json!({"read": read, "hidden": [hidden]}));
}

#[test]
fn run_backs_up_only_when_due() {
    let sb = docs_with_remote("");
    sb.common_env(&[("BACKUP_CRON", "0 0 * * *")]);
    // Never backed up: due at once, because nothing has the sources yet.
    sb.run(&["due", "--all"])
        .assert_exit(0)
        .assert_output("rustic backup")
        .assert_result("copied", json!({"snapshots": 1}));
    // It just ran: the next one is due at midnight.
    sb.run(&["due", "--all"]).assert_exit(0).assert_no_output("rustic backup");
    assert_eq!(sb.snapshots("Docs").len(), 1);
}

#[test]
fn profile_settings_override_the_profiles_they_use() {
    let sb = docs_with_remote(&global_env(&[("BACKUP_CRON", "@yearly")]));
    sb.common_env(&[("BACKUP_CRON", EVERY_MINUTE)]);
    // Never backed up: a warning.
    sb.run(&["collections"]).assert_exit(2).assert_json(&json!({"backup_schedule": {"cron": "@yearly"}}));
}

#[test]
fn profile_settings_in_the_environment_are_refused() {
    let sb = docs_with_remote("");
    let run = sb.run_with(&["collections"], &[(&var("BACKUP_CRON"), EVERY_MINUTE)]);
    run.assert_exit(1).assert_output(&format!("{} is read from profiles only", var("BACKUP_CRON")));
}

#[test]
fn failed_steps_are_retried_with_backoff_not_on_every_run() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["missing"]], &[], "");
    sb.common_env(&[("BACKUP_CRON", EVERY_MINUTE)]);
    sb.run(&["due", "--all"]).assert_exit(1).assert_result("failed", json!({}));
    // The first retry waits 5 minutes, even though the schedule says every minute.
    sb.run(&["due", "--all"]).assert_exit(0).assert_no_result("failed", json!({}));
}

#[test]
fn run_skips_a_job_that_is_busy() {
    let sb = docs_with_remote("");
    fs::create_dir_all(sb.path("store/locks")).unwrap();
    // The lock taken while collection `Docs` backs up (FNV-1a of "run:Docs:backup").
    let hash = "run:Docs:backup"
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    let lock = File::create(sb.path(&format!("store/locks/{hash:016x}.lock"))).unwrap();
    lock.lock().unwrap();
    sb.common_env(&[("BACKUP_CRON", EVERY_MINUTE)]);
    sb.run(&["due", "--all"]).assert_exit(0).assert_result("running-already", json!({}));
    assert!(sb.snapshots("Docs").is_empty());
}

#[test]
fn a_backup_made_with_rustic_counts_once_seen() {
    let sb = docs_with_remote("");
    assert!(sb.rustic(&["-P", "Docs", "backup", "--init"]).status.success());
    // A new state lists the repositories first.
    let status = sb.run(&["collections"]);
    status.assert_no_result("never-backed-up", json!({}));
    assert!(status.json()["collections"][0]["last_backup"].is_string(), "{}", status.stdout);
    // The daily backup it already ran is done.
    sb.common_env(&[("BACKUP_CRON", "@daily")]);
    sb.run(&["due", "--all"]).assert_exit(0).assert_no_output("rustic backup");
    assert_eq!(sb.snapshots("Docs").len(), 1);
}

/// The runs recorded for `collection`, oldest first: (kind, scope).
fn recorded(sb: &Sandbox, profile: &str) -> Vec<(String, String)> {
    let runs = sb.store().runs(profile, usize::MAX).into_iter().rev();
    runs.map(|r| (<&str>::from(r.kind).to_owned(), r.scope.unwrap_or_default())).collect()
}

#[test]
fn scheduled_work_is_recorded_as_what_it_did() {
    let sb = docs_with_remote("");
    sb.common_env(&[("BACKUP_CRON", EVERY_MINUTE), ("CHECK_CRON", EVERY_MINUTE)]);
    sb.scheduled_long_ago(&["check:Docs", "check:Docs-remote"]);
    sb.run(&["due", "--all"]).assert_exit(0);
    let runs = recorded(&sb, "Docs");
    let names = |o: &str, s: &str| (o.to_owned(), s.to_owned());
    assert_eq!(runs, [names("backup", ""), names("check", "Docs"), names("check", "Docs-remote")]);
}

#[test]
fn a_target_known_to_lack_snapshots_is_caught_up_before_its_prune() {
    let sb = docs_with_remote("");
    sb.target("Docs-remote", "Docs", KEEP);
    sb.run(&["backup", "--all"]).assert_exit(0);
    // A snapshot made with rustic: no copy is pending, but the target lacks it.
    sb.write("docs/a.txt", "version 2");
    assert!(sb.rustic(&["-P", "Docs", "backup"]).status.success());
    sb.run(&["collections", "--refresh"]).assert_json(&json!({"name": "Docs-remote", "status": {"kind": "behind"}}));
    sb.scheduled_long_ago(&["prune:Docs-remote"]);
    sb.common_env(&[("PRUNE_CRON", EVERY_MINUTE)]);
    sb.run(&["due", "--all"])
        .assert_exit(0)
        .assert_result("copied", json!({"target": "Docs-remote", "snapshots": 1}))
        .assert_result("pruned", json!({"repo": "Docs-remote"}));
    assert_eq!(sb.snapshots("Docs-remote").len(), 2);
}

#[test]
fn a_prune_waiting_for_a_copy_is_not_retried_every_minute() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.write("docs/a.txt", "version 2");
    assert!(sb.rustic(&["-P", "Docs", "backup"]).status.success());
    sb.run(&["collections", "--refresh"]).assert_json(&json!({"name": "Docs-remote", "status": {"kind": "behind"}}));
    // The target is unreachable.
    fs::rename(sb.path("remote"), sb.path("remote-away")).unwrap();
    fs::write(sb.path("remote"), "").unwrap();
    sb.scheduled_long_ago(&["prune:Docs-remote"]);
    sb.common_env(&[("PRUNE_CRON", EVERY_MINUTE)]);
    sb.run(&["due", "--all"])
        .assert_result("failed", json!({}))
        .assert_no_result("pruned", json!({"repo": "Docs-remote"}))
        .assert_no_result("prune-postponed", json!({"repo": "Docs-remote"}));
    let before = recorded(&sb, "Docs").len();
    // A minute later, the copy waits for its retry and the prune for the copy: nothing is recorded.
    sb.run(&["due", "--all"]).assert_exit(0).assert_silent();
    assert_eq!(recorded(&sb, "Docs").len(), before);
}

/// The repository has this computer's snapshots, no backup ran here, and the sources
/// are empty (a reinstalled computer): a scheduled backup waits for a restore or a manual backup,
/// then continues. With the sources present (only the state was lost), it continues at once.
#[test]
fn a_scheduled_first_backup_waits_when_this_computers_snapshots_exist() {
    let sb = docs_with_remote("");
    assert!(sb.rustic(&["-P", "Docs", "backup", "--init", "--time", "2020-01-01 00:00:00+0000"]).status.success());
    fs::remove_file(sb.path("source/docs/a.txt")).unwrap();
    sb.common_env(&[("BACKUP_CRON", "@daily")]);
    // Rustic's dry run finds the source empty: no backup.
    sb.run(&["due", "--all"])
        .assert_exit(2)
        .assert_result("held-back", json!({}))
        .assert_output("rustic backup --dry-run")
        .assert_no_output("rustic backup --json");
    assert_eq!(sb.snapshots("Docs").len(), 1);
    // Held back until its next scheduled time, not again at once, which would make the server loop.
    sb.run(&["due", "--all"]).assert_exit(0).assert_no_result("held-back", json!({}));
    // After the restore, backups continue.
    sb.run(&["restore", "--all"]).assert_exit(0);
    sb.common_env(&[("BACKUP_CRON", EVERY_MINUTE)]);
    sb.run(&["due", "--all"]).assert_exit(0).assert_output("rustic backup");
}

/// Once this state has a successful backup, a scheduled backup runs without a dry run first, though the
/// repository has this computer's snapshots: only a first backup, as after a reinstall, asks rustic.
#[test]
fn a_scheduled_backup_after_a_recorded_one_has_no_dry_run() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &[], "");
    sb.write("docs/a.txt", "version 1");
    assert!(sb.rustic(&["-P", "Docs", "backup", "--init", "--time", "2020-01-01 00:00:00+0000"]).status.success());
    // The restore records the backup it restored, from 2020, so the next one is due at once.
    sb.run(&["restore", "--all"]).assert_exit(0);
    sb.common_env(&[("BACKUP_CRON", EVERY_MINUTE)]);
    sb.run(&["due", "--all"]).assert_exit(0).assert_output("rustic backup --json").assert_no_output("--dry-run");
}

/// A source that a backup hook fills counts as rustic sees it, once the hook ran: the backup carries on.
#[test]
fn a_scheduled_first_backup_carries_on_when_a_hook_fills_the_source() {
    let sb = Sandbox::new();
    let file = sb.path("source/docs/a.txt");
    let hook = format!("[backup.hooks]\nrun-before = [\"sh -c 'echo version 1 > {}'\"]\n", file.display());
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-remote"], &hook);
    sb.target("Docs-remote", "Docs", "");
    sb.write("docs/a.txt", "version 1");
    assert!(sb.rustic(&["-P", "Docs", "backup", "--init", "--time", "2020-01-01 00:00:00+0000"]).status.success());
    fs::remove_file(&file).unwrap();
    sb.common_env(&[("BACKUP_CRON", "@daily")]);
    sb.run(&["due", "--all"])
        .assert_exit(0)
        .assert_output("rustic backup --json")
        .assert_no_result("held-back", json!({}));
}

/// Only the state was lost (a new state directory or install) and the sources are present: a
/// scheduled backup continues, without a warning or a manual backup.
#[test]
fn a_scheduled_first_backup_carries_on_when_only_the_state_is_new() {
    let sb = docs_with_remote("");
    assert!(sb.rustic(&["-P", "Docs", "backup", "--init", "--time", "2020-01-01 00:00:00+0000"]).status.success());
    sb.common_env(&[("BACKUP_CRON", "@daily")]);
    sb.run(&["due", "--all"]).assert_exit(0).assert_output("rustic backup").assert_no_result("held-back", json!({}));
    assert_eq!(sb.snapshots("Docs").len(), 2);
}
