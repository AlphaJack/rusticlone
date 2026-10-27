//! The steps a request runs, in order, with the earlier steps each one needs. Each step is one operation on
//! one repository. A step whose needs did not do their job is skipped; no step calls another. Steps run at the
//! same time once the steps before them ended, see [`follows`]: a backup's snapshot comes before its copies,
//! which send it, yet the copies run when it fails, to send what earlier backups left pending.

use anyhow::Result;

use super::{Collection, Error};
use crate::request::{Request, Step};

pub(super) fn plan(request: &Request, collection: &Collection<'_>) -> Result<Vec<(Step, Vec<usize>)>> {
    // A repository without a `[forget]` policy has nothing to forget.
    let idle =
        |step: &Step| matches!(step, Step::Forget { repo } if !collection.repo(repo).is_some_and(|r| r.has_keep));
    Ok(with_needs(steps(request, collection)?.into_iter().filter(|s| !idle(s)).collect(), collection))
}

/// The step that step `i` of `request`'s plan waits for, whether or not it did its job: a backup's copies
/// follow its snapshot, which they send, and a re-create's steps follow each other. Other steps wait only for
/// the steps they need, so that steps without conflicting locks run at the same time.
pub(crate) fn follows(request: Option<&Request>, i: usize) -> Option<usize> {
    match request? {
        Request::Backup if i > 0 => Some(0),
        Request::Recreate { .. } => i.checked_sub(1),
        Request::Backup
        | Request::Copy(_)
        | Request::Prune(_)
        | Request::Check(_)
        | Request::Restore(_)
        | Request::TestRestore(..) => None,
    }
}

/// A target's forget needs its copy. The primary's forget needs every copy to a usable target, so that
/// it keeps what a target lacks.
fn with_needs(steps: Vec<Step>, collection: &Collection<'_>) -> Vec<(Step, Vec<usize>)> {
    let copy = |to: &str| steps.iter().position(|s| matches!(s, Step::Copy { to: t } if t == to));
    let needs = |step: &Step| match step {
        Step::Forget { repo } if *repo == collection.name() => {
            collection.targets.iter().filter_map(|t| copy(&t.name)).collect()
        }
        Step::Forget { repo } => copy(repo).into_iter().collect(),
        Step::Snapshot { .. }
        | Step::Copy { .. }
        | Step::Prune { .. }
        | Step::Check { .. }
        | Step::Restore { .. }
        | Step::TestRestore { .. }
        | Step::Recreate { .. } => Vec::new(),
    };
    let needs: Vec<Vec<usize>> = steps.iter().map(needs).collect();
    steps.into_iter().zip(needs).collect()
}

fn steps(request: &Request, collection: &Collection<'_>) -> Result<Vec<Step>> {
    let repo = |name: &str| collection.repo_named(name).map(|p| p.name.clone());
    let primary = collection.name().to_owned();
    let target = |to: String| [Step::Copy { to: to.clone() }, Step::Forget { repo: to }];
    Ok(match request {
        Request::Backup => std::iter::once(Step::Snapshot { repo: primary }).chain(copies(collection)).collect(),
        // A copy from the primary goes to every target.
        Request::Copy(name) if *name == primary => copies(collection),
        Request::Copy(name) => target(repo(name)?).into(),
        Request::Prune(names) => {
            names.iter().map(|name| Ok(Step::Prune { repo: repo(name)? })).collect::<Result<_>>()?
        }
        Request::Check(names) => (names.iter())
            .map(|(name, reading)| match reading {
                Some(reading) => Ok(Step::Check { repo: repo(name)?, reading: *reading }),
                None => Err(Error::NoSubset { repo: name.clone() }.into()),
            })
            .collect::<Result<_>>()?,
        Request::Restore(options) => vec![Step::Restore { options: options.clone() }],
        Request::TestRestore(name, tested) => {
            vec![Step::TestRestore { from: name.as_deref().map(repo).transpose()?, tested: tested.clone() }]
        }
        Request::Recreate { repo: name, from } => {
            let (name, from) = (repo(name)?, from.as_deref().map(repo).transpose()?);
            if from.as_ref() == Some(&name) {
                return Err(Error::FromItself { repo: name }.into());
            }
            if name == primary {
                vec![Step::Recreate { repo: primary, from, requested: true }]
            } else {
                // From the primary: copy again, re-creating the target first if it is missing. From another
                // target: start with that target's snapshots.
                let from = from.filter(|f| *f != primary);
                [
                    Step::Recreate { repo: primary, from: None, requested: false },
                    Step::Recreate { repo: name.clone(), from, requested: true },
                ]
                .into_iter()
                .chain(target(name))
                .collect()
            }
        }
    })
}

/// A copy and forget for every target, then the primary's forget. A target without a usable profile
/// also gets its copy step, which fails and says why.
fn copies(collection: &Collection<'_>) -> Vec<Step> {
    let primary = Step::Forget { repo: collection.name().to_owned() };
    let targets = collection
        .targets
        .iter()
        .flat_map(|t| [Step::Copy { to: t.name.clone() }, Step::Forget { repo: t.name.clone() }]);
    let missing = collection.missing_targets.iter().map(|t| Step::Copy { to: t.clone() });
    targets.chain(missing).chain([primary]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{profile, request::Tested, rustic::Reading};

    #[test]
    fn plans() {
        let docs = profile::parse(
            "Docs",
            "[repository]\nrepository = \"rest:http://backup/docs\"\n[[backup.snapshots]]\nsources = [\"-\"]\n\
             as-path = \"/docs\"\n[copy]\ntargets = [\"usb\", \"cloud\"]\n[forget]\nkeep-last = 3\n",
        );
        // Only usb forgets: cloud keeps every snapshot, so it has no forget step.
        let usb = profile::parse("usb", "[repository]\nrepository = \"rest:http://u\"\n[forget]\nkeep-last = 3\n");
        let cloud = profile::parse("cloud", "[repository]\nrepository = \"rest:http://c\"\n");
        let collection =
            Collection { primary: &docs, title: "Docs", targets: vec![&usb, &cloud], missing_targets: Vec::new() };
        let requests = [
            Request::Backup,
            Request::Copy("Docs".into()),
            Request::Copy("usb".into()),
            Request::Prune(vec!["cloud".into()]),
            Request::Prune(vec!["Docs".into(), "usb".into()]),
            Request::Check(vec![("Docs".into(), None)]),
            Request::Check(vec![("usb".into(), Some(Reading::Subset { n: 3, of: 10 }))]),
            Request::Check(vec![("cloud".into(), Some(Reading::Structure))]),
            Request::Check(vec![("Docs".into(), Some(Reading::Structure)), ("usb".into(), Some(Reading::Structure))]),
            Request::TestRestore(Some("usb".into()), Tested::Newest(Some(0))),
            Request::TestRestore(None, Tested::Newest(None)),
            Request::Recreate { repo: "Docs".into(), from: None },
            Request::Recreate { repo: "Docs".into(), from: Some("cloud".into()) },
            Request::Recreate { repo: "cloud".into(), from: None },
            Request::Recreate { repo: "cloud".into(), from: Some("usb".into()) },
            Request::Recreate { repo: "cloud".into(), from: Some("cloud".into()) },
            Request::Check(vec![("elsewhere".into(), Some(Reading::Structure))]),
        ];
        let plans: Vec<_> =
            requests.iter().map(|r| (r.label("Docs"), plan(r, &collection).map_err(|e| e.to_string()))).collect();
        insta::assert_debug_snapshot!(plans);
    }
}
