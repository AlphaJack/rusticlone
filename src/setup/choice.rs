//! The setup form's choices.

use serde::Deserialize;

/// What a repository is: the kinds a first backup most likely has, among rustic's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, crate::i18n::Fluent, strum::EnumIter)]
#[fluent(prefix = "setup-kind")]
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
        let offered = |k: &Kind| !(!super::os::SFTP && *k == Kind::Sftp || self == Self::Primary && *k == Kind::None);
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
#[fluent(prefix = "setup-no-choice")]
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

/// Makes each type a [`Choice`] that the form's text converts to.
macro_rules! choices {
    ($($choice:ty),*) => {$(
        impl Choice for $choice {}

        impl TryFrom<String> for $choice {
            type Error = Unknown;
            fn try_from(sent: String) -> Result<Self, Unknown> {
                Self::chosen(&sent)
            }
        }
    )*};
}

choices!(Kind, Every, Keep);

/// How to configure a remote.
#[cfg(feature = "server")]
const RCLONE_DOCS: &str = "https://rclone.org/docs/";

/// How rustic reaches each kind.
impl Kind {
    /// What the repository starts with, followed by the [`super::Key::Repository`] field if it has one.
    pub(super) fn scheme(self) -> &'static str {
        match self {
            Self::None | Self::Local | Self::Smb | Self::Nfs => "",
            Self::Sftp => "opendal:sftp",
            Self::S3 => "opendal:s3",
            Self::Webdav => "opendal:webdav",
            Self::Pcloud => "opendal:pcloud",
            Self::Rest => "rest:",
            Self::Gdrive | Self::Onedrive | Self::Dropbox | Self::Rclone => "rclone:",
        }
    }

    #[cfg(feature = "server")]
    pub(crate) fn group(self) -> Group {
        match self.scheme() {
            "" => Group::Mounted,
            "rclone:" => Group::Rclone,
            _ => Group::Direct,
        }
    }

    /// Where [`Self::hint`] is explained.
    #[cfg(feature = "server")]
    pub(crate) fn docs(self) -> Option<&'static str> {
        (self.group() == Group::Rclone).then_some(RCLONE_DOCS)
    }

    /// What to do before saving: rclone signs in to the drives that opendal would need an OAuth app for.
    #[cfg(feature = "server")]
    pub(crate) fn hint(self) -> Option<String> {
        (self.group() == Group::Rclone).then(|| crate::i18n::t!("field-rclone-hint"))
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;

    /// The form offers the kinds rustic supports here, in rows: those on a mounted disk, those rustic reaches
    /// itself, then rclone's. A primary must have a repository; a copy may have none, which comes first.
    #[test]
    fn the_form_offers_the_kinds_rustic_supports_by_row() {
        for role in [Role::Primary, Role::Target] {
            let rows = role.rows();
            assert_eq!(
                rows.iter().map(|(g, _)| *g).collect::<Vec<_>>(),
                [Group::Mounted, Group::Direct, Group::Rclone]
            );
            for (group, kinds) in &rows {
                assert!(kinds.iter().all(|k| k.group() == *group), "{role} {group:?}: {kinds:?}");
                let named: Vec<String> =
                    kinds.iter().filter(|k| **k != Kind::None).map(|k| k.to_string().to_lowercase()).collect();
                assert!(named.is_sorted(), "{role} {group:?}: {named:?}");
            }
            let all: Vec<Kind> = rows.into_iter().flat_map(|(_, kinds)| kinds).collect();
            assert_eq!(all.contains(&Kind::Sftp), super::super::os::SFTP, "{role}: sftp where rustic has it");
            assert_eq!(all.contains(&Kind::None), role == Role::Target);
        }
        assert_eq!(Role::Target.rows()[0].1.first(), Some(&Kind::None), "no copy, first");
    }

    /// A mounted disk needs no scheme, rclone's remotes say where to sign in first, and rustic's own need neither.
    #[test]
    fn each_kind_is_reached_its_way() {
        assert_eq!(
            [Kind::Local, Kind::S3, Kind::Gdrive].map(Kind::group),
            [Group::Mounted, Group::Direct, Group::Rclone]
        );
        assert_eq!((Kind::Gdrive.docs(), Kind::S3.docs()), (Some(RCLONE_DOCS), None));
        assert!(Kind::Rclone.hint().is_some_and(|h| !h.is_empty()) && Kind::Local.hint().is_none());
    }
}
