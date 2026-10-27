//! Snapshots and repository statistics, as rustic prints them.

use std::fmt;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// Validated as hexadecimal, so that rustic never reads it as an option.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SnapshotId(String);

impl SnapshotId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn short(&self) -> &str {
        short_id(&self.0)
    }
}

/// The characters of an id that rustic shows.
const SHORT_ID: usize = 8;
/// Rustic accepts a prefix of at least 4 characters, up to the whole 64.
const ID_LENGTH: std::ops::RangeInclusive<usize> = 4..=64;

/// The first characters of an id, as rustic shows them.
pub(crate) fn short_id(id: &str) -> &str {
    id.get(..SHORT_ID).unwrap_or(id)
}

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "rustic-error-not-a-snapshot-id")]
pub struct NotASnapshotId {
    id: String,
}

impl TryFrom<String> for SnapshotId {
    type Error = NotASnapshotId;

    fn try_from(id: String) -> Result<Self, NotASnapshotId> {
        match ID_LENGTH.contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit()) {
            true => Ok(Self(id)),
            false => Err(NotASnapshotId { id }),
        }
    }
}

impl std::str::FromStr for SnapshotId {
    type Err = NotASnapshotId;

    fn from_str(id: &str) -> Result<Self, NotASnapshotId> {
        id.to_owned().try_into()
    }
}

impl From<SnapshotId> for String {
    fn from(id: SnapshotId) -> Self {
        id.0
    }
}

impl fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: SnapshotId,
    pub time: Timestamp,
    pub paths: Vec<String>,
    #[serde(default)]
    pub hostname: String,
    /// The snapshot it was first copied from. Copies keep it.
    pub original: Option<SnapshotId>,
    pub summary: Option<Summary>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Summary {
    pub total_files_processed: u64,
    pub total_bytes_processed: u64,
    pub data_added_packed: u64,
    /// In seconds.
    pub backup_duration: f64,
}

/// The newest snapshot of each source, summed: what the last backup of every source holds.
#[cfg(feature = "server")]
pub(crate) fn newest_summary(snapshots: &[Snapshot]) -> Option<Summary> {
    let mut newest: std::collections::BTreeMap<String, &Snapshot> = std::collections::BTreeMap::new();
    for sn in snapshots {
        let kept = newest.entry(sn.source()).or_insert(sn);
        if sn.time > kept.time {
            *kept = sn;
        }
    }
    newest.values().filter_map(|sn| sn.summary.as_ref()).fold(None, |sum, s| {
        let sum = sum.unwrap_or_default();
        Some(Summary {
            total_files_processed: sum.total_files_processed + s.total_files_processed,
            total_bytes_processed: sum.total_bytes_processed + s.total_bytes_processed,
            data_added_packed: sum.data_added_packed + s.data_added_packed,
            backup_duration: sum.backup_duration + s.backup_duration,
        })
    })
}

/// `repoinfo --only-index --json`: from the cached index, without listing the repository's files,
/// which is slow on a remote.
#[derive(Deserialize)]
pub(crate) struct RepoStats {
    index: Option<RepoIndex>,
}

#[derive(Deserialize)]
struct RepoIndex {
    blobs: Vec<Sized>,
}

#[derive(Deserialize)]
struct Sized {
    size: u64,
    #[serde(default)]
    data_size: u64,
}

impl RepoStats {
    /// The packed data size. The repository is slightly larger (about 0.1%): pack headers, and the
    /// config, key, index and snapshot files.
    pub(crate) fn stored(&self) -> u64 {
        self.index.iter().flat_map(|i| &i.blobs).map(|b| b.size).sum()
    }

    /// Before compression.
    pub(crate) fn data(&self) -> u64 {
        self.index.iter().flat_map(|i| &i.blobs).map(|b| b.data_size).sum()
    }
}

impl Snapshot {
    /// The `[[backup.snapshots]]` entry it belongs to.
    pub fn source(&self) -> String {
        source_key(&self.paths)
    }

    /// The same in every repository, while `copy` gives each copy a new id.
    pub fn identity(&self) -> &SnapshotId {
        self.original.as_ref().unwrap_or(&self.id)
    }

    pub fn same(&self, other: &Snapshot) -> bool {
        self.identity() == other.identity()
    }
}

/// `--filter-paths-exact` for each group, to list only a profile's snapshots.
pub(crate) fn path_filters(groups: impl IntoIterator<Item = impl AsRef<[String]>>) -> Vec<String> {
    groups.into_iter().flat_map(|g| ["--filter-paths-exact".to_owned(), g.as_ref().join(",")]).collect()
}

/// Identifies a source by its paths, in any order.
pub(crate) fn source_key(paths: &[String]) -> String {
    let mut paths = paths.to_vec();
    paths.sort_unstable();
    paths.dedup();
    paths.join(",")
}

/// Snapshots as rustic groups them (`group-by`).
#[derive(Deserialize)]
pub(crate) struct SnapshotGroup {
    pub group_key: GroupKey,
    pub snapshots: Vec<Snapshot>,
}

#[derive(Deserialize)]
pub(crate) struct GroupKey {
    /// Unset if the group spans hosts (`group-by` without `host`).
    pub hostname: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ForgetGroup {
    pub items: Vec<ForgetItem>,
}

#[derive(Deserialize)]
pub(crate) struct ForgetItem {
    pub snapshot: Snapshot,
    pub keep: bool,
    /// Why it is kept: `last`, `daily`, ..., or `id` for `--keep-id`.
    #[serde(default)]
    pub reasons: Vec<String>,
}
