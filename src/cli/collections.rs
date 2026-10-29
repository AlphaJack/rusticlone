//! `collections`: each collection's status, with what its profiles configure, as a tree.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::{
    each_collection,
    text::{Kind, Text, Warning},
    tree::{Tree, tree},
};
use crate::{
    engine::{CollectionOverview, Engine, Last, RepoOverview, Schedule, StatusReport},
    fmt,
    i18n::t,
    log::{self, Level, Message},
    profile,
    request::RunKind,
    rustic, settings,
    state::RepoInfo,
};

/// What `collections` shows: each collection's status, with what its profiles configure.
#[derive(Serialize)]
struct Overview {
    #[serde(flatten, serialize_with = "crate::shape::serialize")]
    status: StatusReport,
    directories: String,
    /// The profile folders, the current one named so.
    #[serde(skip)]
    folders: Vec<String>,
    profiles: Vec<ProfileInfo>,
    /// Each repository's last listing, by collection and repository.
    #[serde(skip)]
    listings: BTreeMap<(String, String), RepoInfo>,
}

pub(super) fn show(engine: &Engine, refresh: bool) -> Level {
    let level = if refresh {
        // Broken profiles are listed below, with why.
        each_collection(engine, &engine.collections(), &t!("cli-refresh"), |collection| engine.refresh(collection))
    } else {
        Level::Success
    };
    let inherited: BTreeSet<&str> =
        engine.rustic_profiles.values().flat_map(|p| p.uses.iter().map(String::as_str)).collect();
    let mut profiles: Vec<ProfileInfo> =
        engine.rustic_profiles.values().map(|p| profile_info(engine, p, &inherited)).collect();
    profiles.extend(engine.broken.iter().map(|(name, err)| ProfileInfo {
        name: name.clone(),
        error: Some(err.clone()),
        ..ProfileInfo::default()
    }));
    let warned = profiles.iter().any(|i| !i.warnings.is_empty()).then_some(Level::Warning);
    let invalid = profiles.iter().any(|i| i.kind == Kind::Broken).then_some(Level::Failure);
    let status = engine.report();
    let level = level.max(status.level).max(warned.unwrap_or_default()).max(invalid.unwrap_or_default());
    let repos =
        status.collections.iter().flat_map(|c| std::iter::once(&c.primary).chain(&c.targets).map(move |r| (c, r)));
    let listings = repos
        .filter_map(|(c, r)| {
            let info = engine.store.inventory(&c.primary.name, &r.name)?.result.ok()?;
            Some(((c.primary.name.clone(), r.name.clone()), info))
        })
        .collect();
    let status = StatusReport { level, ..status };
    let here = std::env::current_dir().ok();
    let folders = (profile::profile_dirs().unwrap_or_default().iter())
        .map(|d| match Some(d) == here.as_ref() {
            true => Text::Here { dir: &d.display().to_string() }.to_string(),
            false => d.display().to_string(),
        })
        .collect();
    let overview = Overview { status, directories: profile::dirs_display(), folders, profiles, listings };
    let text = match log::is_json() {
        true => format!("{}\n", serde_json::to_string(&overview).unwrap_or_default()),
        false => overview.to_string(),
    };
    log::clear();
    // A closed pipe, as with `| head`, ends the output, not the process.
    _ = std::io::Write::write_all(&mut std::io::stdout().lock(), text.as_bytes());
    level
}

impl Overview {
    fn profile(&self, name: &str) -> Option<&ProfileInfo> {
        self.profiles.iter().find(|p| p.name == name)
    }

    fn listing(&self, collection: &CollectionOverview, repo: &str) -> Option<&RepoInfo> {
        self.listings.get(&(collection.primary.name.clone(), repo.to_owned()))
    }

    fn last(&self, last: Last) -> String {
        match last {
            Last::At(t) => fmt::human_ago(t, self.status.time),
            Last::Never => Last::never(),
            Last::Unknown => unknown(),
            Last::Failing(n) => Text::Failing { failures: n }.to_string(),
        }
    }

    /// Each source with the files and size of its newest snapshot in the primary.
    fn sources(&self, c: &CollectionOverview, info: &ProfileInfo) -> Tree {
        let snapshots = self.listing(c, &c.primary.name).map(|l| l.snapshots.as_slice()).unwrap_or_default();
        let sources = info.sources.iter().map(|g| {
            let key = rustic::source_key(g);
            let newest = snapshots.iter().filter(|sn| sn.source() == key).max_by_key(|sn| sn.time);
            let summary = newest.and_then(|sn| sn.summary.as_ref());
            let size = summary.map_or_else(unknown, |s| fmt::human_size(s.total_bytes_processed));
            let files = summary.map_or_else(unknown, |s| s.total_files_processed.to_string());
            Tree { line: log::bold(&g.join(", ")), below: vec![field(Text::Files, &files), field(Text::Size, &size)] }
        });
        Tree { line: section(Text::Sources), below: sources.collect() }
    }

    fn repo(&self, c: &CollectionOverview, r: &RepoOverview) -> Tree {
        let primary = r.name == c.primary.name;
        let listed = self.listing(c, &r.name).filter(|_| !r.missing);
        let count = listed.map_or_else(unknown, |l| l.snapshots.len().to_string());
        let size = listed.and_then(|l| l.stored).map_or_else(unknown, fmt::human_size);
        let file = self.profile(&r.name).and_then(|i| i.file.as_deref()).unwrap_or_default();
        let mut below = vec![
            field(Text::Profile, file),
            field(Text::Location, r.repository.as_deref().unwrap_or_default()),
            field(Text::Snapshots, &count),
            field(Text::Size, &size),
        ];
        // Every repository is backed up on the collection's schedule, a target by a copy; checks and prunes
        // have the repository's own.
        let backup = if primary { RunKind::Backup } else { RunKind::Copy };
        let schedules = vec![
            self.scheduled(backup, c.backup_schedule.as_ref(), c.backed_up(r)),
            self.scheduled(RunKind::Check, r.check_schedule.as_ref(), r.checked),
            self.scheduled(RunKind::Prune, r.prune_schedule.as_ref(), r.pruned),
        ];
        below.push(Tree { line: section(Text::Schedules), below: schedules });
        let role = if primary { Text::Primary } else { Text::CopyTarget };
        Tree { line: format!("{} ({role})", log::bold(&r.name)), below }
    }

    /// `check: @weekly, last 2 days ago`, `prune: manual, no prune yet`.
    fn scheduled(&self, kind: RunKind, schedule: Option<&Schedule>, last: Last) -> Tree {
        let cron = match schedule {
            None => Text::Unscheduled.to_string(),
            Some(Schedule { error: Some(e), .. }) => Text::Unschedulable { error: e }.to_string(),
            Some(s) => s.cron.clone(),
        };
        let when = match last {
            Last::Never => Text::NotYet { cron: &cron, step: &kind.label() }.to_string(),
            last @ (Last::At(_) | Last::Unknown | Last::Failing(_)) => {
                Text::Scheduled { cron: &cron, ran: &self.last(last) }.to_string()
            }
        };
        field(log::capitalized(&kind.label()), &when)
    }

    fn collection(&self, c: &CollectionOverview) -> Vec<Tree> {
        let p = &c.primary;
        let mut branches = Vec::new();
        let names = std::iter::once(&p.name).chain(c.targets.iter().map(|t| &t.name));
        let hidden = (names.filter_map(|n| self.profile(n)).flat_map(|i| &i.warnings))
            .filter(|w| matches!(w, Warning::Hidden { .. }))
            .map(|w| (w.level(), w.to_string()));
        let issues = c.issues.iter().map(|i| (i.level(), i.text(self.status.time))).chain(hidden);
        let issues: Vec<Tree> =
            issues.map(|(level, i)| Tree::leaf(format!("{} {}", level.dot(), log::paint(level, &i)))).collect();
        if !issues.is_empty() {
            branches.push(Tree { line: section(Text::Issues), below: issues });
        }
        if let Some(info) = self.profile(&p.name).filter(|i| i.error.is_none()) {
            branches.push(self.sources(c, info));
            let repos = std::iter::once(p).chain(&c.targets).map(|r| self.repo(c, r)).collect();
            branches.push(Tree { line: section(Text::Repositories), below: repos });
        }
        branches
    }
}

impl std::fmt::Display for Overview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let folders: Vec<Tree> = self.folders.iter().cloned().map(Tree::leaf).collect();
        writeln!(f, "{}", section(Text::Directories))?;
        f.write_str(&tree(&folders))?;
        if let Some(untested) = &self.status.untested {
            writeln!(f, "{} {}", Level::Warning.dot(), log::paint(Level::Warning, &untested.to_string()))?;
        }
        // Broken profiles are listed with the other profiles that are not collections.
        let readable = |c: &&CollectionOverview| self.profile(&c.primary.name).is_some_and(|i| i.error.is_none());
        let mut collections = self.status.collections.iter().filter(readable).peekable();
        if collections.peek().is_none() {
            writeln!(f, "\n{} {}", Level::Warning.dot(), log::paint(Level::Warning, &Text::NoCollections.to_string()))?;
        }
        for c in collections {
            writeln!(f, "\n{}  {} {}", log::bold(&c.title), c.level.dot(), log::paint(c.level, &c.level.word()))?;
            f.write_str(&tree(&self.collection(c)))?;
        }
        let kinds = [Kind::Inherited, Kind::Unused, Kind::Broken].map(|kind| {
            let profiles: Vec<Tree> = (self.profiles.iter().filter(|p| p.kind == kind))
                .map(|p| match &p.error {
                    Some(e) => Tree::leaf(format!("{}: {}", log::bold(&p.name), log::paint(Level::Failure, e))),
                    None => Tree::leaf(log::bold(&p.name)),
                })
                .collect();
            Tree { line: section(kind), below: profiles }
        });
        let other: Vec<Tree> = kinds.into_iter().filter(|k| !k.below.is_empty()).collect();
        if !other.is_empty() {
            writeln!(f, "\n{}", section(Text::Other))?;
            f.write_str(&tree(&other))?;
        }
        Ok(())
    }
}

/// A heading, in bold.
fn section(text: impl std::fmt::Display) -> String {
    log::bold(&text.to_string())
}

/// `key: value`, with the key in bold.
fn field(key: impl std::fmt::Display, value: &str) -> Tree {
    Tree::leaf(format!("{}: {value}", section(key)))
}

fn unknown() -> String {
    Text::Unknown.to_string()
}

/// A profile, as `collections --json` shows it.
#[derive(Default, Serialize)]
struct ProfileInfo {
    name: String,
    kind: Kind,
    /// The file rustic reads it from.
    file: Option<String>,
    /// The collections that copy to it.
    copy_target_of: Vec<String>,
    /// The paths of each `[[backup.snapshots]]` entry.
    sources: Vec<Vec<String>>,
    targets: Vec<String>,
    /// It has `keep-*` options.
    forgets: bool,
    #[serde(serialize_with = "crate::log::serialize_messages")]
    warnings: Vec<Warning>,
    /// Why it cannot be read.
    error: Option<String>,
}

/// `inherited`: the profiles that others name in `use-profiles`.
fn profile_info(engine: &Engine, p: &profile::RusticProfile, inherited: &BTreeSet<&str>) -> ProfileInfo {
    let copying: Vec<&profile::RusticProfile> =
        engine.rustic_profiles.values().filter(|j| j.is_backup_profile() && j.targets.contains(&p.name)).collect();
    let kind = match (p.is_backup_profile(), copying.is_empty()) {
        (true, _) => Kind::BackupProfile,
        (false, false) => Kind::CopyTarget,
        // Meant as a backup profile, which backs up nothing: invalid, with why.
        (false, true) if p.sourceless => Kind::Broken,
        (false, true) if inherited.contains(p.name.as_str()) => Kind::Inherited,
        (false, true) => Kind::Unused,
    };
    let files = p.files();
    let mut warnings = Vec::new();
    if let [read, hidden @ ..] = files.as_slice()
        && !hidden.is_empty()
    {
        let hidden = hidden.iter().map(|f| f.display().to_string()).collect();
        warnings.push(Warning::Hidden { read: read.display().to_string(), hidden });
    }
    // A profile without a repository is used by others, which report its settings.
    if p.repository.is_some() {
        warnings.extend(settings::ignored(p, &copying).into_iter().map(Warning::Ignored));
    }
    ProfileInfo {
        name: p.name.clone(),
        kind,
        file: files.first().map(|f| f.display().to_string()),
        copy_target_of: copying.iter().map(|j| j.name.clone()).collect(),
        sources: if p.is_backup_profile() { p.groups().collect() } else { Vec::new() },
        targets: if p.is_backup_profile() { p.targets.clone() } else { Vec::new() },
        forgets: p.has_keep,
        warnings,
        error: (kind == Kind::Broken).then(|| Text::NoSources.to_string()),
    }
}
