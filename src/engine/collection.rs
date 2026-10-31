//! A collection: a backup profile and its copy targets.

use anyhow::Result;

use super::Error;
use crate::{
    profile::RusticProfile,
    request::{Request, RestoreOptions, RunKind, Tested},
};

pub(crate) struct Collection<'a> {
    pub primary: &'a RusticProfile,
    /// What it is called: see [`title`].
    pub title: &'a str,
    pub targets: Vec<&'a RusticProfile>,
    pub missing_targets: Vec<String>,
}

impl<'a> Collection<'a> {
    /// The runs of `kind` on the collection, or on one of its repositories. A prune or check of the collection is one
    /// run, with a step per repository. A copy from the primary goes to every target. `options`:
    /// what a restore or test restore reads, and where a restore writes.
    pub(crate) fn requests(
        &self,
        kind: RunKind,
        repo: Option<&RusticProfile>,
        options: &RestoreOptions,
    ) -> Result<Vec<Request>> {
        let collection = self;
        let named = |p: Option<&RusticProfile>| p.unwrap_or(collection.primary).name.clone();
        // One repository, or every one, each a step of the run.
        let names: Vec<String> =
            repo.map_or_else(|| collection.repos().collect(), |p| vec![p]).iter().map(|p| p.name.clone()).collect();
        Ok(match kind {
            RunKind::Backup if repo.is_some() => return Err(Error::BackupOfRepository.into()),
            RunKind::Backup => vec![Request::Backup],
            RunKind::Copy => {
                if let Some(why) = collection.copy_blocked() {
                    return Err(Error::Blocked(why).into());
                }
                vec![Request::Copy(named(repo))]
            }
            RunKind::Prune => vec![Request::Prune(names)],
            RunKind::Check => vec![Request::Check(names.into_iter().map(|name| (name, None)).collect())],
            RunKind::Restore => {
                vec![Request::Restore(RestoreOptions { from: repo.map(|p| p.name.clone()), ..options.clone() })]
            }
            RunKind::TestRestore => {
                let tested = options.snapshot.clone().map_or(Tested::Newest(options.source), Tested::Snapshot);
                vec![Request::TestRestore(repo.map(|p| p.name.clone()), tested)]
            }
            RunKind::Recreate => vec![Request::Recreate { repo: named(repo), from: options.from.clone() }],
        })
    }

    pub(crate) fn name(&self) -> &str {
        &self.primary.name
    }

    /// The primary, then the targets.
    pub(crate) fn repos(&self) -> impl Iterator<Item = &'a RusticProfile> + '_ {
        std::iter::once(self.primary).chain(self.targets.iter().copied())
    }

    /// The `[[backup.snapshots]]` entry that backs up `path`, by its index.
    pub(crate) fn source_index(&self, path: &str) -> Option<usize> {
        self.primary.sources.iter().position(|s| s.paths().iter().chain(&s.sources).any(|p| p == path))
    }

    pub(crate) fn repo(&self, name: &str) -> Option<&'a RusticProfile> {
        self.repos().find(|p| p.name == name)
    }

    pub(crate) fn repo_named(&self, name: &str) -> Result<&'a RusticProfile> {
        Ok(self
            .repo(name)
            .ok_or_else(|| Error::NotARepository { name: name.to_owned(), collection: self.title.to_owned() })?)
    }

    pub(crate) fn copy_blocked(&self) -> Option<Blocked> {
        if self.targets.is_empty() {
            Some(Blocked::NoTargets)
        } else if !self.primary.exists() {
            Some(Blocked::PrimaryMissing)
        } else {
            None
        }
    }
}

/// What separates the words of a profile's name.
const SEPARATORS: [char; 4] = ['-', '_', ' ', '.'];

/// Whether `name` is `start` followed by a separator.
pub(super) fn continues(name: &str, start: &str) -> bool {
    name.strip_prefix(start).is_some_and(|rest| rest.starts_with(SEPARATORS))
}

/// What the collection of `primary` is called: the start of its name, up to a separator, that every copy
/// target continues and no other profile does. `Documents-usb` copying to `Documents-s3` is `Documents`.
/// Otherwise the backup profile's name, which stays the collection's key: a title changes with targets.
pub(crate) fn title<'a>(primary: &'a str, targets: &[String], profiles: &[String]) -> &'a str {
    let ours = |name: &String| name == primary || targets.contains(name);
    let shared = |start: &&str| !start.is_empty() && !targets.is_empty() && targets.iter().all(|t| continues(t, start));
    let mut starts = primary.match_indices(SEPARATORS).rev().filter_map(|(i, _)| primary.get(..i));
    match std::iter::once(primary).chain(&mut starts).find(shared) {
        Some(start) if profiles.iter().filter(|n| *n == start || continues(n, start)).all(ours) => start,
        _ => primary,
    }
}

/// Why a collection cannot copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, crate::i18n::Fluent)]
pub(crate) enum Blocked {
    NoTargets,
    PrimaryMissing,
}

#[cfg(test)]
mod tests {
    use super::title;

    #[test]
    fn a_title_is_the_start_that_only_the_collection_shares() {
        let names = |n: &[&str]| n.iter().map(ToString::to_string).collect::<Vec<_>>();
        let title = |primary, targets: &[&str], others: &[&str]| {
            let profiles = names(&[&[primary], targets, others].concat());
            title(primary, &names(targets), &profiles).to_owned()
        };
        assert_eq!(title("Documents", &["Documents-s3"], &[]), "Documents");
        assert_eq!(title("Documents-usb", &["Documents-s3"], &[]), "Documents");
        assert_eq!(title("Documents_usb", &["Documents s3"], &[]), "Documents");
        assert_eq!(title("home-docs-usb", &["home-docs-s3", "home-docs-b2"], &[]), "home-docs");
        assert_eq!(title("Documents-usb", &[], &[]), "Documents-usb");
        assert_eq!(title("Photos", &["offsite"], &[]), "Photos");
        assert_eq!(title("Documents-usb", &["Documents-s3", "offsite"], &[]), "Documents-usb");
        assert_eq!(title("Doc-usb", &["Documents-s3"], &[]), "Doc-usb");
        assert_eq!(title("Documents-usb", &["Documents-s3"], &["Documents-nas"]), "Documents-usb");
        assert_eq!(title("Documents-usb", &["Documents", "Documents-s3"], &[]), "Documents-usb");
        assert_eq!(title("Documents-usb", &["Documents-s3"], &["Documents"]), "Documents-usb");
    }
}
