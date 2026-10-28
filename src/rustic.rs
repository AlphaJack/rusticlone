//! The `rustic` CLI: its exit codes and JSON output, and from its log only what is reported nowhere
//! else: why a command failed, and what a backup skipped (`[WARN] ignoring
//! error`). Copies are verified by listing the target, because rustic exits 0 when a target fails.

mod cli;
mod commands;
#[cfg(feature = "server")]
mod contents;
mod ctx;
mod install;
mod output;
mod process;
mod snapshot;

pub(crate) use cli::Cli;
pub(crate) use commands::{Reading, RestorePlan};
#[cfg(feature = "server")]
pub(crate) use contents::{Diff, Dirs, Found, NodeKind};
#[cfg(feature = "server")]
use contents::{diff, dirs, found};
pub(crate) use ctx::{Ctx, Origin, RunHandle, Stop};
#[cfg(any(feature = "self-update", all(windows, feature = "desktop")))]
pub(crate) use install::bin;
use install::minor;
pub(crate) use install::{TESTED, Untested};
#[cfg(feature = "self-update")]
pub(crate) use install::{downloaded, name, parse_version, pinned};
#[cfg(feature = "server")]
pub(crate) use output::Ended;
pub(crate) use output::{Failure, Output, Progress, Pruned};
#[cfg(feature = "server")]
pub(crate) use process::kill;
#[cfg(all(windows, feature = "server"))]
pub(crate) use process::tie;
#[cfg(test)]
mod replay;
#[cfg(test)]
pub(crate) use replay::Replay;
#[cfg(feature = "server")]
pub(crate) use snapshot::NotASnapshotId;
#[cfg(feature = "server")]
pub(crate) use snapshot::newest_summary;
#[cfg(feature = "server")]
pub(crate) use snapshot::short_id;
pub(crate) use snapshot::{ForgetGroup, RepoStats, Snapshot, SnapshotGroup, SnapshotId, path_filters, source_key};

use std::{ffi::OsStr, path::Path, sync::Arc, time::Duration};

use anyhow::Result;
use jiff::Timestamp;
pub(crate) use semver::Version;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::profile::RusticProfile;

/// A rustic command that a run executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Call {
    /// The index of the run's step that made it.
    pub step: Option<usize>,
    pub args: String,
    pub bin: Option<String>,
    pub dir: Option<String>,
    pub started: Timestamp,
    pub secs: f64,
    /// The exit code, or why the command ended without one. `None` while it runs.
    pub exit: Option<String>,
}

impl Call {
    pub fn command(&self) -> String {
        let bin = self.bin.as_deref().unwrap_or("rustic");
        let command = format!("{} {}", shell_word(OsStr::new(bin)), self.args);
        match &self.dir {
            Some(dir) => format!("cd {} && {command}", shell_word(OsStr::new(dir))),
            None => command,
        }
    }
}

fn shell_word(word: &OsStr) -> String {
    let word = word.to_string_lossy();
    if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || "_-./:=,@%+".contains(c)) {
        word.into_owned()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// The rustic commands that change a repository or the disk, which a dry run runs with `--dry-run`. Others
/// would only echo it, as `show-config` does.
const WRITES: [&str; 5] = ["backup", "copy", "forget", "prune", "restore"];

/// How often rustic reports progress, which is logged and shown in the web UI.
pub(crate) const PROGRESS: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "rustic-error")]
pub(crate) enum Error {
    DryRunCreates,
    #[fluent(key = crate::settings::RUSTIC.key, docs = install::DOCS)]
    CannotRun(#[source] std::io::Error),
    #[fluent(output = format!("{output:?}"))]
    NoVersion {
        output: String,
    },
    #[fluent(major = TESTED.major, minor = TESTED.minor)]
    TooOld {
        found: Version,
    },
    #[fluent(transparent)]
    Failed(Failure),
    NoSnapshot {
        id: String,
        profile: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    InvalidProfile {
        name: String,
        error: Failure,
    },
    Unreadable {
        command: String,
        version: Version,
        source: serde_json::Error,
    },
    #[fluent(transparent)]
    Stopped(Stop),
    #[fluent(key = crate::settings::STALL_TIMEOUT.key)]
    Stalled {
        command: String,
        silent: String,
    },
    NoStatus,
    NotPiped,
    Percent,
    NoSize,
    #[cfg(test)]
    NoAnswer {
        command: String,
    },
}

/// Runs rustic through a [`Runner`]: the real CLI ([`Cli`]), or recorded answers in tests.
#[derive(Clone)]
pub(crate) struct Rustic {
    runner: Arc<dyn Runner>,
    pub version: Version,
    /// Every call is a dry run, and none creates a repository: rustic's own dry run would.
    dry_run: bool,
}

/// A rustic command with profile `profile`, run in `dir`.
pub(crate) struct Invocation<'a> {
    pub profile: &'a str,
    pub dir: &'a Path,
    pub args: &'a [&'a str],
    pub stdout: Stdout,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stdout {
    Text,
    /// Counted and discarded, for `dump`.
    Count,
}

pub(crate) trait Runner: Send + Sync {
    /// Returns the output, and the bytes written with [`Stdout::Count`]. `progress` receives each
    /// progress report. Rustic is killed when the run is cancelled, or when it shows no activity for
    /// the stall timeout.
    fn run(
        &self,
        cx: &Ctx,
        call: &Invocation<'_>,
        progress: Option<&mut dyn FnMut(&Progress)>,
    ) -> Result<(Output, u64)>;

    /// A rustic process to stream a download from.
    #[cfg(feature = "server")]
    fn command(&self, profile: &str, dir: &Path) -> std::process::Command;
}

impl Rustic {
    /// Fails if rustic is older than [`TESTED`].
    pub(crate) fn new(stall_timeout: Duration, dry_run: bool) -> Result<Self> {
        let cli = Cli { stall_timeout };
        let version = Cli::version()?;
        if minor(&version) < minor(&TESTED) {
            return Err(Error::TooOld { found: version }.into());
        }
        Ok(Self { runner: Arc::new(cli), version, dry_run })
    }

    #[cfg(test)]
    pub(crate) fn with_runner(runner: Arc<dyn Runner>) -> Self {
        Self { runner, version: TESTED.clone(), dry_run: false }
    }

    pub(crate) fn untested(&self) -> Option<Untested> {
        (minor(&self.version) > minor(&TESTED)).then(|| Untested { found: self.version.clone() })
    }

    fn exec(
        &self,
        cx: &Ctx,
        profile: &str,
        dir: &Path,
        args: &[&str],
        progress: Option<&mut dyn FnMut(&Progress)>,
        stdout: Stdout,
    ) -> Result<(Output, u64)> {
        let dry: Vec<&str> = std::iter::once("--dry-run").chain(args.iter().copied()).collect();
        let writes = args.first().is_some_and(|command| WRITES.contains(command));
        let args = if self.dry_run && writes && !args.contains(&"--dry-run") { &dry } else { args };
        self.runner.run(cx, &Invocation { profile, dir, args, stdout }, progress)
    }

    /// Refuses to create a repository in a dry run.
    fn creates(&self) -> Result<()> {
        if self.dry_run { Err(Error::DryRunCreates.into()) } else { Ok(()) }
    }

    /// The output, or rustic's explanation as the error.
    fn run(
        &self,
        cx: &Ctx,
        p: &RusticProfile,
        args: &[&str],
        progress: Option<&mut dyn FnMut(&Progress)>,
    ) -> Result<Output> {
        ok(self.exec(cx, &p.name, &p.dir, args, progress, Stdout::Text)?.0)
    }

    /// Output that does not parse is reported with the rustic version.
    fn json<T: DeserializeOwned>(&self, cx: &Ctx, p: &RusticProfile, args: &[&str]) -> Result<T> {
        let out = self.run(cx, p, args, None)?;
        let unreadable =
            |source| Error::Unreadable { command: out.command.clone(), version: self.version.clone(), source };
        Ok(serde_json::from_str(&out.stdout).map_err(unreadable)?)
    }
}

fn ok(out: Output) -> Result<Output> {
    if out.success { Ok(out) } else { Err(Error::Failed(out.failure()).into()) }
}

/// The fixture `name` recorded with each rustic release in `tests/fixtures/rustic-*/`, with the
/// release. Parsers are tested with every release supported.
#[cfg(test)]
#[expect(clippy::disallowed_methods, reason = "tests read fixtures")]
pub(crate) fn fixtures(name: &str) -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    // One directory per rustic release; files are other fixtures.
    let mut recorded: Vec<_> =
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_dir()).collect();
    recorded.sort();
    let read = |d: std::path::PathBuf| {
        (d.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(d.join(name)).unwrap())
    };
    recorded.into_iter().map(read).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_arguments_preserve_spaces_and_metacharacters() {
        for (word, expected) in [
            ("Docs", "Docs"),
            ("", "''"),
            ("/home/jack/My Docs", "'/home/jack/My Docs'"),
            ("Jack's files", "'Jack'\\''s files'"),
            ("$HOME; echo test", "'$HOME; echo test'"),
        ] {
            assert_eq!(shell_word(OsStr::new(word)), expected);
        }
    }
}
