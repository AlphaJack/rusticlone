//! `cargo xtask fixtures`: records what the installed rustic prints where text is parsed, into
//! `tests/fixtures/rustic-MAJOR.MINOR`, for the parser tests. The sandbox has fixed files and times,
//! and paths, ids, users, times and repository sizes are normalized, so that recording again shows
//! only what rustic changed.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::{Context, Result, bail};

/// The path of the sources in the recordings.
const ROOT: &str = "/fixture";

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn rustic(&self, profile: &str, args: &[&str]) -> Result<Output> {
        let mut cmd = Command::new("rustic");
        cmd.args(["--no-cache", "-P", profile]).args(args).current_dir(&self.dir).env("TZ", "UTC").env("NO_COLOR", "1");
        // As the engine runs it: progress only when requested, as JSON.
        if !args.contains(&"--json-progress") {
            cmd.env("RUSTIC_NO_PROGRESS", "true");
        }
        let out = cmd.output().context("cannot run rustic")?;
        if !out.status.success() {
            bail!("rustic {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr));
        }
        Ok(out)
    }

    fn stdout(&self, args: &[&str]) -> Result<String> {
        self.stdout_of("a", args)
    }

    fn stdout_of(&self, profile: &str, args: &[&str]) -> Result<String> {
        Ok(String::from_utf8(self.rustic(profile, args)?.stdout)?)
    }

    fn stderr(&self, args: &[&str]) -> Result<String> {
        Ok(String::from_utf8(self.rustic("a", args)?.stderr)?)
    }

    /// The log of a command that must fail.
    fn failing(&self, args: &[&str]) -> Result<String> {
        let out = Command::new("rustic")
            .args(["--no-cache", "-P", "a"])
            .args(args)
            .current_dir(&self.dir)
            .env("TZ", "UTC")
            .env("NO_COLOR", "1")
            .env("RUSTIC_NO_PROGRESS", "true")
            .output()?;
        if out.status.success() {
            bail!("rustic {} succeeded", args.join(" "));
        }
        Ok(String::from_utf8(out.stderr)?)
    }

    fn file(&self, path: &str, content: &str) -> Result<()> {
        let path = self.dir.join(path);
        fs::create_dir_all(path.parent().context("no parent")?)?;
        Ok(fs::write(path, content)?)
    }

    fn run(&self, program: &str, args: &[&str]) -> Result<()> {
        let status = Command::new(program).args(args).current_dir(&self.dir).status()?;
        if !status.success() {
            bail!("{program} {}: {status}", args.join(" "));
        }
        Ok(())
    }

    /// Sets modification times, to the minute, as `ls` shows them.
    fn touch(&self, at: &str, paths: &[&str]) -> Result<()> {
        self.run("touch", &[&["-h", "-d", at][..], paths].concat())
    }
}

pub(crate) fn record() -> Result<()> {
    if Command::new("id").arg("-u").output().is_ok_and(|o| o.stdout == b"0\n") {
        bail!("run as another user than root, for whom the files made unreadable stay readable");
    }
    let version = Command::new("rustic").arg("--version").output().context("cannot run rustic")?;
    let version = String::from_utf8(version.stdout)?;
    let minor = version
        .split_whitespace()
        .last()
        .and_then(|v| v.trim_start_matches('v').rsplit_once('.').map(|(minor, _)| minor.to_owned()))
        .with_context(|| format!("no version in {version:?}"))?;
    let sb = Sandbox { dir: env::temp_dir().join(format!("xtask-fixtures-{}", std::process::id())) };
    _ = fs::remove_dir_all(&sb.dir);
    let recorded = record_in(&sb);
    _ = sb.run("chmod", &["-R", "u+rwx", "."]);
    _ = fs::remove_dir_all(&sb.dir);
    let files = recorded?;

    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../tests/fixtures/rustic-{minor}"));
    _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out)?;
    for (name, content) in &files {
        fs::write(out.join(name), content)?;
    }
    println!("rustic {}: {} files into {}", version.trim(), files.len(), out.display());
    Ok(())
}

fn record_in(sb: &Sandbox) -> Result<Vec<(&'static str, String)>> {
    let dir = sb.dir.to_str().context("the temporary directory is not UTF-8")?;
    let docs = format!("{dir}/src/docs");
    set_up(sb, dir, &docs)?;
    let mut files = capture(sb, &docs)?;
    anonymize(sb, dir, &mut files)?;
    Ok(files)
}

/// Creates the collections and sources, and runs the first backup.
fn set_up(sb: &Sandbox, dir: &str, docs: &str) -> Result<()> {
    for (name, repo, extra) in [
        (
            "a",
            "repo",
            &*format!(
                "[backup]\nhost = \"host\"\n[[backup.snapshots]]\nsources = [\"{docs}\"]\nas-path = \"{ROOT}/src/docs\"\n"
            ),
        ),
        ("t", "target", ""),
        ("u", "copy", ""),
    ] {
        sb.file(
            &format!("{name}.toml"),
            &format!("[repository]\nrepository = \"{dir}/{repo}\"\npassword = \"test\"\nno-cache = true\n{extra}"),
        )?;
    }
    sb.file("src/docs/a \"x\"\tb\\c.txt", "hello")?;
    sb.file("src/docs/sub/b.txt", "one")?;
    fs::create_dir_all(sb.dir.join("src/docs/empty"))?;
    sb.run("ln", &["-s", "sub/b.txt", "src/docs/link"])?;
    sb.touch(
        "2026-10-15T13:20:00Z",
        &["src/docs", "src/docs/a \"x\"\tb\\c.txt", "src/docs/empty", "src/docs/sub", "src/docs/sub/b.txt"],
    )?;
    sb.touch("2026-10-15T13:21:00Z", &["src/docs/link"])?;
    sb.rustic("a", &["init"])?;
    // Another chunker, so that copies to it are rejected.
    sb.rustic("t", &["init"])?;
    sb.rustic("a", &["backup", "--time", "2026-10-15 13:20:00+0000"])?;
    Ok(())
}

/// What rustic prints for a second backup, copies, listings and failures.
fn capture(sb: &Sandbox, docs: &str) -> Result<Vec<(&'static str, String)>> {
    sb.file("src/docs/sub/b.txt", "two")?;
    sb.file("src/docs/new.txt", "new")?;
    fs::remove_file(sb.dir.join("src/docs/a \"x\"\tb\\c.txt"))?;
    sb.touch("2026-10-16T09:00:00Z", &["src/docs", "src/docs/sub", "src/docs/sub/b.txt", "src/docs/new.txt"])?;
    // As the engine runs a backup: progress and summary as JSON lines.
    let backup = sb.stdout(&[
        "--json-progress",
        "--progress-interval",
        "1s",
        "backup",
        "--json",
        "--time",
        "2026-10-16 09:00:00+0000",
    ])?;
    let snapshots = sb.stdout(&["snapshots", "--json"])?;
    let ids = snapshot_ids(&snapshots)?;
    let [first, second] = &ids[..] else { bail!("expected 2 snapshots, got {ids:?}") };
    sb.rustic("a", &["copy", "--filter-paths-exact", "/nonexistent", "--target", "u", "--init"])?;
    sb.rustic("a", &["copy", first, "--target", "u"])?;

    let source = format!("{ROOT}/src/docs");
    let mut files = vec![
        ("backup.stdout", backup),
        ("snapshots.stdout", snapshots),
        // A copy has its own id, and the original's in `original`.
        ("snapshots-copied.stdout", sb.stdout_of("u", &["snapshots", "--json"])?),
        ("snapshots-copied-both.stdout", {
            sb.rustic("a", &["copy", second, "--target", "u"])?;
            sb.stdout_of("u", &["snapshots", "--json"])?
        }),
        ("cat-snapshot.stdout", sb.stdout(&["cat", "snapshot", first])?),
        ("repoinfo.stdout", sb.stdout(&["repoinfo", "--only-index", "--json"])?),
        ("cat-config.stdout", sb.stdout(&["cat", "config"])?),
        (
            "forget.stdout",
            sb.stdout(&["forget", "--json", "--dry-run", "--keep-last", "1", "--filter-paths-exact", &source])?,
        ),
        ("ls.stdout", sb.stdout(&["ls", "--long", "--recursive", first])?),
        ("find.stdout", sb.stdout(&["find", "--iglob=*b.txt*", "--filter-paths-exact", &source])?),
        // The same file in both snapshots: reported once for both.
        ("find-unchanged.stdout", sb.stdout(&["find", "--iglob=*link*", "--filter-paths-exact", &source])?),
        ("diff.stdout", sb.stdout(&["diff", first, second])?),
        (
            "restore-dry-run.stdout",
            sb.stdout(&["restore", "--dry-run", "--verify-existing", &format!("{first}:{source}"), docs])?,
        ),
    ];
    sb.file("src/docs/secret", "secret")?;
    sb.run("chmod", &["000", "src/docs/sub", "src/docs/secret"])?;
    files.push(("backup-skipped.stderr", sb.stderr(&["backup", "--time", "2026-10-17 09:00:00+0000"])?));
    sb.run("chmod", &["700", "src/docs/sub", "src/docs/secret"])?;
    // Rustic exits 0 even so.
    files.push(("copy-refused.stderr", sb.stderr(&["copy", "--target", "t"])?));
    // An unreadable file: its path in the context, and the system's reason as the cause.
    sb.run("chmod", &["000", &format!("repo/snapshots/{first}")])?;
    files.push(("unreadable.stderr", sb.failing(&["snapshots", "--json"])?));
    sb.run("chmod", &["600", &format!("repo/snapshots/{first}")])?;
    Ok(files)
}

/// Replaces what varies between recordings, or between users, with fixed values.
fn anonymize(sb: &Sandbox, dir: &str, files: &mut [(&'static str, String)]) -> Result<()> {
    // The primary's snapshots, oldest first, then the copy's.
    let mut ids = snapshot_ids(&sb.stdout(&["snapshots", "--json"])?)?;
    ids.extend(snapshot_ids(&sb.stdout_of("u", &["snapshots", "--json"])?)?);
    let user = |flag| -> Result<String> {
        Ok(String::from_utf8(Command::new("id").arg(flag).output()?.stdout)?.trim().to_owned())
    };
    let (user, group) = (user("-un")?, user("-gn")?);
    // The repository's random values, replaced with fixed ones.
    let config: serde_json::Value = serde_json::from_str(&sb.stdout(&["cat", "config"])?)?;
    let random: Vec<(String, String)> = [("id", 'c'), ("chunker_polynomial", 'd')]
        .into_iter()
        .filter_map(|(key, digit)| {
            let value = config[key].as_str()?;
            Some((value.to_owned(), digit.to_string().repeat(value.len())))
        })
        .collect();
    let home = env::var("HOME").unwrap_or_default();
    for (name, text) in files {
        let mut out = text.replace(dir, ROOT);
        if let Some(json) = stable_json(&out).ok().filter(|_| name.ends_with(".stdout")) {
            out = json;
        }
        for (value, fixed) in &random {
            out = out.replace(value, fixed);
        }
        for (i, id) in ids.iter().enumerate() {
            let same = char::from_digit(u32::try_from(i)? + 1, 16).context("too many snapshots")?.to_string();
            out = out.replace(id.as_str(), &same.repeat(64));
            out = out.replace(id.get(..8).context("short id")?, &same.repeat(8));
        }
        // Keeps the width of the `ls` columns.
        for name in [&user, &group] {
            out = out.replace(&format!(" {name} "), &format!(" {:>w$} ", "me", w = name.len()));
        }
        if !home.is_empty() {
            out = out.replace(&home, "/home/me");
        }
        // Varies with the random ids and keys stored with the data.
        let added = "[INFO] Added to the repo: ";
        let lines: Vec<&str> = out
            .lines()
            .map(|l| if l.starts_with(added) { "[INFO] Added to the repo: 1 KiB (raw: 2 KiB)" } else { l })
            .collect();
        *text = lines.join("\n") + if out.ends_with('\n') { "\n" } else { "" };
    }
    Ok(())
}

/// Oldest first.
fn snapshot_ids(json: &str) -> Result<Vec<String>> {
    let groups: serde_json::Value = serde_json::from_str(json)?;
    let mut snapshots: Vec<(String, String)> = groups
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|g| g["snapshots"].as_array().into_iter().flatten())
        .filter_map(|s| Some((s["time"].as_str()?.to_owned(), s["id"].as_str()?.to_owned())))
        .collect();
    snapshots.sort();
    Ok(snapshots.into_iter().map(|(_, id)| id).collect())
}

/// Replaces what varies between recordings with fixed values: tree ids, the time and duration of a
/// backup, and sizes after compression and encryption. JSON lines stay single lines.
fn stable_json(text: &str) -> Result<String> {
    const VARYING: &[&str] = &[
        "tree",
        "backup_start",
        "backup_end",
        "backup_duration",
        "total_duration",
        "seconds_elapsed",
        "total_dirsize_processed",
        "data_added",
        "data_added_packed",
        "data_added_files",
        "data_added_files_packed",
        "data_added_trees",
        "data_added_trees_packed",
        "size",
        "data_size",
        "min_size",
        "max_size",
    ];
    fn stable(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, value) in map.iter_mut() {
                    if !VARYING.contains(&key.as_str()) {
                        stable(value);
                        continue;
                    }
                    *value = match value {
                        serde_json::Value::Number(_) => 1000.into(),
                        serde_json::Value::String(_) if key == "tree" => "0".repeat(64).into(),
                        serde_json::Value::String(_) => "2026-10-15T13:20:00+00:00".into(),
                        serde_json::Value::Null
                        | serde_json::Value::Bool(_)
                        | serde_json::Value::Array(_)
                        | serde_json::Value::Object(_) => continue,
                    };
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(stable),
            serde_json::Value::Null
            | serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::String(_) => {}
        }
    }
    if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(text) {
        stable(&mut value);
        return Ok(serde_json::to_string_pretty(&value)? + "\n");
    }
    let mut lines = Vec::new();
    for line in text.lines() {
        let mut value: serde_json::Value = serde_json::from_str(line)?;
        stable(&mut value);
        lines.push(serde_json::to_string(&value)?);
    }
    Ok(lines.join("\n") + "\n")
}
