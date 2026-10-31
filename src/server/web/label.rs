//! The text of the pages, defined once, by what it is on the page: a heading, a button, a word or a
//! sentence. Templates show it as `{{ Heading::Runs }}`.

/// A heading, a column name or a form label.
#[derive(Clone, Copy, crate::i18n::Fluent)]
pub(super) enum Heading<'a> {
    Collections,
    Collection,
    CollectionName,
    Repositories,
    Repository,
    Sources,
    Source,
    Folders,
    Snapshots,
    Snapshot,
    SnapshotPath,
    Issues,
    Runs,
    RecentRuns,
    RunDetails,
    Run,
    Steps,
    Step,
    Commands,
    Command,
    Log,
    NotFound,
    Name,
    Size,
    Modified,
    Actions,
    Kind,
    Path,
    Paths,
    Files,
    Added,
    Previous,
    Current,
    FileInfo,
    BackedUp,
    Checked,
    Pruned,
    Status,
    Time,
    Host,
    Index,
    Result,
    Duration,
    Start,
    Wait,
    Trigger,
    WaitingFor,
    Using,
    From,
    To,
    AtTime,
    DryRun,
    Token,
    Desktop,
    ChangesIn { snapshot: &'a str },
    FindIn { repo: &'a str },
    SetUp,
    ProfilesSaved,
    RestoreOnNewComputer,
}

/// A button or a link.
#[derive(Clone, Copy, crate::i18n::Fluent)]
pub(super) enum Button<'a> {
    BackToCollections,
    BackUp,
    BackUpAll,
    CheckAll,
    PruneAll,
    RefreshAll,
    Copy,
    CopyHere,
    CopyThis {
        text: &'a str,
    },
    Check,
    Prune,
    Recreate,
    RecreateThis {
        repo: &'a str,
    },
    Create,
    Restore,
    Download,
    DownloadPrevious,
    DownloadCurrent,
    Dismiss,
    TestNotification,
    Stop,
    Cancel,
    Changes,
    Find,
    FindAFile,
    CopyCommands,
    CopyLog,
    LogIn,
    LogOut,
    Save,
    AddFolder,
    RemoveFolder,
    /// The file manager's name for it.
    #[fluent(os = std::env::consts::OS)]
    OpenFolder,
    CopyFolderPath,
    SeeHow,
    CronSyntax,
    EmergencyKit,
    /// Uploads an emergency kit.
    RestoreFromKit,
}

/// A word or a state, in a table or a breadcrumb.
#[derive(Clone, Copy, crate::i18n::Fluent)]
pub(super) enum Label<'a> {
    EveryCollection,
    Unknown,
    Error,
    /// What `app.js` shows: the live dot's states.
    Live,
    Connecting,
    Reconnecting,
    Waiting,
    Stopping,
    WaitingForTheRunning,
    Left,
    /// A run in another process: who started it.
    Elsewhere {
        who: &'a str,
    },
    AlreadyQueued {
        what: &'a str,
    },
    Listing,
    ListingNow,
    Since,
    Latest,
    SnapshotWord,
    ChangesCrumb,
    FindCrumb,
    /// The run a schedule starts.
    BackupSchedule,
    CopySchedule,
    CheckSchedule,
    PruneSchedule,
    NotScheduled {
        what: &'a str,
    },
    Unschedulable {
        what: &'a str,
        error: &'a str,
    },
    Scheduled {
        what: &'a str,
        cron: &'a str,
    },
    ScheduledNext {
        what: &'a str,
        cron: &'a str,
        next: &'a str,
    },
}

/// A sentence, a tooltip or a placeholder.
#[derive(Clone, Copy, crate::i18n::Fluent)]
pub(super) enum Hint<'a> {
    /// What `app.js` shows: the outcome of a copy.
    Copied,
    CannotCopy,
    LogSelected,
    SourceCode,
    Empty,
    NothingChanged,
    NoRuns,
    WrongToken,
    NothingToRestore,
    SizeOnDisk,
    SizeOfFiles,
    SizeBackedUp,
    FilesBackedUp,
    AddedOnDisk,
    SizeOutdated,
    Holder,
    FirstSnapshot,
    FoundNowhere {
        collection: &'a str,
    },
    MoreHolding {
        more: usize,
    },
    NoSnapshotHere {
        collection: &'a str,
    },
    CreateProfilesIn {
        dir: &'a str,
    },
    /// The setup form cannot save in this folder.
    WriteProfilesIn {
        dir: &'a str,
    },
    KeepACopy,
    /// On a fresh install: the kit brings the profiles back.
    UploadKit {
        file: &'a str,
    },
    /// What `app.js` shows: an upload that got no answer.
    NotSent,
    NowVisible {
        app: &'a str,
    },
    ProfilesStep {
        dir: &'a str,
    },
    InstallStep {
        app: &'a str,
    },
    RestoreStep,
    ResumeStep,
    TokenIn {
        host: &'a str,
        file: &'a str,
    },
    TokenSet {
        host: &'a str,
        key: &'a str,
    },
    OnlyIfUnlisted,
    CreatedEmpty,
    NewestTarget,
    FromPrimary,
    LogInFirst,
    MetricsOnly {
        metrics: &'a str,
        status: &'a str,
    },
    CrossSite,
    NameOrPath,
    WhereItWas,
    Now,
    Notify,
    KitHelp,
}
