//! Messages for the pages' flash area: what a request did, and why a run a page started failed.

use crate::{log::Level, result::StepResult};

/// A message for the page's flash area. `collection`: the collection it concerns, or `Label::EveryCollection`.
#[derive(Debug, Clone, crate::i18n::Fluent)]
pub(crate) enum Notice {
    /// What started, and what was already queued.
    Started {
        collection: String,
        what: String,
    },
    Refreshing {
        collection: String,
    },
    RefreshingRepo {
        collection: String,
        repo: String,
    },
    Stopping {
        collection: String,
    },
    Dismissed {
        collection: String,
    },
    Notified {
        collection: String,
    },
    NotNotified {
        collection: String,
        error: String,
    },
    TestNotified,
    FolderNotOpened,
    /// The profiles of an uploaded kit, saved in `dir`.
    KitSaved {
        saved: u64,
        dir: String,
    },
    KitNotSaved {
        error: String,
    },
    TestNotNotified {
        error: String,
    },
    /// A run that the page started.
    RunFailed {
        collection: String,
        line: Box<StepResult>,
    },
    DestinationNotAbsolute,
    #[fluent(transparent)]
    InvalidSnapshot(String),
    #[fluent(transparent)]
    InvalidTime(String),
    NotSubmitted {
        collection: String,
        error: String,
    },
    #[fluent(transparent)]
    NotRefreshed(String),
    SetupNotSaved {
        error: String,
    },
    NotRunningHere {
        collection: String,
    },
    NothingToDismiss {
        collection: String,
    },
    CannotStop {
        error: String,
    },
    CannotDismiss {
        error: String,
    },
}

impl Notice {
    pub(crate) fn level(&self) -> Level {
        match self {
            Self::Started { .. }
            | Self::Refreshing { .. }
            | Self::RefreshingRepo { .. }
            | Self::Dismissed { .. }
            | Self::Notified { .. }
            | Self::TestNotified
            | Self::KitSaved { .. } => Level::Success,
            Self::Stopping { .. } => Level::Warning,
            Self::RunFailed { .. }
            | Self::DestinationNotAbsolute
            | Self::InvalidSnapshot(_)
            | Self::InvalidTime(_)
            | Self::NotSubmitted { .. }
            | Self::NotRefreshed(_)
            | Self::SetupNotSaved { .. }
            | Self::NotRunningHere { .. }
            | Self::NothingToDismiss { .. }
            | Self::NotNotified { .. }
            | Self::TestNotNotified { .. }
            | Self::FolderNotOpened
            | Self::KitNotSaved { .. }
            | Self::CannotStop { .. }
            | Self::CannotDismiss { .. } => Level::Failure,
        }
    }
}
