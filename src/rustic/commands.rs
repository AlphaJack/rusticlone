//! The rustic commands that are run. Their arguments are defined only here, and their output
//! is parsed into values. A failed command returns rustic's explanation as the error.

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::{
    Ctx, Error, Failure, ForgetGroup, Progress, Pruned, RepoStats, Rustic, Snapshot, SnapshotGroup, SnapshotId, Stdout,
    ok, path_filters,
};
use crate::profile::RusticProfile;

/// Matches no snapshot. `copy --init` with it creates the target with the source's chunker.
/// `rustic init` cannot do that: it picks a random chunker, and copies need matching ones.
const NOTHING: &str = "/nonexistent/nothing";

/// What a backup saved, and the paths it could not read, with the reason.
pub(crate) struct Backup {
    pub snapshots: Vec<SnapshotId>,
    /// See [`super::Output::not_backed_up`]. Rustic then exits 1.
    pub not_backed_up: Vec<(String, String)>,
    pub skipped: Vec<(String, String)>,
    /// See [`super::Output::unreadable_parent`].
    pub unreadable_parent: Option<(String, String)>,
}

/// What `backup --dry-run` would save, after the backup's hooks ran.
pub(crate) struct BackupPlan {
    pub snapshots: Vec<PlannedSnapshot>,
    /// See [`super::Output::not_backed_up`].
    pub not_backed_up: Vec<(String, String)>,
}

/// A snapshot that `backup --dry-run` would save.
pub(crate) struct PlannedSnapshot {
    pub paths: Vec<String>,
    pub files: u64,
    pub bytes: u64,
}

/// A snapshot as `backup --dry-run --json` prints it, without an id.
#[derive(Deserialize)]
struct Planned {
    paths: Vec<String>,
    #[serde(default)]
    summary: Option<super::snapshot::Summary>,
}

/// The file counts of `restore --dry-run`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestorePlan {
    pub create: u64,
    pub replace: u64,
    /// Unchanged, or verified to be identical.
    pub keep: u64,
    /// Present and not in the snapshot. A restore leaves them.
    pub additional: u64,
}

/// What a check reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reading {
    /// The index, and that every pack it lists exists.
    Structure,
    /// Also data subset `n` of `of` (`check --read-data-subset n/of`).
    Subset { n: u32, of: u32 },
    /// Also random packs, as `check --read-data-subset` selects for `5%`, `2GiB` or `all`.
    Sample(Sample),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Sample {
    /// In hundredths of a percent: 12.5% is 1250.
    Percent(u32),
    Bytes(u64),
    All,
}

impl std::str::FromStr for Sample {
    type Err = anyhow::Error;

    /// `all`, `12.5%`, `2GiB`, as rustic accepts them.
    fn from_str(s: &str) -> Result<Self> {
        if s == "all" {
            return Ok(Self::All);
        }
        if let Some(percent) = s.strip_suffix('%') {
            let percent: f64 = percent.trim().parse()?;
            if !(percent > 0.0 && percent <= 100.0) {
                return Err(Error::Percent.into());
            }
            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "at most 10000")]
            return Ok(Self::Percent((percent * 100.0).round() as u32));
        }
        let bytes = s.parse::<bytesize::ByteSize>().map_err(anyhow::Error::msg)?.as_u64();
        if bytes == 0 {
            return Err(Error::NoSize.into());
        }
        Ok(Self::Bytes(bytes))
    }
}

impl Reading {
    /// What a check reads: `data subset 3/10`.
    pub(crate) fn what(self) -> String {
        let kind = match self {
            Self::Structure => "structure",
            Self::Subset { .. } => "subset",
            Self::Sample(Sample::All) => "all",
            Self::Sample(_) => "sample",
        };
        crate::i18n::t!("reading-what", kind, reading = self)
    }

    /// The value of `check --read-data-subset`.
    fn arg(self) -> String {
        if let Self::Sample(Sample::Bytes(bytes)) = self { bytes.to_string() } else { self.to_string() }
    }
}

impl std::fmt::Display for Reading {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Structure => write!(f, "structure"),
            Self::Subset { n, of } => write!(f, "{n}/{of}"),
            Self::Sample(Sample::Percent(hundredths)) => {
                let percent = format!("{}.{:02}", hundredths / 100, hundredths % 100);
                write!(f, "{}%", percent.trim_end_matches('0').trim_end_matches('.'))
            }
            Self::Sample(Sample::Bytes(bytes)) => write!(f, "{}", crate::fmt::human_size(*bytes)),
            Self::Sample(Sample::All) => write!(f, "all"),
        }
    }
}

impl Rustic {
    /// The merged configuration of profile `name`, found from `dir`.
    pub(crate) fn config(&self, name: &str, dir: &Path) -> Result<crate::profile::Config> {
        let out = self.exec(&Ctx::default(), name, dir, &["show-config"], None, Stdout::Text)?.0;
        if !out.success {
            return Err(Error::InvalidProfile { name: name.to_owned(), error: out.failure() }.into());
        }
        Ok(toml_edit::de::from_str(&out.stdout)?)
    }

    /// One snapshot, without path filters.
    pub(crate) fn snapshot(&self, cx: &Ctx, p: &RusticProfile, id: &SnapshotId) -> Result<Snapshot> {
        // The snapshot file does not contain its own id.
        let mut snapshot: serde_json::Value = (self.json(cx, p, &["cat", "snapshot", id.as_str()]))
            .map_err(|e| Error::NoSnapshot { id: id.as_str().to_owned(), profile: p.name.clone(), source: e.into() })?;
        snapshot["id"] = id.as_str().into();
        Ok(serde_json::from_value(snapshot)?)
    }

    /// The snapshots of these sources, grouped by `group_by`.
    pub(crate) fn snapshots(
        &self,
        cx: &Ctx,
        p: &RusticProfile,
        sources: impl IntoIterator<Item = impl AsRef<[String]>>,
        group_by: &str,
    ) -> Result<Vec<SnapshotGroup>> {
        let filters = path_filters(sources);
        let mut args = vec!["snapshots", "--json", "--group-by", group_by];
        args.extend(filters.iter().map(String::as_str));
        self.json(cx, p, &args)
    }

    /// From the locally cached index, without listing the repository's files.
    pub(crate) fn stats(&self, cx: &Ctx, p: &RusticProfile) -> Result<RepoStats> {
        self.json(cx, p, &["repoinfo", "--only-index", "--json"])
    }

    /// `init`: create the repository first.
    pub(crate) fn backup(
        &self,
        cx: &Ctx,
        p: &RusticProfile,
        init: bool,
        progress: &mut dyn FnMut(&Progress),
    ) -> Result<Backup> {
        if init {
            self.creates()?;
        }
        let args: &[&str] = if init { &["backup", "--json", "--init"] } else { &["backup", "--json"] };
        let out = self.exec(cx, &p.name, &p.dir, args, Some(progress), Stdout::Text)?.0;
        let not_backed_up = out.not_backed_up();
        // A failure that no source explains is rustic's own.
        let out = if not_backed_up.is_empty() { ok(out)? } else { out };
        Ok(Backup {
            snapshots: out.summary_snapshot_ids(),
            not_backed_up,
            skipped: out.skipped(),
            unreadable_parent: out.unreadable_parent(),
        })
    }

    /// What a backup would save of each source, as rustic sees it once its hooks prepared the sources.
    /// Saves nothing.
    pub(crate) fn backup_plan(&self, cx: &Ctx, p: &RusticProfile) -> Result<BackupPlan> {
        let out = self.exec(cx, &p.name, &p.dir, &["backup", "--dry-run", "--json"], None, Stdout::Text)?.0;
        let not_backed_up = out.not_backed_up();
        let out = if not_backed_up.is_empty() { ok(out)? } else { out };
        // One JSON document per snapshot, one after the other.
        let planned = serde_json::Deserializer::from_str(&out.stdout).into_iter::<Planned>();
        let snapshots = planned.map(|s| {
            s.map(|s| {
                let (files, bytes) = s.summary.map_or((0, 0), |m| (m.total_files_processed, m.total_bytes_processed));
                PlannedSnapshot { paths: s.paths, files, bytes }
            })
        });
        Ok(BackupPlan { snapshots: snapshots.collect::<Result<_, _>>()?, not_backed_up })
    }

    /// Copies `ids` from `from` to the profile `to`. With no ids, it only creates `to` (`init`) with
    /// `from`'s chunker. Rustic exits 0 even when the target fails, so the caller must list the target.
    /// Returns rustic's own error text, to explain missing snapshots.
    pub(crate) fn copy(
        &self,
        cx: &Ctx,
        from: &RusticProfile,
        ids: &[&SnapshotId],
        to: &str,
        init: bool,
        progress: &mut dyn FnMut(&Progress),
    ) -> Result<Failure> {
        let mut args = vec!["copy"];
        args.extend(ids.iter().map(|id| id.as_str()));
        if ids.is_empty() {
            args.extend(["--filter-paths-exact", NOTHING]);
        }
        args.extend(["--target", to]);
        if init {
            self.creates()?;
            args.push("--init");
        }
        Ok(self.exec(cx, &from.name, &from.dir, &args, Some(progress), Stdout::Text)?.0.failure())
    }

    /// Applies the repository's `[forget]` policy to these groups, always keeping `held`.
    pub(crate) fn forget(
        &self,
        cx: &Ctx,
        p: &RusticProfile,
        groups: impl IntoIterator<Item = impl AsRef<[String]>>,
        held: &[&SnapshotId],
    ) -> Result<Vec<ForgetGroup>> {
        let filters = path_filters(groups);
        let mut args = vec!["forget", "--json"];
        args.extend(filters.iter().map(String::as_str));
        args.extend(held.iter().flat_map(|id| ["--keep-id", id.as_str()]));
        self.json(cx, p, &args)
    }

    pub(crate) fn repo_id(&self, cx: &Ctx, p: &RusticProfile) -> Result<String> {
        #[derive(Deserialize)]
        struct Config {
            id: String,
        }
        Ok(self.json::<Config>(cx, p, &["cat", "config"])?.id)
    }

    /// Creates an empty repository. Only for a repository that no copy needs a matching chunker for.
    pub(crate) fn init(&self, cx: &Ctx, p: &RusticProfile) -> Result<()> {
        self.creates()?;
        self.run(cx, p, &["init"], None).map(drop)
    }

    /// The removed and remaining sizes, if rustic reported them.
    pub(crate) fn prune(
        &self,
        cx: &Ctx,
        p: &RusticProfile,
        progress: &mut dyn FnMut(&Progress),
    ) -> Result<Option<Pruned>> {
        let out = self.run(cx, p, &["prune"], Some(progress))?;
        Ok(out.pruned())
    }

    pub(crate) fn check(
        &self,
        cx: &Ctx,
        p: &RusticProfile,
        reading: Reading,
        progress: &mut dyn FnMut(&Progress),
    ) -> Result<()> {
        let subset = reading.arg();
        let args: &[&str] = match reading {
            Reading::Structure => &["check"],
            Reading::Subset { .. } | Reading::Sample(_) => &["check", "--read-data", "--read-data-subset", &subset],
        };
        self.run(cx, p, args, Some(progress)).map(drop)
    }

    /// Restores `path` of snapshot `id` to `dest`. Changed files are replaced, comparing contents
    /// because ransomware can keep mtime and size. No file is deleted.
    pub(crate) fn restore(
        &self,
        cx: &Ctx,
        p: &RusticProfile,
        id: &SnapshotId,
        path: &str,
        dest: &Path,
        progress: &mut dyn FnMut(&Progress),
    ) -> Result<()> {
        let (spec, dest) = (format!("{id}:{path}"), dest.to_string_lossy());
        let args = ["restore", "--verify-existing", &spec, &dest];
        self.run(cx, p, &args, Some(progress)).map(drop)
    }

    /// What restoring `path` of snapshot `id` in place would do, without writing. `None` if rustic
    /// reported no counts.
    pub(crate) fn restore_plan(
        &self,
        cx: &Ctx,
        p: &RusticProfile,
        id: &SnapshotId,
        path: &str,
        progress: &mut dyn FnMut(&Progress),
    ) -> Result<Option<RestorePlan>> {
        let spec = format!("{id}:{path}");
        let args = ["restore", "--dry-run", "--verify-existing", &spec, path];
        let out = self.run(cx, p, &args, Some(progress))?;
        Ok(restore_plan(&out.stdout))
    }

    /// Reads the whole snapshot `id` as a tar archive, and returns its size.
    pub(crate) fn dump_size(&self, cx: &Ctx, p: &RusticProfile, id: &SnapshotId) -> Result<u64> {
        let (out, bytes) =
            self.exec(cx, &p.name, &p.dir, &["dump", "--archive", "tar", id.as_str()], None, Stdout::Count)?;
        ok(out).map(|_| bytes)
    }

    /// All directories of a snapshot, in one call.
    #[cfg(feature = "server")]
    pub(crate) fn ls(&self, p: &RusticProfile, id: &SnapshotId) -> Result<super::Dirs> {
        let out = self.run(&Ctx::default(), p, &["ls", "--long", "--recursive", id.as_str()], None)?;
        Ok(super::dirs(&out.stdout))
    }

    /// Snapshots of these groups with a path that contains `name`, ignoring case.
    #[cfg(feature = "server")]
    pub(crate) fn find(
        &self,
        p: &RusticProfile,
        groups: impl IntoIterator<Item = impl AsRef<[String]>>,
        name: &str,
    ) -> Result<Vec<super::Found>> {
        let (glob, filters) = (format!("--iglob=*{name}*"), path_filters(groups));
        let mut args = vec!["find", glob.as_str()];
        args.extend(filters.iter().map(String::as_str));
        Ok(super::found(&self.run(&Ctx::default(), p, &args, None)?.stdout))
    }

    #[cfg(feature = "server")]
    pub(crate) fn diff(&self, p: &RusticProfile, from: &SnapshotId, to: &SnapshotId) -> Result<super::Diff> {
        let args = ["diff", from.as_str(), to.as_str()];
        Ok(super::diff(&self.run(&Ctx::default(), p, &args, None)?.stdout))
    }

    /// A command that streams `path` of snapshot `id`: a file as it is, or a zip archive. An empty
    /// `path` means the whole snapshot.
    #[cfg(feature = "server")]
    pub(crate) fn dump(&self, p: &RusticProfile, id: &SnapshotId, path: &str, zip: bool) -> std::process::Command {
        let spec = if path.is_empty() { id.to_string() } else { format!("{id}:{path}") };
        let archive = if zip { "zip" } else { "content" };
        let mut command = self.runner.command(&p.name, &p.dir);
        command.args(["dump", "--archive", archive, &spec]);
        command
    }
}

/// Rustic's counts of files that a restore writes.
const TO_RESTORE: &str = "to restore";
const TO_MODIFY: &str = "to modify";

/// Parses `Files:  1 to restore, 2 unchanged, 1 verified, 3 to modify, 4 additional`.
fn restore_plan(stdout: &str) -> Option<RestorePlan> {
    let files = stdout.lines().find_map(|l| l.trim().strip_prefix("Files:"))?;
    let count = |what: &str| -> u64 {
        files.split(',').find_map(|part| part.trim().strip_suffix(what)?.trim().parse().ok()).unwrap_or(0)
    };
    Some(RestorePlan {
        create: count(TO_RESTORE),
        replace: count(TO_MODIFY),
        keep: count("unchanged") + count("verified"),
        additional: count("additional"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_as_rustic_takes_them() {
        let sample = |s: &str| s.parse::<Sample>().ok().map(Reading::Sample).map(|r| (r.to_string(), r.arg()));
        assert_eq!(sample("12.5%"), Some(("12.5%".into(), "12.5%".into())));
        assert_eq!(sample("5%"), Some(("5%".into(), "5%".into())));
        assert_eq!(sample("2GiB"), Some(("2.0 GiB".into(), "2147483648".into())));
        assert_eq!(sample("all"), Some(("all".into(), "all".into())));
        for wrong in ["0%", "101%", "0", "lots", ""] {
            assert_eq!(sample(wrong), None, "{wrong}");
        }
    }

    #[test]
    fn restore_plan_counts() {
        for (_, out) in super::super::fixtures("restore-dry-run.stdout") {
            insta::assert_debug_snapshot!("restore_dry_run", restore_plan(&out));
        }
        assert_eq!(restore_plan(""), None);
    }
}
