//! The answers, as the form sends them.

use std::collections::BTreeMap;

use serde::Deserialize;

#[cfg(any(feature = "server", test))]
use super::Error;
use super::{Choice, Every, Field, Keep, Key, Kind, Role};
use crate::{settings, sys};

/// What the fields of the folders to back up are named, numbered from 0: `source.0`, `source.1`…
pub(crate) const SOURCE: &str = "source";

/// The form's questions, which also say in the profiles what each part answers.
#[derive(Clone, Copy, crate::i18n::Fluent)]
pub(crate) enum Question {
    WhatToBackUp,
    WhereSaved,
    WhereCopied,
    WhenBackUp,
    HowLongKeep,
    HowOftenCheck,
    WhereNotify,
}

/// The answers. The folders are named `source.<n>`, the repositories' values `<role>.<kind>.<key>`, and
/// custom schedules `<step>.custom`: those not chosen are ignored.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub(crate) struct Setup {
    /// The collection's name, which its profiles' names start with.
    pub name: String,
    pub primary: Kind,
    #[serde(rename = "primary.password")]
    pub password: String,
    pub target: Kind,
    #[serde(rename = "target.password")]
    pub target_password: String,
    #[serde(flatten)]
    pub values: BTreeMap<String, String>,
    pub every: Every,
    pub keep: Keep,
    pub notify: String,
}

/// `path` with a leading `~` as the home folder: rustic would read it as a folder named `~`.
pub(crate) fn home(path: &str) -> String {
    let rest =
        path.strip_prefix('~').filter(|rest| rest.is_empty() || rest.starts_with(['/', std::path::MAIN_SEPARATOR]));
    match (rest, etcetera::home_dir()) {
        (Some(rest), Ok(home)) => format!("{}{rest}", home.display()),
        _ => path.to_owned(),
    }
}

/// `text` as typed or pasted, without the quotes that File Explorer's Copy as path adds around a path.
pub(crate) fn pasted(text: &str) -> &str {
    let text = text.trim();
    text.strip_prefix('"').and_then(|t| t.strip_suffix('"')).unwrap_or(text)
}

/// A repository's folder as typed, without the `local:` that rustic also takes, and with `~` as the home folder.
pub(crate) fn repository_folder(text: &str) -> String {
    let text = pasted(text);
    home(text.strip_prefix("local:").unwrap_or(text))
}

/// The first backup works without a second disk.
impl Default for Setup {
    fn default() -> Self {
        Self::of_folder(FOLDERS[0])
    }
}

/// The home folders a collection is first suggested for in this order.
const FOLDERS: [&str; 6] = ["Documents", "Pictures", "Desktop", "Music", "Videos", "Downloads"];

impl Setup {
    /// A further collection: for the first home folder that exists and that no profile is named after yet.
    #[cfg(feature = "server")]
    #[expect(clippy::disallowed_methods, reason = "looks for the user's folders")]
    pub(crate) fn next(profiles: &[String]) -> Self {
        let home = etcetera::home_dir().unwrap_or_default();
        let taken = |name: &str| profiles.iter().any(|p| p == name || p.starts_with(&format!("{name}-")));
        let free = FOLDERS.iter().find(|name| !taken(name) && home.join(name).is_dir());
        free.map_or_else(|| Self { name: String::new(), ..Self::default() }, |name| Self::of_folder(name))
    }

    /// The answers for backing up the home folder `name` (the system's files for root) to a folder named
    /// after it.
    fn of_folder(name: &str) -> Self {
        let home = etcetera::home_dir().unwrap_or_else(|_| "/home/me".into());
        let source = if sys::is_root() { super::os::ROOT_SOURCE.into() } else { home.join(name).display().to_string() };
        let folder = &Kind::Local.fields()[0];
        Self {
            name: name.into(),
            primary: Kind::Local,
            // Blank if the system has no randomness, which saving refuses.
            password: sys::random_hex().unwrap_or_default(),
            target_password: sys::random_hex().unwrap_or_default(),
            target: Kind::None,
            values: [
                (format!("{SOURCE}.0"), source),
                (Self::name(Role::Primary, Kind::Local, folder), home.join("backup").join(name).display().to_string()),
            ]
            .into(),
            every: Every::Daily,
            keep: Keep::default(),
            notify: desktop().unwrap_or_default().into(),
        }
    }
}

/// The notification URL of this desktop. A system service has none to show it on.
pub(crate) fn desktop() -> Option<&'static str> {
    (!sys::is_root()).then_some(super::os::DESKTOP)
}

impl Setup {
    /// The form's name for a field of `kind`.
    fn name(role: Role, kind: Kind, field: &Field) -> String {
        format!("{role}.{}.{}", kind.id(), field.key)
    }

    /// `role`'s repository password, with which rustic encrypts it.
    pub(crate) fn password(&self, role: Role) -> &str {
        match role {
            Role::Primary => self.password.trim(),
            Role::Target => self.target_password.trim(),
        }
    }

    pub(crate) fn kind(&self, role: Role) -> Kind {
        match role {
            Role::Primary => self.primary,
            Role::Target => self.target,
        }
    }

    /// What the form shows in `field` of `kind`: the answer, else the field's default.
    pub(crate) fn shown(&self, role: Role, kind: Kind, field: &Field) -> &str {
        self.values.get(&Self::name(role, kind, field)).map_or(field.value, String::as_str)
    }

    /// What `role`'s profile name ends with: its kind, or the name of another rclone remote, as far as it
    /// fits in a profile's name.
    fn suffix(&self, role: Role) -> String {
        let kind = self.kind(role);
        let remote = (kind == Kind::Rclone).then(|| {
            let repository =
                kind.fields().iter().find(|f| f.key == Key::Repository).map_or("", |f| self.value(role, f));
            let name = repository.split_once(':').map_or("", |(name, _)| name);
            name.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '-' }).collect::<String>()
        });
        remote.map(|r| r.trim_matches('-').to_owned()).filter(|r| !r.is_empty()).unwrap_or_else(|| kind.id())
    }

    /// The name of `role`'s profile: the collection's, then its suffix, unless it is the only one. A target with
    /// the primary's suffix is its copy.
    pub(crate) fn profile_name(&self, role: Role) -> String {
        let (suffix, name) = (self.suffix(role), self.name.trim());
        if self.target == Kind::None {
            name.to_owned()
        } else if role == Role::Target && suffix == self.suffix(Role::Primary) {
            format!("{name}-{suffix}-copy")
        } else {
            format!("{name}-{suffix}")
        }
    }

    /// The folders to back up, in the order of their fields, with `~` as the home folder. One given twice counts
    /// once.
    pub(crate) fn sources(&self) -> Vec<String> {
        let mut sources: Vec<String> = Vec::new();
        for source in self.source_fields().into_iter().filter(|source| !source.is_empty()).map(home) {
            if !sources.contains(&source) {
                sources.push(source);
            }
        }
        sources
    }

    /// The folder of `role`'s repository on a disk the system mounts, if given, with `~` as the home folder.
    pub(super) fn folder(&self, role: Role) -> Option<String> {
        let kind = self.kind(role);
        let field = kind.fields().iter().find(|f| f.key == Key::Repository).filter(|_| kind.scheme().is_empty())?;
        Some(self.value(role, field)).filter(|folder| !folder.is_empty()).map(repository_folder)
    }

    /// The form's folder fields in order, blank ones too, or one blank field.
    pub(crate) fn source_fields(&self) -> Vec<&str> {
        let mut fields: Vec<(usize, &str)> = (self.values.iter())
            .filter_map(|(key, value)| {
                Some((key.strip_prefix(SOURCE)?.strip_prefix('.')?.parse().ok()?, pasted(value)))
            })
            .collect();
        fields.sort_unstable_by_key(|(n, _)| *n);
        let fields: Vec<&str> = fields.into_iter().map(|(_, source)| source).collect();
        if fields.is_empty() { vec![""] } else { fields }
    }

    /// The value of `field` of `role`'s chosen kind.
    pub(super) fn value(&self, role: Role, field: &Field) -> &str {
        pasted(self.shown(role, self.kind(role), field))
    }

    /// Whether `role`'s repository refuses deletes.
    pub(super) fn immutable(&self, role: Role) -> bool {
        (self.kind(role).fields().iter()).any(|f| f.key == Key::Immutable && !self.value(role, f).is_empty())
    }

    /// Whether any repository forgets backups. Otherwise how long they are kept is not asked.
    pub(crate) fn forgets(&self) -> bool {
        <Role as strum::IntoEnumIterator>::iter().any(|r| self.kind(r) != Kind::None && !self.immutable(r))
    }

    /// How long backups are kept: all of them when no repository forgets any.
    pub(super) fn keep(&self) -> Keep {
        if self.forgets() { self.keep } else { Keep::All }
    }

    #[cfg(any(feature = "server", test))]
    pub(super) fn check(&self) -> Result<(), Error> {
        let name = self.name.trim();
        // rustic reads a profile name that starts with `-` as options.
        if !name.starts_with(char::is_alphanumeric) || !name.chars().all(|c| c.is_alphanumeric() || "-_.".contains(c)) {
            return Err(Error::Name);
        }
        if self.sources().is_empty() {
            return Err(Error::NoSources);
        }
        if self.primary == Kind::None {
            return Err(Error::NoPrimary);
        }
        for (_, step, ..) in self.schedules().into_iter().filter(|(_, _, every, _)| *every == Every::Custom) {
            match self.custom(step) {
                "" => return Err(Error::NoCustom),
                custom => settings::cron(custom)?,
            };
        }
        crate::notification::check(&self.notify)?;
        for role in <Role as strum::IntoEnumIterator>::iter().filter(|r| self.kind(*r) != Kind::None) {
            if self.password(role).is_empty() {
                return Err(Error::Incomplete { role, field: "repository-password" });
            }
            if let Some(f) = self.kind(role).fields().iter().find(|f| f.required && self.value(role, f).is_empty()) {
                return Err(Error::Incomplete { role, field: f.label });
            }
            let chosen = |f: &&Field| f.choices.is_empty() || f.choices.iter().any(|v| *v == self.value(role, f));
            if let Some(f) = self.kind(role).fields().iter().find(|f| !chosen(f)) {
                let values = f.choices.join(", ");
                return Err(Error::NotAChoice { role, field: f.label, choices: values });
            }
        }
        let folders = <Role as strum::IntoEnumIterator>::iter().filter_map(|role| self.folder(role));
        if let Some(relative) =
            self.sources().into_iter().chain(folders).find(|f| !std::path::Path::new(f).is_absolute())
        {
            return Err(Error::Relative { path: relative });
        }
        Ok(())
    }

    /// Each schedule's setting, the form's field, its answer and the question it answers.
    pub(super) fn schedules(&self) -> [(settings::Setting, &'static str, Every, Question); 3] {
        [
            (settings::BACKUP, "every", self.every, Question::WhenBackUp),
            (settings::CHECK, "check", self.every.check(), Question::HowOftenCheck),
            (settings::PRUNE, "prune", self.keep().prune(), Question::HowLongKeep),
        ]
    }

    /// The schedule `step` writes in the profile, if any.
    pub(super) fn cron(&self, step: &str, every: Every) -> Option<String> {
        match every {
            Every::Manual => None,
            Every::Custom => Some(self.custom(step).to_owned()).filter(|expr| !expr.is_empty()),
            Every::Hourly | Every::Daily | Every::Weekly | Every::Monthly => Some(format!("@{}", every.id())),
        }
    }

    /// The cron expression `step` was given, blank if none.
    fn custom(&self, step: &str) -> &str {
        self.values.get(&format!("{step}.custom")).map_or("", |e| e.trim())
    }
}
