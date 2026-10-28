//! Where rustic puts things, as rusticlone must expect: remote sources, sources and repositories that hooks
//! prepare, other backends, shared repositories. Rustic itself runs every command and resolves every path.

use std::{fs, path::Path, time::Duration};

use serde_json::json;

use crate::common::{KEEP, Sandbox, docs_with_remote, wait_for};

/// A profile `name` into `repo/<name>`, whose sources are links that its backup hook creates into
/// `source/<target>`, each as `link-<source>`, and removes when rustic is done.
fn hooked(sb: &Sandbox, name: &str, sources: &[(&str, &str)], extra: &str) {
    let links: String = sources
        .iter()
        .map(|(source, target)| {
            format!(
                "ln -sfn {} {}; ",
                sb.path(&format!("source/{target}")).display(),
                sb.path(&format!("link-{source}")).display()
            )
        })
        .collect();
    let snapshots: String = sources
        .iter()
        .map(|(source, _)| {
            format!("[[backup.snapshots]]\nsources = ['{}']\n\n", sb.path(&format!("link-{source}")).display())
        })
        .collect();
    let unlink: Vec<String> =
        sources.iter().map(|(s, _)| sb.path(&format!("link-{s}")).display().to_string()).collect();
    sb.profile(
        name,
        &format!(
            "[global]\nuse-profiles = ['common']\n\n[repository]\nrepository = '{}'\n\n[backup.hooks]\n\
             run-before = [\"sh -c '{links}'\"]\nrun-finally = ['rm -f {}']\n\n{snapshots}[forget]\nkeep-daily = 7\n\n{extra}",
            sb.path(&format!("repo/{name}")).display(),
            unlink.join(" ")
        ),
    );
}

/// A remote source (`opendal:`) is backed up as the snapshot's root, which rustic records as `""`: its
/// backups are matched and copied, and restored into a folder, but not in place, which is not here.
#[test]
fn a_remote_source_is_restored_into_a_folder() {
    let sb = Sandbox::new();
    sb.write("bucket/docs/a.txt", "a");
    sb.target("Remote-copy", "Remote", "");
    sb.profile(
        "Remote",
        &format!(
            "[global]\nuse-profiles = ['common']\n\n[repository]\nrepository = '{}'\n\n\
             [[backup.snapshots]]\nsources = ['opendal:fs']\n\n[backup.snapshots.options]\nroot = '{}'\n\n\
             [copy]\ntargets = ['Remote-copy']\n",
            sb.path("local/Remote").display(),
            sb.path("source/bucket").display()
        ),
    );
    sb.run(&["backup", "--all"]).assert_exit(0).assert_result("copied", json!({"snapshots": 1}));
    sb.run(&["restore", "--all"]).assert_exit(2).assert_result("no-place", json!({"source": "opendal:fs"}));
    let to = sb.path("restored");
    sb.run(&["restore", "--repo", "Remote-copy", "--to", to.to_str().unwrap()]).assert_exit(0);
    assert_eq!(sb.tree("restored"), sb.tree("source/bucket"));
}

/// Hooks may mount the repository and link the sources into place only while rustic runs. The repository
/// is not missing, its check runs, each source's paths are what rustic records, learned from the first
/// backup, and again from the repository once the state is lost. The sources are not restored in place,
/// because a restore runs no backup hooks.
#[test]
fn hooks_mount_the_repository_and_link_the_sources() {
    let sb = Sandbox::new();
    sb.write("real/one/a.txt", "a");
    sb.write("real/two/b.txt", "b");
    let (away, disk, real, link) = (sb.path("away"), sb.path("disk"), sb.path("source/real"), sb.path("link"));
    fs::create_dir_all(&away).unwrap();
    fs::create_dir_all(&disk).unwrap();
    sb.profile(
        "Hooked",
        &format!(
            "[global]\nuse-profiles = ['common']\n\n[repository]\nrepository = '{disk}/repo'\n\n[repository.hooks]\n\
             run-before = [\"sh -c 'test -d {away}/repo && mv {away}/repo {disk}; true'\"]\n\
             run-finally = [\"sh -c 'test -d {disk}/repo && mv {disk}/repo {away}; true'\"]\n\n\
             [backup.hooks]\nrun-before = ['ln -sfn {real} {link}']\nrun-finally = ['rm {link}']\n\n\
             [[backup.snapshots]]\nsources = ['{link}/two']\n\n[[backup.snapshots]]\nsources = ['{link}/one']\n\n\
             [forget]\nkeep-daily = 7\n",
            away = away.display(),
            disk = disk.display(),
            real = real.display(),
            link = link.display()
        ),
    );
    sb.run(&["backup", "--all"]).assert_exit(0).assert_result("backed-up", json!({}));
    sb.run(&["check", "--all"]).assert_exit(0).assert_no_result("missing-skipped", json!({}));
    sb.run(&["restore", "--all"]).assert_exit(2).assert_result("hooked", json!({}));
    // A new computer: no state.
    sb.remove("store");
    let to = sb.path("restored");
    sb.run(&["restore", "--all", "--to", to.to_str().unwrap()]).assert_exit(0);
    let restored = sb.path(&format!("restored/{}", real.strip_prefix("/").unwrap().display()));
    assert_eq!(fs::read_to_string(restored.join("one/a.txt")).unwrap(), "a");
    assert_eq!(fs::read_to_string(restored.join("two/b.txt")).unwrap(), "b");
}

/// Hooks that run before rustic, as one that mounts a disk, may print into the output that rustic writes its
/// JSON to. Not those that run after: rustic 0.11 has not written all of it yet, and their output lands inside.
#[test]
fn hooks_may_print() {
    let sb = docs_with_remote("");
    let print = if cfg!(windows) { "cmd /c echo [printed by a hook]" } else { "echo [printed by a hook]" };
    let hooks = |kind| format!("[{kind}.hooks]\nrun-before = ['{print}']\n");
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-remote"], &(hooks("repository") + &hooks("backup")));
    sb.target("Docs-remote", "Docs", &hooks("repository"));
    sb.run(&["backup", "--all"]).assert_exit(0).assert_result("copied", json!({"snapshots": 1}));
    sb.run(&["collections"]).assert_exit(0);
}

/// Another profile backs up into the same repository last. After the state is lost, a run that reads only
/// the hooked profile still does not take the other profile's snapshot for its own.
#[test]
fn a_shared_repository_keeps_each_profile_its_snapshots() {
    let sb = Sandbox::new();
    sb.write("real/one/a.txt", "a");
    sb.write("plain/p.txt", "p");
    hooked(&sb, "Hooked", &[("one", "real/one")], "");
    sb.profile(
        "Plain",
        &format!(
            "[global]\nuse-profiles = ['common']\n\n[repository]\nrepository = '{}'\n\n\
             [[backup.snapshots]]\nsources = ['{}']\n",
            sb.path("repo/Hooked").display(),
            sb.path("source/plain").display()
        ),
    );
    sb.run(&["backup", "--collection", "Hooked"]).assert_exit(0);
    sb.run(&["backup", "--collection", "Plain"]).assert_exit(0);
    sb.remove("store");
    let to = sb.path("restored");
    sb.run(&["restore", "--collection", "Hooked", "--to", to.to_str().unwrap()]).assert_exit(0);
    assert_eq!(sb.tree(&format!("restored/{}", sb.path("source/real").strip_prefix("/").unwrap().display())).len(), 1);
    assert!(!to.join(sb.path("source/plain").strip_prefix("/").unwrap()).exists(), "the other profile's data");
}

/// Profiles that reach one repository by different URLs, here a path and `opendal:fs`, share its locks,
/// which are named after the repository's id: a forget waits for a backup into it.
#[test]
fn a_repository_reached_two_ways_is_locked_as_one() {
    let sb = Sandbox::new();
    let shared = sb.path("local/shared");
    sb.profile(
        "Slow",
        &format!(
            "[global]\nuse-profiles = ['common']\n\n[repository]\nrepository = '{}'\n\n\
             [[backup.snapshots]]\nsources = ['-']\nstdin-command = 'sleep 3'\n",
            shared.display()
        ),
    );
    let opendal = |root: &Path| {
        format!(
            "[global]\nuse-profiles = ['common']\n\n[repository]\nrepository = 'opendal:fs'\n\n\
             [repository.options]\nroot = '{}'\n\n",
            root.display()
        )
    };
    let docs = format!(
        "[[backup.snapshots]]\nsources = ['{}']\n\n[copy]\ntargets = ['Docs-fs']\n\n{KEEP}",
        sb.path("source/docs").display()
    );
    sb.profile("Docs", &(opendal(&shared) + &docs));
    sb.profile("Docs-fs", &opendal(&sb.path("remote/fs")));
    sb.write("docs/a.txt", "a");
    assert!(sb.rustic(&["-P", "Slow", "init"]).status.success());
    sb.run(&["backup", "--collection", "Docs"]).assert_exit(0);
    let mut slow = sb.spawn(&["backup", "--collection", "Slow"], &[]);
    wait_for(10, "the slow backup", || sb.store().unfinished(Some("Slow")).iter().any(|r| !r.holds().using.is_empty()));
    sb.run(&["copy", "--collection", "Docs"]).assert_exit(0).assert_result("in-sync", json!({}));
    assert!(slow.wait().unwrap().success());
    let call = |collection: &str, args: &str| {
        let run = sb.store().runs(collection, 1).remove(0);
        run.calls.into_iter().find(|c| c.args.contains(args)).unwrap()
    };
    let (backup, forget) = (call("Slow", "backup --json"), call("Docs", "forget --json"));
    let written = backup.started + Duration::from_secs_f64(backup.secs);
    assert!(forget.started >= written, "the forget ran during the backup");
}

/// A hook that links the source elsewhere: the next backup learns the new place, and restores it.
#[test]
fn a_hook_that_links_elsewhere_is_learned_again() {
    let sb = Sandbox::new();
    sb.write("real/v1/a.txt", "v1");
    sb.write("real/v2/a.txt", "v2");
    hooked(&sb, "Hooked", &[("one", "real/v1")], "");
    sb.run(&["backup", "--all"]).assert_exit(0);
    hooked(&sb, "Hooked", &[("one", "real/v2")], "");
    sb.run(&["backup", "--all"]).assert_exit(0).assert_result("backed-up", json!({}));
    let to = sb.path("restored");
    sb.run(&["restore", "--all", "--to", to.to_str().unwrap()]).assert_exit(0);
    let v2 = sb.path("restored").join(sb.path("source/real/v2/a.txt").strip_prefix("/").unwrap());
    assert_eq!(fs::read_to_string(v2).unwrap(), "v2");
}

/// A hooked source that cannot be backed up: nothing is learned from that backup, whose order would not
/// tell which snapshot is whose. The next one learns every source.
#[test]
fn a_failed_hooked_source_learns_nothing() {
    let sb = Sandbox::new();
    sb.write("real/one/a.txt", "a");
    hooked(&sb, "Hooked", &[("one", "real/one"), ("two", "real/two")], "");
    sb.run(&["backup", "--all"]).assert_exit(1).assert_no_result("backed-up", json!({}));
    sb.write("real/two/b.txt", "b");
    sb.run(&["backup", "--all"]).assert_exit(0).assert_result("backed-up", json!({}));
    let to = sb.path("restored");
    sb.run(&["restore", "--all", "--to", to.to_str().unwrap()]).assert_exit(0);
    assert_eq!(sb.tree(&format!("restored/{}", sb.path("source/real").strip_prefix("/").unwrap().display())).len(), 2);
}

/// A copy target on an `opendal:` backend, here the local file system: created with the primary's chunker,
/// copied to, and restored from.
#[test]
fn an_opendal_copy_target_is_created_and_restored_from() {
    let sb = Sandbox::new();
    sb.write("docs/a.txt", "a");
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-fs"], "");
    sb.profile(
        "Docs-fs",
        &format!(
            "[global]\nuse-profiles = ['common']\n\n[repository]\nrepository = 'opendal:fs'\n\n\
             [repository.options]\nroot = '{}'\n",
            sb.path("remote/fs").display()
        ),
    );
    sb.run(&["backup", "--all"]).assert_exit(0).assert_result("copied", json!({"snapshots": 1}));
    sb.run(&["check", "--all"]).assert_exit(0);
    let to = sb.path("restored");
    sb.run(&["restore", "--repo", "Docs-fs", "--to", to.to_str().unwrap()]).assert_exit(0);
    let restored = to.join(sb.path("source/docs/a.txt").strip_prefix("/").unwrap());
    assert_eq!(fs::read_to_string(restored).unwrap(), "a");
}

/// Stdin was never a file: it is not restored in place, only into a folder.
#[test]
fn stdin_is_restored_only_into_a_folder() {
    let sb = Sandbox::new();
    sb.profile(
        "Dump",
        &format!(
            "[global]\nuse-profiles = ['common']\n\n[repository]\nrepository = '{}'\n\n\
             [[backup.snapshots]]\nsources = ['-']\nstdin-command = 'echo dump'\nstdin-filename = 'dump.sql'\n",
            sb.path("repo/Dump").display()
        ),
    );
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.run(&["restore", "--all"]).assert_exit(2).assert_result("no-place", json!({"source": "-"}));
    let to = sb.path("restored");
    sb.run(&["restore", "--all", "--to", to.to_str().unwrap()]).assert_exit(0);
    assert_eq!(fs::read_to_string(to.join("dump.sql")).unwrap(), "dump\n");
}
