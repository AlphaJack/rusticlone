use std::fs;

use serde_json::json;

use crate::common::{Sandbox, docs_with_remote};

#[test]
fn expired_local_snapshots_wait_for_every_target() {
    let sb = Sandbox::new();
    fs::write(sb.path("remote/unplugged"), "").unwrap();
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-usb"], "[forget]\nkeep-last = 1\n");
    sb.target("Docs-usb", "unplugged/Docs", "");
    sb.write("docs/a.txt", "version 1");
    sb.run(&["backup", "--all"]).assert_exit(1);
    sb.write("docs/a.txt", "version 2");
    sb.run(&["backup", "--all"]).assert_exit(1).assert_result("unmet", json!({}));
    assert_eq!(sb.snapshots("Docs").len(), 2);
}

#[test]
fn copied_snapshots_are_forgotten_by_policy() {
    let sb = docs_with_remote("[forget]\nkeep-last = 1\n");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.write("docs/a.txt", "version 2");
    sb.run(&["backup", "--all"]).assert_exit(0).assert_result("forgot", json!({"snapshots": 1}));
    assert_eq!(sb.snapshots("Docs").len(), 1);
    // The target has no [forget] policy of its own: it keeps everything.
    assert_eq!(sb.snapshots("Docs-remote").len(), 2);
    sb.assert_stored_matches("Docs", &["Docs", "Docs-remote"]);
}

#[test]
fn forget_only_touches_the_jobs_own_sources() {
    let sb = Sandbox::new();
    // Two collections sharing one repository; only A has a policy.
    sb.backup_profile_in("A", "local/shared", &[&["a"]], &[], "[forget]\nkeep-last = 1\n");
    sb.backup_profile_in("B", "local/shared", &[&["b"]], &[], "");
    sb.write("a/file", "a");
    sb.write("b/file", "b");
    for _ in 0..2 {
        sb.run(&["backup", "--all"]).assert_exit(0);
    }
    // A keeps its newest snapshot, B keeps both.
    assert_eq!(sb.snapshots("A").len(), 3);
}

/// A source that is nearly empty again (its disk did not mount, the computer was reinstalled):
/// forgetting would leave only the empty snapshot. The full one is kept, and the collection says why.
#[test]
fn a_source_that_shrank_to_almost_nothing_keeps_what_it_held() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &[], "[forget]\nkeep-last = 1\n");
    sb.write("docs/big.bin", &"x".repeat(2 << 20));
    sb.run(&["backup", "--all"]).assert_exit(0);
    fs::remove_file(sb.path("source/docs/big.bin")).unwrap();
    sb.write("docs/a.txt", "a");
    sb.run(&["backup", "--all"]).assert_exit(2).assert_result("forgot", json!({"shrunk": 1}));
    assert_eq!(sb.snapshots("Docs").len(), 2);
    sb.run(&["collections"]).assert_exit(2).assert_result("shrunk", json!({}));
    // The same size again: forgotten by policy.
    sb.write("docs/big.bin", &"y".repeat(2 << 20));
    sb.run(&["backup", "--all"]).assert_exit(0).assert_result("forgot", json!({"snapshots": 2}));
}
