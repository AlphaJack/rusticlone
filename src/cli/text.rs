//! The text of `collections`, defined once.

use serde::Serialize;

use crate::{log, settings::Ignored};

/// A word or line of `collections`, with its values.
#[derive(Clone, Copy, crate::i18n::Fluent)]
#[fluent(prefix = "collections")]
pub(super) enum Text<'a> {
    Directories,
    /// The current folder, among the profile folders.
    Here {
        dir: &'a str,
    },
    Sources,
    Repositories,
    Issues,
    Size,
    Files,
    Profile,
    Location,
    Snapshots,
    Status,
    Schedules,
    Primary,
    CopyTarget,
    /// Where no profile is a collection.
    NoCollections,
    /// The profiles that are not collections.
    Other,
    Unknown,
    Failing {
        failures: u32,
    },
    /// A schedule and when its step last ran.
    Scheduled {
        cron: &'a str,
        ran: &'a str,
    },
    /// A schedule, and the step that never ran: `manual, no prune yet`.
    NotYet {
        cron: &'a str,
        step: &'a str,
    },
    Unscheduled,
    Unschedulable {
        error: &'a str,
    },
    /// A repository's status, with what its last listing found.
    Newest {
        status: &'a str,
        taken: &'a str,
    },
    Behind {
        status: &'a str,
        age: &'a str,
    },
    Listed {
        status: &'a str,
        age: &'a str,
    },
}

/// What a profile is. `Display`: the heading of the profiles of a kind.
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, crate::i18n::Fluent)]
#[fluent(prefix = "collections-kind")]
#[serde(rename_all = "kebab-case")]
pub(super) enum Kind {
    BackupProfile,
    CopyTarget,
    /// `[[backup.snapshots]]` without sources.
    NoSources,
    /// Named in another profile's `use-profiles`.
    Inherited,
    Unused,
    #[default]
    Broken,
}

/// Something wrong with a profile that `collections` reports.
#[derive(Serialize, crate::i18n::Fluent)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[fluent(prefix = "collections-warning")]
pub(super) enum Warning {
    /// Rustic reads only the first file with the profile's name.
    #[fluent(hidden = hidden.join(", "))]
    Hidden { read: String, hidden: Vec<String> },
    #[fluent(transparent)]
    Ignored(Ignored),
}

log::level!(Warning: Warning);
