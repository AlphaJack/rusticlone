//! Prunes and checks, one repository at a time.

use anyhow::Result;

use super::{Collection, Engine, Error, Run, Use, behind};
use crate::{
    profile::RusticProfile,
    request::{Request, RunKind, Step},
    result::StepResult,
    rustic::Reading,
    settings,
    state::{Event, Read},
};

impl Engine {
    /// Runs a prune or check step on `repo`.
    pub(super) fn maintain(&self, run: &Run<'_>, repo: &RusticProfile, step: &Step) {
        let collection = run.collection;
        let started = self.store.now();
        let reading = if let Step::Check { reading, .. } = step { Some(*reading) } else { None };
        if !repo.exists() {
            if let Some(reading) = reading {
                self.check_read(collection, &repo.name, reading, Read::Not);
            }
            // Reported like a backup of a missing source. It is not recorded as an attempt, so the status
            // reports it only once, as missing.
            let kind = if reading.is_some() { RunKind::Check } else { RunKind::Prune };
            return run.push(StepResult::MissingSkipped { run: kind, repo: repo.name.clone() });
        }
        // The recorded lag may be old: measure it again.
        if reading.is_none()
            && behind(&self.store.load(), collection, repo)
            && let Ok(primary) = self.list(run, collection.primary)
        {
            let latest = self.list(run, repo).ok().map(|s| self.covered(collection, repo, &primary, &s));
            self.record_lag(run, repo, &primary, latest.as_ref());
        }
        // An append-only repository must keep every snapshot, so it has nothing to prune.
        if reading.is_none() && !repo.has_keep {
            return run.push(StepResult::KeepsEverything { repo: repo.name.clone() });
        }
        if reading.is_none() && behind(&self.store.load(), collection, repo) {
            // A prune would discard the data of a partially copied snapshot.
            return run.push(StepResult::PrunePostponed { repo: repo.name.clone() });
        }
        let result = (|| -> Result<StepResult> {
            // A prune repacks, so it also holds the CPU: two prunes never run at once. A check only reads.
            let _locks =
                self.lock(run, reading.is_none(), &[(repo, if reading.is_none() { Use::Prune } else { Use::Read })])?;
            let name = repo.name.clone();
            match reading {
                None => {
                    let freed = run.tracked(None, |progress| self.rustic.prune(&run.cx, repo, progress))?;
                    if self.store.is_dry_run() {
                        return Ok(StepResult::WouldPrune { repo: name, freed });
                    }
                    self.measure(run, repo);
                    Ok(StepResult::Pruned { repo: name, freed })
                }
                // Reading the data is the check itself: a dry run says what it would read.
                Some(reading) if self.store.is_dry_run() => Ok(StepResult::WouldCheck { repo: name, reading }),
                Some(reading) => {
                    run.tracked(None, |progress| self.rustic.check(&run.cx, repo, reading, progress))?;
                    Ok(StepResult::Verified { repo: name, reading })
                }
            }
        })();
        // Recorded before the attempt, so that the check fails while any subset fails.
        if let Some(reading) = reading {
            let read = match &result {
                Ok(_) => Read::Through,
                Err(_) if run.cx.stopping() => Read::Not,
                Err(_) => Read::Failed,
            };
            self.check_read(collection, &repo.name, reading, read);
        }
        self.step(run, started, result);
    }

    /// A subset that was not read through is owed, and taken again first.
    pub(super) fn check_read(&self, collection: &Collection<'_>, repo: &str, reading: Reading, read: Read) {
        if let Reading::Subset { n, .. } = reading {
            self.emit(collection.name(), &[Event::CheckRead { repo: repo.to_owned(), n, read }]);
        }
    }

    /// A check that will not run owes its data subsets.
    pub(super) fn unread(&self, collection: &Collection<'_>, request: &Request) {
        let Request::Check(repos) = request else { return };
        for (repo, reading) in repos {
            if let Some(reading) = reading {
                self.check_read(collection, repo, *reading, Read::Not);
            }
        }
    }

    /// The request as it runs. A check takes the next data subset now, so that a check queued after it
    /// reads the following one. With [`settings::CHECK_SUBSETS`] at 0 it reads the structure only, and a
    /// sample (`5%`, `2GiB`, `all`) takes no subset. Only this function takes subsets.
    pub(crate) fn claim(&self, collection: &Collection<'_>, request: &Request) -> Result<Request> {
        let Request::Check(repos) = request else { return Ok(request.clone()) };
        let claimed = repos.iter().map(|(name, reading)| match reading {
            Some(reading) => Ok((name.clone(), Some(*reading))),
            None => Ok((name.clone(), Some(self.reading(collection, name)?))),
        });
        Ok(Request::Check(claimed.collect::<Result<_>>()?))
    }

    /// What the next check of `name` reads.
    fn reading(&self, collection: &Collection<'_>, name: &str) -> Result<Reading> {
        let repo = collection.repo_named(name)?;
        let value = settings::CHECK_SUBSETS.get(&[repo, collection.primary]).unwrap_or_default();
        Ok(match value.parse::<u32>() {
            Ok(0) => Reading::Structure,
            Ok(of) => Reading::Subset { n: self.store.take_subset(name, of)?, of },
            Err(_) => Reading::Sample(value.parse().map_err(|_unreadable| Error::NoSubsets { value: value.clone() })?),
        })
    }
}

/// How many subsets checks cycle through. `None` for a sample (`5%`) or for structure-only checks.
pub(super) fn subsets(chain: &[&RusticProfile]) -> Option<u32> {
    settings::CHECK_SUBSETS.get(chain)?.parse().ok().filter(|&of| of > 0)
}
