//! What a collection can be asked to do, by whom, and the steps a request runs.

use std::{fmt, path::PathBuf, str::FromStr};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::{
    i18n::t,
    rustic::{Reading, SnapshotId},
    settings::{self, Setting},
};

/// What a run applies to: every collection, one collection, or one repository of a collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Selection {
    All,
    Collection(String),
    Repo(String, String),
}

impl Selection {
    pub(crate) fn collection(&self) -> Option<&str> {
        match self {
            Self::All => None,
            Self::Collection(collection) | Self::Repo(collection, _) => Some(collection),
        }
    }
}

impl RunKind {
    /// What a run of it is called: a test restore is the `--dry-run` of `restore` that reads the snapshots.
    pub fn label(self) -> String {
        t!("run-kind-label", kind = <&str>::from(self))
    }

    /// `backing up`.
    pub fn doing(self) -> String {
        t!("run-kind-doing", kind = <&str>::from(self))
    }

    pub(crate) fn schedule(self) -> Option<&'static Setting> {
        match self {
            Self::Backup => Some(&settings::BACKUP),
            Self::Prune => Some(&settings::PRUNE),
            Self::Check => Some(&settings::CHECK),
            Self::Copy | Self::Restore | Self::TestRestore | Self::Recreate => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RestoreOptions {
    /// The repository to restore from. Unset: the one with the newest snapshots.
    pub from: Option<String>,
    pub at: Option<Timestamp>,
    /// One snapshot, instead of the newest of each source.
    pub snapshot: Option<SnapshotId>,
    /// One path in the snapshot.
    pub path: Option<String>,
    /// One `[[backup.snapshots]]` entry, by index.
    pub source: Option<usize>,
    pub to: Option<PathBuf>,
    /// Only paths that are absent or empty.
    pub missing: bool,
    /// Says what it would restore where, and restores nothing.
    #[serde(default)]
    pub dry_run: bool,
}

/// A step whose attempts the state records, as in `copy:Docs/usb`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, strum::IntoStaticStr)]
#[serde(into = "String", try_from = "String")]
#[strum(serialize_all = "lowercase")]
pub enum StepKey {
    /// A collection's snapshots of its sources, in its primary.
    Snapshot(String),
    /// A collection's copies to one of its targets.
    Copy(String, String),
    /// A collection's snapshots in one of its repositories.
    Forget(String, String),
    /// Removing unused data from a repository.
    Prune(String),
    /// Checking a repository.
    Check(String),
}

impl StepKey {
    /// The repository it works on: the primary for a snapshot.
    pub(crate) fn repo(&self) -> &str {
        match self {
            Self::Snapshot(repo)
            | Self::Prune(repo)
            | Self::Check(repo)
            | Self::Copy(_, repo)
            | Self::Forget(_, repo) => repo,
        }
    }
}

impl fmt::Display for StepKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind: &str = self.into();
        // Profile names cannot contain `/`.
        match self {
            Self::Snapshot(name) | Self::Prune(name) | Self::Check(name) => write!(f, "{kind}:{name}"),
            Self::Copy(collection, repo) | Self::Forget(collection, repo) => write!(f, "{kind}:{collection}/{repo}"),
        }
    }
}

impl FromStr for StepKey {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        let not_a_step = || NotAStep { step: s.to_owned() };
        let (kind, name) = s.split_once(':').ok_or_else(not_a_step)?;
        let pair = || name.split_once('/').map(|(a, b)| (a.to_owned(), b.to_owned()));
        Ok(match (kind, pair()) {
            ("snapshot", None) => Self::Snapshot(name.into()),
            ("copy", Some((collection, target))) => Self::Copy(collection, target),
            ("forget", Some((collection, repo))) => Self::Forget(collection, repo),
            ("prune", None) => Self::Prune(name.into()),
            ("check", None) => Self::Check(name.into()),
            _ => return Err(not_a_step().into()),
        })
    }
}

impl From<StepKey> for String {
    fn from(key: StepKey) -> Self {
        key.to_string()
    }
}

impl TryFrom<String> for StepKey {
    type Error = anyhow::Error;

    fn try_from(s: String) -> anyhow::Result<Self> {
        s.parse()
    }
}

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "request-error-not-a-step")]
pub(crate) struct NotAStep {
    step: String,
}

/// One run of a collection. Its [`RunKind`] is the variant alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, strum::EnumDiscriminants)]
#[strum_discriminants(
    name(RunKind),
    vis(pub),
    derive(PartialOrd, Ord, Serialize, Deserialize, strum::IntoStaticStr),
    serde(rename_all = "kebab-case"),
    strum(serialize_all = "kebab-case")
)]
#[serde(rename_all = "kebab-case")]
pub enum Request {
    /// Back up, then copy and forget.
    Backup,
    /// Copy to one target. The primary's name means every target.
    Copy(String),
    /// Prune these repositories, one step each: one, or every repository of the collection.
    Prune(Vec<String>),
    /// Check these repositories, one step each. A reading is set when the run takes the next data subset.
    Check(Vec<(String, Option<Reading>)>),
    Restore(RestoreOptions),
    /// Read a snapshot as a restore would, and write nothing. `None`: from the repository a restore reads.
    TestRestore(Option<String>, Tested),
    /// Copy a lost repository back from `from`. The primary comes from a target (unset: the most recent
    /// one); a target comes from the primary or another target.
    #[serde(rename = "re-create")]
    #[strum_discriminants(serde(rename = "re-create"))]
    #[strum_discriminants(strum(serialize = "re-create"))]
    Recreate {
        repo: String,
        from: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tested {
    /// The newest snapshot of each source, or of one `[[backup.snapshots]]` entry.
    Newest(Option<usize>),
    Snapshot(SnapshotId),
}

impl Request {
    pub fn kind(&self) -> RunKind {
        self.into()
    }

    /// The one repository it works on, if any.
    fn repo(&self) -> Option<&str> {
        match self {
            Request::Backup | Request::Restore(_) => None,
            Request::Copy(repo) | Request::Recreate { repo, .. } => Some(repo),
            Request::Prune(repos) => only(repos).map(String::as_str),
            Request::Check(repos) => only(repos).map(|(repo, _)| repo.as_str()),
            Request::TestRestore(repo, _) => repo.as_deref(),
        }
    }

    /// A prune or check of only the repositories that `keep` keeps.
    pub(crate) fn retain(&self, mut keep: impl FnMut(&str) -> bool) -> Request {
        match self {
            Request::Prune(repos) => Request::Prune(repos.iter().filter(|r| keep(r)).cloned().collect()),
            Request::Check(repos) => Request::Check(repos.iter().filter(|(r, _)| keep(r)).cloned().collect()),
            Request::Backup
            | Request::Copy(_)
            | Request::Restore(_)
            | Request::TestRestore(..)
            | Request::Recreate { .. } => self.clone(),
        }
    }

    /// The repositories a prune or check works on, one step each.
    pub(crate) fn maintained(&self) -> Vec<&str> {
        match self {
            Request::Prune(repos) => repos.iter().map(String::as_str).collect(),
            Request::Check(repos) => repos.iter().map(|(repo, _)| repo.as_str()).collect(),
            Request::Backup
            | Request::Copy(_)
            | Request::Restore(_)
            | Request::TestRestore(..)
            | Request::Recreate { .. } => Vec::new(),
        }
    }

    /// Whether it adds nothing to the same request queued before it, which would find what it finds. Not a
    /// check, which reads the next data subset, nor a restore, which may be repeated on purpose.
    pub fn joins(&self) -> bool {
        matches!(self, Request::Backup | Request::Copy(_) | Request::Prune(_))
    }

    /// Two requests with the same key run one after the other. It ignores the check reading, and names
    /// the request's lock.
    pub fn key(&self) -> String {
        let kind: &str = self.kind().into();
        self.repo().map_or_else(|| kind.to_owned(), |repo| format!("{kind}:{repo}"))
    }

    /// The repository it works on, or `None` when it applies to the whole collection.
    pub fn scope<'a>(&'a self, collection: &str) -> Option<&'a str> {
        // A copy from the primary goes to every target.
        self.repo().filter(|repo| !matches!(self, Request::Copy(_) if *repo == collection))
    }

    /// `verb` followed by the scope and reading: `backup collection`, `copy to Docs-usb`, `check Docs 3/10`.
    fn words(&self, collection: &str, verb: &str) -> String {
        let scope = match (self, self.scope(collection)) {
            (Request::Copy(_), Some(to)) => Scope::CopyTo { target: to }.to_string(),
            (Request::Copy(_), None) => Scope::CopyToAll.to_string(),
            (Request::TestRestore(..), Some(from)) => Scope::From { repo: from }.to_string(),
            (Request::Backup | Request::Prune(_) | Request::Check(_), None) => Scope::Collection.to_string(),
            (_, scope) => scope.unwrap_or_default().to_owned(),
        };
        let reading = if let Request::Check(repos) = self { only(repos).and_then(|(_, r)| r.as_ref()) } else { None };
        let reading = reading.map(ToString::to_string).unwrap_or_default();
        [verb, &scope, &reading].into_iter().filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
    }

    /// `checking Docs 3/10`, `copying to Docs-usb`.
    pub fn doing(&self, collection: &str) -> String {
        self.words(collection, &self.shown().doing())
    }

    /// `prune Docs`, `check Docs 3/10`, `restore (dry-run)`.
    pub fn label(&self, collection: &str) -> String {
        self.words(collection, &self.shown().label())
    }

    /// The kind its words name: a restore's dry run, like a test restore, is `restore (dry-run)`.
    fn shown(&self) -> RunKind {
        match self {
            Request::Restore(options) if options.dry_run => RunKind::TestRestore,
            Request::Backup
            | Request::Copy(_)
            | Request::Prune(_)
            | Request::Check(..)
            | Request::Restore(_)
            | Request::TestRestore(..)
            | Request::Recreate { .. } => self.kind(),
        }
    }
}

/// The one item of `items`, if it holds one.
fn only<T>(items: &[T]) -> Option<&T> {
    if let [item] = items { Some(item) } else { None }
}

/// The scope in a request's words.
#[derive(crate::i18n::Fluent)]
#[fluent(prefix = "request-scope")]
enum Scope<'a> {
    CopyTo {
        target: &'a str,
    },
    CopyToAll,
    From {
        repo: &'a str,
    },
    /// A backup's, or a prune's or check of every repository, as the others name a repository.
    Collection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::IntoStaticStr)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum Trigger {
    Cli,
    Schedule,
    Web,
}

impl Trigger {
    /// For `started by ...`.
    pub fn who(self) -> String {
        t!("trigger-who", trigger = <&str>::from(self))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Step {
    /// `rustic backup` into the primary.
    Snapshot {
        repo: String,
    },
    /// Copy the primary's snapshots that `to` lacks. Fails if the primary cannot be listed.
    Copy {
        to: String,
    },
    /// Apply `repo`'s `[forget]` policy. For a target, after its copy in this run succeeded. For the
    /// primary, after every copy succeeded, keeping snapshots that some target still lacks.
    Forget {
        repo: String,
    },
    Prune {
        repo: String,
    },
    Check {
        repo: String,
        reading: Reading,
    },
    /// Restore from the repository with the newest snapshots, unless `options` name one.
    Restore {
        options: RestoreOptions,
    },
    /// Read from `from` as a restore would, and write nothing. `None`: from the repository a restore reads.
    TestRestore {
        from: Option<String>,
        tested: Tested,
    },
    /// Re-create `repo` if it cannot be listed. Only an explicit request does this: backups and copies
    /// create a repository only if it never existed. `requested`: named by the request, so it says when
    /// there is nothing to re-create, unlike the primary checked before a target.
    Recreate {
        repo: String,
        from: Option<String>,
        requested: bool,
    },
}

impl Step {
    /// `snapshot to Docs`, `copy to usb`, `check usb 3/10`.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Snapshot { repo } => t!("step-snapshot", repo),
            Self::Copy { to } => t!("step-copy", to),
            Self::Forget { repo } => t!("step-forget", repo),
            Self::Prune { repo } => t!("step-prune", repo),
            Self::Check { repo, reading } => t!("step-check", repo, reading),
            Self::Restore { .. } => RunKind::Restore.label(),
            Self::TestRestore { from: Some(from), .. } => {
                format!("{} {}", RunKind::TestRestore.label(), Scope::From { repo: from })
            }
            Self::TestRestore { from: None, .. } => RunKind::TestRestore.label(),
            Self::Recreate { repo, from: None, .. } => t!("step-recreate", repo),
            Self::Recreate { repo, from: Some(from), .. } => t!("step-recreate-from", repo, from),
        }
    }

    /// The step whose attempts the state records, if any.
    pub(crate) fn key(&self, collection: &str) -> Option<StepKey> {
        let collection = || collection.to_owned();
        match self {
            Self::Snapshot { .. } => Some(StepKey::Snapshot(collection())),
            Self::Copy { to } => Some(StepKey::Copy(collection(), to.clone())),
            Self::Forget { repo } => Some(StepKey::Forget(collection(), repo.clone())),
            Self::Prune { repo } => Some(StepKey::Prune(repo.clone())),
            Self::Check { repo, .. } => Some(StepKey::Check(repo.clone())),
            Self::Restore { .. } | Self::TestRestore { .. } | Self::Recreate { .. } => None,
        }
    }

    /// The repository it works on, and what it does to it: `copying`. A restore names it only if chosen.
    pub(crate) fn on(&self) -> Option<(&str, String)> {
        match self {
            Self::Snapshot { repo } => Some((repo, RunKind::Backup.doing())),
            Self::Copy { to } => Some((to, RunKind::Copy.doing())),
            Self::Forget { repo } => Some((repo, t!("step-forgetting"))),
            Self::Prune { repo } => Some((repo, RunKind::Prune.doing())),
            Self::Check { repo, .. } => Some((repo, RunKind::Check.doing())),
            Self::Restore { options } => Some((options.from.as_deref()?, RunKind::Restore.doing())),
            Self::TestRestore { from, .. } => Some((from.as_deref()?, RunKind::TestRestore.doing())),
            Self::Recreate { repo, .. } => Some((repo, RunKind::Recreate.doing())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_keys_round_trip() {
        for key in [
            StepKey::Snapshot("Docs".into()),
            StepKey::Copy("Docs".into(), "Docs:usb".into()),
            StepKey::Forget("Docs".into(), "Docs".into()),
            StepKey::Prune("Docs-offsite".into()),
        ] {
            assert_eq!(key.to_string().parse::<StepKey>().unwrap(), key);
        }
        assert!("copy:Docs".parse::<StepKey>().is_err() && "unknown:x".parse::<StepKey>().is_err());
    }
}
