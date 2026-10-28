//! A sandbox per test: a private HOME and XDG tree on a RAM-backed tmpfs and a cleared environment,
//! so that neither rustic nor we see the developer's own collections or repositories.

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Component, Path, PathBuf},
    process::{Command, Output},
};

use serde_json::{Value, json};

use rusticlone::{ENV_PREFIX, Event, StepKey, Store};

pub(crate) const NAME: &str = env!("CARGO_PKG_NAME");
const BIN: &str = env!("CARGO_BIN_EXE_rusticlone");

/// The environment variable `key`, with our prefix.
pub(crate) fn var(key: &str) -> String {
    format!("{ENV_PREFIX}{key}")
}

/// A `[global.env]` section setting these variables.
pub(crate) fn global_env(vars: &[(&str, &str)]) -> String {
    vars.iter().fold("[global.env]\n".to_owned(), |toml, (key, value)| toml + &format!("{} = '{value}'\n", var(key)))
}

/// The sandbox's repositories' master key, from `rustic key create`. With a password, rustic would derive
/// the key with scrypt on every call, which takes most of a call's time.
const KEY: &str = r#"{"mac":{"k":"EkjZwyPH/BJv6uYve2gN0g==","r":"pg7Pf5rk8XbJqnP8yCLxfw=="},"encrypt":"6s3UuAVp1lKecfYKnk5NbAsbGFfcEpYoE3rYteG/JJk="}"#;

pub(crate) struct Sandbox {
    root: PathBuf,
}

/// One invocation, with `--json`.
pub(crate) struct Run {
    pub code: i32,
    /// The report (stdout).
    pub stdout: String,
    /// Stdout and stderr (the report and rustic's log).
    pub output: String,
}

/// Whether `value` has every field of `fields`, recursively.
pub(crate) fn has(value: &Value, fields: &Value) -> bool {
    match (value, fields) {
        (Value::Object(value), Value::Object(fields)) => {
            fields.iter().all(|(k, f)| value.get(k).is_some_and(|v| has(v, f)))
        }
        _ => value == fields,
    }
}

/// Every object in `value`, itself included.
pub(crate) fn objects(value: &Value) -> Vec<&Value> {
    let inner: Vec<&Value> = match value {
        Value::Object(map) => map.values().flat_map(objects).collect(),
        Value::Array(items) => items.iter().flat_map(objects).collect(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Vec::new(),
    };
    std::iter::once(value).filter(|v| v.is_object()).chain(inner).collect()
}

/// Whether some object in the JSON `text` (one document, or one per line) has every field of `fields`.
pub(crate) fn json_has(text: &str, fields: &Value) -> bool {
    let documents = serde_json::from_str(text).map_or_else(
        |_| text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect(),
        |document| vec![document],
    );
    documents.iter().flat_map(objects).any(|o| has(o, fields))
}

impl Run {
    #[track_caller]
    pub(crate) fn assert_exit(&self, code: i32) -> &Self {
        assert_eq!(self.code, code, "unexpected exit code, output:\n{}", self.output);
        self
    }

    /// The JSON document that `collections` prints after rustic's log.
    pub(crate) fn json(&self) -> Value {
        let last = self.stdout.lines().last().unwrap_or_default();
        serde_json::from_str(last).unwrap()
    }

    /// An object with these fields, wherever the JSON output has it.
    #[track_caller]
    pub(crate) fn assert_json(&self, fields: &Value) -> &Self {
        assert!(json_has(&self.stdout, fields), "no {fields} in output:\n{}", self.stdout);
        self
    }

    #[track_caller]
    pub(crate) fn assert_no_json(&self, fields: &Value) -> &Self {
        assert!(!json_has(&self.stdout, fields), "unexpected {fields} in output:\n{}", self.stdout);
        self
    }

    /// Nothing reported: no result, issue or log line of ours.
    #[track_caller]
    pub(crate) fn assert_silent(&self) -> &Self {
        assert!(!json_has(&self.stdout, &json!({"target": NAME})), "unexpected report:\n{}", self.stdout);
        self
    }

    /// A result, issue or status of `kind` with these fields.
    #[track_caller]
    pub(crate) fn assert_result(&self, kind: &str, fields: Value) -> &Self {
        self.assert_json(&with_kind(kind, fields))
    }

    #[track_caller]
    pub(crate) fn assert_no_result(&self, kind: &str, fields: Value) -> &Self {
        let fields = with_kind(kind, fields);
        assert!(!json_has(&self.stdout, &fields), "unexpected {fields} in output:\n{}", self.stdout);
        self
    }

    /// Rustic's own text, such as the commands run.
    #[track_caller]
    pub(crate) fn assert_output(&self, needle: &str) -> &Self {
        assert!(self.output.contains(needle), "`{needle}` missing from output:\n{}", self.output);
        self
    }

    #[track_caller]
    pub(crate) fn assert_no_output(&self, needle: &str) -> &Self {
        assert!(!self.output.contains(needle), "unexpected `{needle}` in output:\n{}", self.output);
        self
    }
}

pub(crate) fn with_kind(kind: &str, mut fields: Value) -> Value {
    fields["kind"] = kind.into();
    fields
}

/// Whether process `pid` has ended: on Unix, signal 0 reaches every process that exists.
/// Sends the signal `name` (`INT`, `TERM`) to a process that [`Sandbox::spawn`] started, as Ctrl-C or
/// `systemctl stop` would. Windows has no signals: there, Ctrl-Break to its process group, which stops it
/// as both do. Whether it was sent.
pub(crate) fn signal(pid: u32, name: &str) -> bool {
    #[cfg(unix)]
    return Command::new("kill").args([format!("-{name}"), pid.to_string()]).status().is_ok_and(|s| s.success());
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent};
        _ = name;
        // SAFETY: a plain call, about a process group.
        #[expect(unsafe_code, reason = "sends Ctrl-Break to a process group")]
        let sent = unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) };
        sent != 0
    }
}

#[cfg(unix)]
#[expect(unsafe_code, reason = "signal 0 only checks that the process exists")]
fn ended(pid: i32) -> bool {
    // SAFETY: no signal is sent.
    let sent = unsafe { libc::kill(pid, 0) };
    sent == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

#[cfg(not(unix))]
fn ended(_: i32) -> bool {
    false
}

impl Sandbox {
    /// A fresh sandbox named after the running test.
    pub(crate) fn new() -> Self {
        static ENDED_GONE: std::sync::Once = std::sync::Once::new();
        assert!(
            Command::new("rustic").arg("--version").output().is_ok_and(|o| o.status.success()),
            "the integration tests need `rustic` in PATH"
        );
        let test = std::thread::current().name().unwrap_or("test").replace("::", "-");
        let base = ["/dev/shm"].iter().map(PathBuf::from).find(|d| d.is_dir()).unwrap_or_else(env::temp_dir);
        // One directory per test process, inside a shared one. Directories of ended processes are removed,
        // including kept ones.
        let runs = base.join(env!("CARGO_PKG_NAME"));
        ENDED_GONE.call_once(|| {
            for run in fs::read_dir(&runs).into_iter().flatten().flatten() {
                let pid = run.file_name().to_string_lossy().into_owned();
                if pid.parse().is_ok_and(ended) {
                    _ = fs::remove_dir_all(run.path());
                }
            }
        });
        let root = runs.join(std::process::id().to_string()).join(test);
        _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        // Canonical, as rustic records source paths (on macOS, /var is /private/var). Without Windows's
        // `\\?\` prefix, which rustic does not record.
        let sb = Self { root: dunce::canonicalize(&root).unwrap() };
        for dir in ["home", "config", "state", "cwd", "source", "local", "remote", "tmp"] {
            fs::create_dir_all(sb.path(dir)).unwrap();
        }
        fs::create_dir_all(sb.profile_dir()).unwrap();
        sb.common_env(&[]);
        #[cfg(windows)]
        {
            static CONFINED: std::sync::Once = std::sync::Once::new();
            CONFINED.call_once(|| sb.assert_confined());
        }
        sb
    }

    /// Rustic looks for profiles in the Known Folder `AppData` first. It follows `USERPROFILE`, which the
    /// sandbox sets, so rustic never reads the developer's own profiles: one named like a sandbox's would
    /// send backups, prunes and recreations to a real repository. Checked rather than assumed.
    #[cfg(windows)]
    fn assert_confined(&self) {
        self.profile("confined", "");
        let out = self.rustic(&["-P", "confined", "show-config"]);
        let log = String::from_utf8_lossy(&out.stderr);
        let used = log.lines().find_map(|l| l.split("using config ").nth(1)).unwrap_or_default();
        assert!(
            Path::new(used.trim()).starts_with(&self.root),
            "rustic reads profiles outside the sandbox, so the tests could reach real repositories:\n{log}"
        );
        fs::remove_file(self.profile_path("confined")).unwrap();
    }

    /// Sets these variables in the `[global.env]` of the profile that every other one uses.
    pub(crate) fn common_env(&self, vars: &[(&str, &str)]) {
        let repository = format!("[repository]\nkey = '{KEY}'\ncache-dir = '{}'\n", self.path("cache").display());
        self.profile("common", &(repository + &global_env(vars)));
    }

    /// `rel`, with `/` between its parts, inside the sandbox. With Windows's separator there, as rustic records
    /// paths as given and Windows's own commands take no other.
    pub(crate) fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))
    }

    /// Where rustic looks for the sandbox user's collections. On Windows, the system's: rustic finds no
    /// Known Folder `AppData` in the sandbox.
    pub(crate) fn profile_dir(&self) -> PathBuf {
        if cfg!(windows) {
            self.path(r"programdata\rustic\config")
        } else if cfg!(target_os = "macos") {
            self.path("home/Library/Application Support/rustic")
        } else {
            self.path("config/rustic")
        }
    }

    pub(crate) fn profile_path(&self, name: &str) -> PathBuf {
        self.profile_dir().join(format!("{name}.toml"))
    }

    pub(crate) fn profile(&self, name: &str, toml: &str) {
        fs::write(self.profile_dir().join(format!("{name}.toml")), toml).unwrap();
    }

    /// A collection storing into `local/<name>`. Each entry of `groups` is one `[[backup.snapshots]]`
    /// with sources relative to `source/`.
    pub(crate) fn backup_profile(&self, name: &str, groups: &[&[&str]], targets: &[&str], extra: &str) {
        self.backup_profile_in(name, &format!("local/{name}"), groups, targets, extra);
    }

    /// Like [`Sandbox::collection`], with the repository at `repo` (relative to the sandbox).
    pub(crate) fn backup_profile_in(&self, name: &str, repo: &str, groups: &[&[&str]], targets: &[&str], extra: &str) {
        let mut toml = format!(
            "[global]\nuse-profiles = ['common']\n\n[repository]\nrepository = '{}'\n\n",
            self.path(repo).display()
        );
        for sources in groups {
            let sources: Vec<String> =
                sources.iter().map(|s| format!("'{}'", self.path(&format!("source/{s}")).display())).collect();
            toml += &format!("[[backup.snapshots]]\nsources = [{}]\n\n", sources.join(", "));
        }
        if !targets.is_empty() {
            let targets: Vec<String> = targets.iter().map(|t| format!("'{t}'")).collect();
            toml += &format!("[copy]\ntargets = [{}]\n\n", targets.join(", "));
        }
        toml += extra;
        self.profile(name, &toml);
    }

    /// Kills, with SIGKILL, what a killed run left running: its rustic, in its own process group,
    /// and what rustic started. Otherwise they would abort with a core dump at their next write to the
    /// unread pipes. Found by the sandbox's `HOME` in `/proc`, so Linux only.
    pub(crate) fn kill_leftovers(&self) {
        let home = format!("HOME={}", self.path("home").display());
        let pids: Vec<String> = fs::read_dir("/proc")
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
            .filter(|e| {
                fs::read(e.path().join("environ")).is_ok_and(|env| env.split(|b| *b == 0).any(|v| v == home.as_bytes()))
            })
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        if !pids.is_empty() {
            _ = Command::new("kill").arg("-KILL").args(&pids).status();
        }
    }

    /// The state, opened like another process would.
    pub(crate) fn store(&self) -> Store {
        Store::at(self.path("store")).unwrap()
    }

    /// Marks these steps (`check:Docs`, `prune:Docs`) as scheduled long ago, so that they are due now
    /// if they never ran, like a persistent systemd timer.
    pub(crate) fn scheduled_long_ago(&self, keys: &[&str]) {
        let (store, long_ago) = (self.store(), "2000-01-01T00:00:00Z".parse().unwrap());
        for key in keys.iter().map(|t| t.parse::<StepKey>().unwrap()) {
            store
                .apply(&[Event::Unscheduled { key: key.clone() }, Event::FirstSeen { keys: vec![key], at: long_ago }])
                .unwrap();
        }
    }

    /// A copy target storing into `remote/<dir>`.
    pub(crate) fn target(&self, name: &str, dir: &str, extra: &str) {
        self.profile(
            name,
            &format!(
                "[global]\nuse-profiles = ['common']\n\n[repository]\nrepository = '{}'\n\n{extra}",
                self.path(&format!("remote/{dir}")).display()
            ),
        );
    }

    /// Write a file below `source/`.
    pub(crate) fn write(&self, rel: &str, content: &str) {
        let path = self.path(&format!("source/{rel}"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    pub(crate) fn remove(&self, rel: &str) {
        let path = self.path(rel);
        if path.is_dir() {
            fs::remove_dir_all(path).unwrap();
        } else if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }

    pub(crate) fn command(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.env_clear()
            .env("PATH", env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.path("home"))
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("XDG_STATE_HOME", self.path("state"))
            .env(var("STATE_DIR"), self.path("store"))
            .env("TZ", "UTC")
            .current_dir(self.path("cwd"));
        // Windows's own, without which processes cannot start or reach the network, and the sandbox's
        // profile folders.
        if cfg!(windows) {
            for key in ["SystemRoot", "SystemDrive", "windir", "ComSpec", "PATHEXT", "NUMBER_OF_PROCESSORS"] {
                if let Some(value) = env::var_os(key) {
                    cmd.env(key, value);
                }
            }
            cmd.env("USERPROFILE", self.path("home"))
                .env("APPDATA", self.path(r"home\AppData\Roaming"))
                .env("LOCALAPPDATA", self.path(r"home\AppData\Local"))
                .env("PROGRAMDATA", self.path("programdata"))
                .env("TEMP", self.path("tmp"))
                .env("TMP", self.path("tmp"));
        }
        // `cargo llvm-cov`: where the child writes its coverage.
        if let Some(profile) = env::var_os("LLVM_PROFILE_FILE") {
            cmd.env("LLVM_PROFILE_FILE", profile);
        }
        cmd
    }

    /// In its own process group on Windows, which [`signal`] sends Ctrl-Break to.
    pub(crate) fn spawn(&self, args: &[&str], envs: &[(&str, &str)]) -> std::process::Child {
        let mut cmd = self.command(BIN);
        cmd.arg("--json")
            .args(args)
            .envs(envs.iter().copied())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(
            &mut cmd,
            windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP,
        );
        cmd.spawn().unwrap()
    }

    pub(crate) fn run(&self, args: &[&str]) -> Run {
        self.run_with(args, &[])
    }

    pub(crate) fn run_with(&self, args: &[&str], envs: &[(&str, &str)]) -> Run {
        let out = self.command(BIN).arg("--json").args(args).envs(envs.iter().copied()).output().unwrap();
        to_run(&out)
    }

    pub(crate) fn rustic(&self, args: &[&str]) -> Output {
        self.command("rustic").args(args).output().unwrap()
    }

    /// Snapshot times in a repository, oldest first (empty if it doesn't exist).
    pub(crate) fn snapshots(&self, profile: &str) -> Vec<String> {
        let mut times: Vec<String> = self.listed(profile, "time").into_iter().collect();
        times.sort();
        times
    }

    /// Field `field` of each snapshot in a repository (none if it doesn't exist).
    fn listed(&self, profile: &str, field: &str) -> Vec<String> {
        let out = self.rustic(&["-P", profile, "snapshots", "--json"]);
        if !out.status.success() {
            return Vec::new();
        }
        let groups: Value = serde_json::from_slice(&out.stdout).unwrap();
        let snapshots = groups.as_array().unwrap().iter().flat_map(|g| g["snapshots"].as_array().unwrap());
        snapshots.map(|s| s[field].as_str().unwrap().to_owned()).collect()
    }

    /// What the state keeps of each of `repos` for `collection`, which `collections` and the web UI show,
    /// is what the repository holds: its snapshots, and its size measured since a run last changed it.
    #[track_caller]
    pub(crate) fn assert_stored_matches(&self, collection: &str, repos: &[&str]) {
        let store = self.store();
        for repo in repos {
            let inventory = store.inventory(collection, repo).unwrap();
            // A failed listing panics with its error.
            let info = inventory.result.as_ref().unwrap();
            let stored: BTreeSet<String> = info.snapshots.iter().map(|s| s.id.to_string()).collect();
            assert_eq!(stored, self.listed(repo, "id").into_iter().collect(), "{repo}: snapshots");
            assert!(!inventory.size_outdated, "{repo}: the size is from before the last change");
            let out = self.rustic(&["-P", repo, "repoinfo", "--only-index", "--json"]);
            let blobs: Value = serde_json::from_slice::<Value>(&out.stdout).unwrap()["index"]["blobs"].take();
            let sum = |field: &str| blobs.as_array().unwrap().iter().map(|b| b[field].as_u64().unwrap_or(0)).sum();
            assert_eq!((info.stored, info.data), (Some(sum("size")), Some(sum("data_size"))), "{repo}: size");
        }
    }

    /// All files below `rel`, with their content.
    pub(crate) fn tree(&self, rel: &str) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(base, &path, out);
                } else {
                    out.insert(path.strip_prefix(base).unwrap().to_owned(), fs::read(&path).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        let base = self.path(rel);
        walk(&base, &base, &mut out);
        out
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        self.kill_leftovers();
        if std::thread::panicking() || env::var_os(var("KEEP_SANDBOX")).is_some() {
            eprintln!("sandbox kept at {}", self.root.display());
        } else {
            _ = fs::remove_dir_all(&self.root);
        }
    }
}

pub(crate) fn to_run(out: &Output) -> Run {
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    Run {
        code: out.status.code().unwrap_or(-1),
        output: format!("{stdout}{}", String::from_utf8_lossy(&out.stderr)),
        stdout,
    }
}

/// A collection `Docs` backing up `source/docs` (one file) to the target `Docs-remote`.
/// A `[forget]` policy, without which a repository is never pruned.
pub(crate) const KEEP: &str = "[forget]\nkeep-last = 10\n";

pub(crate) fn docs_with_remote(extra: &str) -> Sandbox {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-remote"], extra);
    sb.target("Docs-remote", "Docs", "");
    sb.write("docs/a.txt", "version 1");
    sb
}

/// Where `restore --to` puts `path`: `/home/me` under `home/me`, `C:\Users\me` under `C/Users/me`.
pub(crate) fn below_root(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(part) => out.push(part),
            Component::Prefix(prefix) => {
                out.extend(prefix.as_os_str().to_string_lossy().split(['\\', '/', ':', '?']).filter(|p| !p.is_empty()));
            }
            Component::RootDir | Component::CurDir | Component::ParentDir => {}
        }
    }
    out
}

/// Where `path` is in a snapshot's tree, as the web UI shows it: `/home/me` there too, `C:\Users\me` at
/// `/C/Users/me`.
#[cfg(feature = "server")]
pub(crate) fn in_tree(path: &Path) -> String {
    below_root(path).components().map(|c| format!("/{}", c.as_os_str().to_string_lossy())).collect()
}

/// `command` as a TOML string.
pub(crate) fn toml_str(command: &str) -> String {
    if command.contains('\'') {
        format!("\"{}\"", command.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        format!("'{command}'")
    }
}

/// Commands for rustic's hooks and stdin sources: Unix tools, or Windows's own. Rustic splits them as a POSIX
/// shell would, without running one, so Windows's paths are quoted to keep their `\\`. On Windows, links are
/// junctions, which need no administrator, and what the commands print is discarded, as Unix's print nothing.
pub(crate) mod cmd {
    use std::path::Path;

    fn quoted(path: &Path) -> String {
        format!("'{}'", path.display())
    }

    pub(crate) fn sleep(seconds: u32) -> String {
        if cfg!(windows) {
            format!("powershell -NoProfile -Command Start-Sleep -Seconds {seconds}")
        } else {
            format!("sleep {seconds}")
        }
    }

    /// Links `link` to the directory `target`, replacing a link there.
    pub(crate) fn link(target: &Path, link: &Path) -> String {
        let (target, link) = (quoted(target), quoted(link));
        if cfg!(windows) {
            format!("cmd /c (if exist {link} rmdir {link}) & mklink /J {link} {target} >nul")
        } else {
            format!("ln -sfn {target} {link}")
        }
    }

    pub(crate) fn unlink(link: &Path) -> String {
        let link = quoted(link);
        if cfg!(windows) { format!("cmd /c if exist {link} rmdir {link}") } else { format!("rm -f {link}") }
    }

    /// Writes the line `text` to `file`.
    pub(crate) fn write(text: &str, file: &Path) -> String {
        if cfg!(windows) {
            format!("cmd /c (echo {text})> {}", quoted(file))
        } else {
            format!("sh -c 'echo {text} > {}'", file.display())
        }
    }

    /// Moves the directory `from` into `into`, if it exists.
    pub(crate) fn move_dir(from: &Path, into: &Path) -> String {
        if cfg!(windows) {
            let (from, into) = (quoted(from), quoted(into));
            format!("cmd /c if exist {from} move {from} {into} >nul")
        } else {
            let (from, into) = (from.display(), into.display());
            format!("sh -c 'test -d {from} && mv {from} {into}; true'")
        }
    }
}

/// A collection whose backup takes `seconds`: its source is a slow stdin command. The command writes
/// nothing, because with a parent snapshot rustic 0.11 does not read stdin and a writer would die of
/// SIGPIPE.
pub(crate) fn slow_profile(sb: &Sandbox, name: &str, seconds: u32, extra: &str) {
    sb.profile(
        name,
        &format!(
            "[global]\nuse-profiles = ['common']\n[repository]\nrepository = '{}'\n\
             [[backup.snapshots]]\nsources = ['-']\nstdin-command = {}\n{extra}",
            sb.path(&format!("local/{name}")).display(),
            toml_str(&cmd::sleep(seconds))
        ),
    );
}

/// `len` bytes that neither compress nor deduplicate, from `seed`: rustic takes a while over them.
pub(crate) fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut seed = seed | 1;
    (0..len.div_ceil(8))
        .flat_map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed.to_le_bytes()
        })
        .take(len)
        .collect()
}

/// Polls `done` until it returns true, for at most `secs`.
#[track_caller]
#[expect(clippy::disallowed_methods, reason = "observe external processes over a bounded interval")]
pub(crate) fn wait_for(secs: u64, what: &str, mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while !done() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
