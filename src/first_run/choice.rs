//! The setup form's choices.

use serde::Deserialize;

/// What a repository is: the kinds a first backup most likely has, among rustic's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, crate::i18n::Fluent, strum::EnumIter)]
#[fluent(prefix = "first-run-kind")]
#[serde(try_from = "String")]
pub(crate) enum Kind {
    #[default]
    None,
    Local,
    Smb,
    Nfs,
    S3,
    Sftp,
    Webdav,
    Rest,
    Gdrive,
    Onedrive,
    Dropbox,
    Pcloud,
    /// Any other rclone remote.
    Rclone,
}

/// A row of kinds in the form: how rustic reaches them.
#[cfg(feature = "server")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, crate::i18n::Fluent, strum::EnumIter)]
pub(crate) enum Group {
    Mounted,
    Direct,
    Rclone,
}

/// Which repository a value is for. It names the form's choice of [`Kind`] and starts its fields' names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumIter)]
#[strum(serialize_all = "lowercase")]
pub(crate) enum Role {
    Primary,
    Target,
}

impl Role {
    /// The kinds rustic supports here, as its sftp does not run on Windows, by row and name. A primary is
    /// needed, and no copy comes first.
    #[cfg(feature = "server")]
    pub(crate) fn rows(self) -> Vec<(Group, Vec<Kind>)> {
        let offered = |k: &Kind| !(cfg!(windows) && *k == Kind::Sftp || self == Self::Primary && *k == Kind::None);
        let mut kinds: Vec<Kind> = <Kind as strum::IntoEnumIterator>::iter().filter(offered).collect();
        kinds.sort_by_key(|k| (*k != Kind::None, k.to_string().to_lowercase()));
        let row = |g| (g, kinds.iter().copied().filter(|k| k.group() == g).collect());
        <Group as strum::IntoEnumIterator>::iter().map(row).collect()
    }
}

/// How often a step runs. Its [`Choice::id`] is the form's value and, but for `Manual` and `Custom`, the
/// schedule's `@` name. `Manual` sets no schedule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, crate::i18n::Fluent, strum::EnumIter)]
#[serde(try_from = "String")]
pub(crate) enum Every {
    /// Only when started by hand.
    Manual,
    Hourly,
    #[default]
    Daily,
    Weekly,
    Monthly,
    /// A cron expression, sent as `<step>.custom`.
    Custom,
}

/// How long backups are kept: the profile's `[forget]`. Each keeps the last ten as well, so that a second
/// backup of the same day is not forgotten at once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, crate::i18n::Fluent, strum::EnumIter)]
#[serde(try_from = "String")]
pub(crate) enum Keep {
    #[default]
    Year,
    Month,
    /// No `[forget]`: the repository only grows.
    All,
}

impl Keep {
    /// How often what it no longer keeps is removed: often enough that a month's backups do not pile up, and
    /// never when it keeps everything.
    pub(super) fn prune(self) -> Every {
        match self {
            Self::Year => Every::Monthly,
            Self::Month => Every::Weekly,
            Self::All => Every::Manual,
        }
    }

    pub(super) fn rules(self) -> &'static [(&'static str, i64)] {
        match self {
            Self::Year => &[("keep-last", 10), ("keep-daily", 7), ("keep-weekly", 4), ("keep-monthly", 12)],
            Self::Month => &[("keep-last", 10), ("keep-daily", 30)],
            Self::All => &[],
        }
    }
}

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "first-run-no-choice")]
pub(crate) struct Unknown {
    choice: String,
}

/// A choice in the setup form.
pub(crate) trait Choice: std::fmt::Debug + strum::IntoEnumIterator {
    /// As the form sends it: the variant's name.
    fn id(&self) -> String {
        format!("{self:?}").to_lowercase()
    }

    /// The choice whose [`Choice::id`] the form sent.
    fn chosen(sent: &str) -> Result<Self, Unknown> {
        Self::iter().find(|c| c.id() == sent).ok_or_else(|| Unknown { choice: sent.to_owned() })
    }
}

impl Choice for Kind {}
impl Choice for Every {}
impl Choice for Keep {}

impl TryFrom<String> for Keep {
    type Error = Unknown;
    fn try_from(sent: String) -> Result<Self, Unknown> {
        Self::chosen(&sent)
    }
}

impl TryFrom<String> for Kind {
    type Error = Unknown;
    fn try_from(sent: String) -> Result<Self, Unknown> {
        Self::chosen(&sent)
    }
}

impl TryFrom<String> for Every {
    type Error = Unknown;
    fn try_from(sent: String) -> Result<Self, Unknown> {
        Self::chosen(&sent)
    }
}
