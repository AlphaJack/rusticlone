//! The first collection: profiles from a few answers, which the web UI asks and the CLI prints as an
//! example.

#![cfg_attr(feature = "server", expect(clippy::disallowed_methods, reason = "saves the profiles"))]

use std::collections::BTreeMap;
#[cfg(feature = "server")]
use std::path::PathBuf;

use serde::Deserialize;
use toml_edit::{Array, DocumentMut, Item, Table, value};

use crate::{profile, settings, sys};

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

impl Every {
    /// How often the repositories are checked for damage: often enough to find it before many backups depend
    /// on a damaged one. Rarely enough not to read them over and over. Never without a schedule.
    fn check(self) -> Self {
        match self {
            Self::Manual => Self::Manual,
            Self::Hourly | Self::Daily | Self::Custom => Self::Weekly,
            Self::Weekly | Self::Monthly => Self::Monthly,
        }
    }
}

impl Keep {
    /// How often what it no longer keeps is removed: often enough that a month's backups do not pile up, and
    /// never when it keeps everything.
    fn prune(self) -> Every {
        match self {
            Self::Year => Every::Monthly,
            Self::Month => Every::Weekly,
            Self::All => Every::Manual,
        }
    }

    fn rules(self) -> &'static [(&'static str, i64)] {
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

/// Where a field's value goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, crate::i18n::Fluent)]
#[fluent(prefix = "first-run-key")]
pub(crate) enum Key {
    /// The repository, after [`Kind::scheme`].
    Repository,
    /// A `[repository.options]` key.
    #[fluent(transparent)]
    Option(&'static str),
    /// A variable in `[global.env]`.
    #[fluent(transparent)]
    Env(&'static str),
    /// A repository that refuses deletes, ticked: it gets no `[forget]`, so it is never forgotten or pruned.
    Immutable,
}

impl Key {
    fn option(self) -> Option<&'static str> {
        if let Self::Option(key) = self { Some(key) } else { None }
    }

    fn env(self) -> Option<&'static str> {
        if let Self::Env(key) = self { Some(key) } else { None }
    }
}

/// A value a copy target needs.
#[cfg_attr(not(feature = "server"), expect(dead_code, reason = "the web UI's form reads them"))]
pub(crate) struct Field {
    pub key: Key,
    /// Its message, `field-<label>`.
    pub label: &'static str,
    pub example: &'static str,
    /// Filled in the form.
    pub value: &'static str,
    pub secret: bool,
    pub required: bool,
    /// The values it takes, if not any text, each meaning message `field-choice-<value>`.
    pub choices: &'static [&'static str],
}

impl Field {
    #[cfg(feature = "server")]
    pub(crate) fn text(&self) -> String {
        Self::text_of(self.label)
    }

    #[cfg(any(feature = "server", test))]
    fn text_of(label: &str) -> String {
        crate::i18n::find(&format!("field-{label}"), []).unwrap_or_default()
    }

    #[cfg(feature = "server")]
    pub(crate) fn meaning(choice: &str) -> String {
        crate::i18n::find(&format!("field-choice-{choice}"), []).unwrap_or_default()
    }
}

const fn field(key: Key, label: &'static str, example: &'static str) -> Field {
    Field { key, label, example, value: "", secret: false, required: true, choices: &[] }
}

const fn secret(key: Key, label: &'static str) -> Field {
    Field { key, label, example: "", value: "", secret: true, required: true, choices: &[] }
}

/// How an SFTP server's key is checked, as opendal's `known_hosts_strategy` takes it. A service has no terminal
/// to confirm a new server's key, so by default any is accepted.
const KNOWN_HOSTS: Field = Field {
    value: "accept",
    choices: &["accept", "add", "strict"],
    ..field(Key::Option("known_hosts_strategy"), "server-key", "")
};

/// For an encrypted rclone configuration.
const RCLONE_PASSWORD: Field = Field { required: false, ..secret(Key::Env("RCLONE_CONFIG_PASS"), "rclone-password") };
/// Reads the password from the system's secret store: Keychain on macOS, the Secret Service of GNOME and KDE.
const RCLONE_PASSWORD_COMMAND: Field = Field {
    required: false,
    ..field(
        Key::Env("RCLONE_PASSWORD_COMMAND"),
        "rclone-password-command",
        native(
            "pass rclone/config",
            "security find-generic-password -s rclone -w",
            "secret-tool lookup service rclone",
        ),
    )
};

/// A remote that rclone is configured with.
const fn rclone(example: &'static str) -> [Field; 3] {
    [field(Key::Repository, "remote", example), RCLONE_PASSWORD, RCLONE_PASSWORD_COMMAND]
}

/// How to configure a remote.
#[cfg(feature = "server")]
const RCLONE_DOCS: &str = "https://rclone.org/docs/";
/// Everything a profile can set.
const RUSTIC_CONFIG_DOCS: &str = "https://rustic.cli.rs/docs/commands/init/configuration_file.html";

/// A folder, at `example`.
const fn folder(example: &'static str) -> Field {
    field(Key::Repository, "folder", example)
}

/// What a user types on this system, as an example: Windows, macOS, or another.
const fn native(windows: &'static str, macos: &'static str, other: &'static str) -> &'static str {
    if cfg!(windows) {
        windows
    } else if cfg!(target_vendor = "apple") {
        macos
    } else {
        other
    }
}

/// What a repository's password is called.
#[cfg(feature = "server")]
pub(crate) fn password() -> String {
    crate::i18n::t!("field-repository-password")
}

/// A repository that refuses deletes, under its name for this kind.
const fn immutable(label: &'static str) -> Field {
    Field { required: false, ..field(Key::Immutable, label, "") }
}

/// `root`: where in the service the repository is.
const ROOT: Field = Field { required: false, ..field(Key::Option("root"), "path", "/backup") };

/// Each kind's fields go from where to who: server, bucket, path, user, then secrets.
impl Kind {
    /// What the repository starts with, followed by the [`Key::Repository`] field if it has one.
    fn scheme(self) -> &'static str {
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

    pub(crate) fn fields(self) -> &'static [Field] {
        match self {
            Self::None => &[],
            Self::Local => const { &[folder(native(r"D:\backup", "/Volumes/USB/backup", "/mnt/usb/backup"))] },
            Self::Smb => {
                const { &[folder(native(r"\\nas\share\backup", "/Volumes/share/backup", "/mnt/share/backup"))] }
            }
            Self::Nfs => const { &[folder(native(r"Z:\backup", "/Volumes/nas/backup", "/mnt/nas/backup"))] },
            // Key authentication only. A service has no ssh-agent, so it needs the key's path.
            Self::Sftp => {
                const {
                    &[
                        field(Key::Option("endpoint"), "server", "ssh://nas.local:22"),
                        ROOT,
                        field(Key::Option("user"), "user", "me"),
                        Field {
                            required: false,
                            ..field(Key::Option("key"), "private-key", "/home/me/.ssh/id_ed25519")
                        },
                        KNOWN_HOSTS,
                    ]
                }
            }
            Self::S3 => {
                const {
                    &[
                        field(Key::Option("endpoint"), "server", "https://s3.example.com"),
                        // Without one, each request waits for the region to be found.
                        Field { value: "auto", ..field(Key::Option("region"), "region", "") },
                        field(Key::Option("bucket"), "bucket", "backups"),
                        ROOT,
                        field(Key::Option("access_key_id"), "access-key", ""),
                        secret(Key::Option("secret_access_key"), "secret-key"),
                        immutable("object-lock"),
                    ]
                }
            }
            Self::Webdav => {
                const {
                    &[
                        field(Key::Option("endpoint"), "server", "https://cloud.example.com/remote.php/dav/files/me"),
                        ROOT,
                        field(Key::Option("username"), "user", "me"),
                        secret(Key::Option("password"), "password"),
                    ]
                }
            }
            Self::Pcloud => {
                const {
                    &[
                        Field {
                            value: "https://api.pcloud.com",
                            ..field(Key::Option("endpoint"), "server", "https://eapi.pcloud.com")
                        },
                        ROOT,
                        field(Key::Option("username"), "email", "me@example.com"),
                        secret(Key::Option("password"), "password"),
                    ]
                }
            }
            Self::Rest => {
                const {
                    &[
                        field(Key::Repository, "url", "https://me:password@backup.example.com:8000/me"),
                        immutable("append-only"),
                    ]
                }
            }
            Self::Gdrive => const { &rclone("gdrive:backup") },
            Self::Onedrive => const { &rclone("onedrive:backup") },
            Self::Dropbox => const { &rclone("dropbox:backup") },
            Self::Rclone => const { &rclone("remote:backup") },
        }
    }
}

/// What the fields of the folders to back up are named, numbered from 0: `source.0`, `source.1`…
pub(crate) const SOURCE: &str = "source";

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
        let source = match (sys::is_root(), cfg!(windows), cfg!(target_vendor = "apple")) {
            (false, ..) => home.join(name).display().to_string(),
            // Every account's files, which the system service reads.
            (true, true, _) => r"C:\Users".into(),
            // `/etc` is a symlink there, which a backup would save as one.
            (true, _, true) => "/private/etc".into(),
            (true, ..) => "/etc".into(),
        };
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
    match (sys::is_root(), cfg!(windows), cfg!(target_vendor = "apple")) {
        (true, ..) => None,
        (false, true, _) => Some("windows://"),
        (false, _, true) => Some("macosx://"),
        (false, ..) => Some("dbus://"),
    }
}

#[cfg(any(feature = "server", test))]
#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "first-run-error")]
pub(crate) enum Error {
    Name,
    /// rustic reads it from wherever it runs, and backs up nothing if nothing is there, without failing.
    Relative {
        path: String,
    },
    NoSources,
    NoPrimary,
    #[fluent(transparent)]
    Schedule(#[from] settings::Error),
    NoCustom,
    #[fluent(transparent)]
    Notify(#[from] crate::notification::Error),
    /// The field, by its [`Field::label`].
    #[fluent(field = Field::text_of(field))]
    Incomplete {
        role: Role,
        field: &'static str,
    },
    /// The field, by its [`Field::label`], and the values it takes.
    #[fluent(field = Field::text_of(field))]
    NotAChoice {
        role: Role,
        field: &'static str,
        choices: String,
    },
    #[cfg(feature = "server")]
    NoDir,
    #[cfg(feature = "server")]
    #[fluent(path = path.display())]
    Exists {
        path: PathBuf,
    },
    /// rustic would back up the repository into itself and grow it at every run.
    #[cfg(feature = "server")]
    Inside,
    /// Neither empty nor a repository: rustic would mix its files in with others.
    #[cfg(feature = "server")]
    NotEmpty {
        folder: String,
    },
}

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

/// A table under the question it answers.
fn section(answers: Question) -> Table {
    let mut table = Table::new();
    table.decor_mut().set_prefix(format!("\n# {answers}\n"));
    table
}

/// `key` of `table`, under the question it answers.
fn answering(table: &mut Table, key: &str, answers: Question) {
    if let Some(mut key) = table.key_mut(key) {
        key.leaf_decor_mut().set_prefix(format!("# {answers}\n"));
    }
}

/// A table that holds only tables, so has no header of its own.
fn implicit<'a>(entries: impl IntoIterator<Item = (&'a str, Item)>) -> Item {
    let mut table: Table = entries.into_iter().collect();
    table.set_implicit(true);
    Item::Table(table)
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
    fn folder(&self, role: Role) -> Option<String> {
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
    fn value(&self, role: Role, field: &Field) -> &str {
        pasted(self.shown(role, self.kind(role), field))
    }

    /// Whether `role`'s repository refuses deletes.
    fn immutable(&self, role: Role) -> bool {
        (self.kind(role).fields().iter()).any(|f| f.key == Key::Immutable && !self.value(role, f).is_empty())
    }

    #[cfg(any(feature = "server", test))]
    fn check(&self) -> Result<(), Error> {
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
    fn schedules(&self) -> [(settings::Setting, &'static str, Every, Question); 3] {
        [
            (settings::BACKUP, "every", self.every, Question::WhenBackUp),
            (settings::CHECK, "check", self.every.check(), Question::HowOftenCheck),
            (settings::PRUNE, "prune", self.keep.prune(), Question::HowLongKeep),
        ]
    }

    /// The schedule `step` writes in the profile, if any.
    fn cron(&self, step: &str, every: Every) -> Option<String> {
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

    /// `role`'s repository, password and options, in `table`.
    fn repository(&self, role: Role, mut table: Table) -> Item {
        let (kind, fields) = (self.kind(role), self.kind(role).fields());
        let repository = self.folder(role).unwrap_or_else(|| {
            fields.iter().find(|f| f.key == Key::Repository).map_or("", |f| self.value(role, f)).to_owned()
        });
        table["repository"] = value(profile::native_drive(&format!("{}{repository}", kind.scheme())));
        table["password"] = value(self.password(role));
        // Required ones even if blank, so that the example shows what to fill in.
        let options: Table = (fields.iter())
            .filter(|f| f.required || !self.value(role, f).is_empty())
            .filter_map(|f| Some((f.key.option()?, value(self.value(role, f)))))
            .collect();
        if !options.is_empty() {
            table["options"] = Item::Table(options);
        }
        Item::Table(table)
    }

    /// `role`'s variables in `[global.env]`.
    fn env(&self, role: Role) -> impl Iterator<Item = (&'static str, Item)> {
        (self.kind(role).fields().iter())
            .filter_map(move |f| Some((f.key.env()?, self.value(role, f))).filter(|(_, v)| !v.is_empty()))
            .map(|(key, v)| (key, value(v)))
    }

    /// File names and contents, the backup profile first, then the target's if any: see [`Self::profile_name`].
    /// Their shared start names the collection.
    pub(crate) fn profiles(&self) -> Vec<(String, String)> {
        let mut primary = DocumentMut::new();
        // One entry per folder: each has its own snapshots and history, so adding a folder later leaves the
        // others' alone. The first one says what the entries answer.
        let snapshots = self.sources().into_iter().enumerate().map(|(i, source)| {
            let mut snapshot = if i == 0 { section(Question::WhatToBackUp) } else { Table::new() };
            snapshot["sources"] = value([source].into_iter().collect::<Array>());
            snapshot
        });
        let mut backup = Table::new();
        // Without the domain that each network gives it, which would start a new snapshot group there.
        if let Some(host) = sys::hostname().split('.').next().filter(|h| !h.is_empty()) {
            backup["host"] = value(host);
        }
        backup["snapshots"] = Item::ArrayOfTables(snapshots.collect());
        primary["backup"] = Item::Table(backup);
        primary["repository"] = self.repository(Role::Primary, section(Question::WhereSaved));
        let forget = (!self.keep.rules().is_empty()).then(|| {
            let mut forget = section(Question::HowLongKeep);
            for (keep, n) in self.keep.rules() {
                forget[keep] = value(*n);
            }
            Item::Table(forget)
        });
        let target = (self.target != Kind::None).then(|| {
            let name = self.profile_name(Role::Target);
            let mut copy = section(Question::WhereCopied);
            copy["targets"] = value([name.as_str()].into_iter().collect::<Array>());
            primary["copy"] = Item::Table(copy);
            let mut target = DocumentMut::new();
            target["repository"] = self.repository(Role::Target, section(Question::WhereCopied));
            // Kept as long as the backups, and checked and pruned on their schedules, which it inherits.
            if let Some(forget) = forget.clone().filter(|_| !self.immutable(Role::Target)) {
                target["forget"] = forget;
            }
            let env: Table = self.env(Role::Target).collect();
            if !env.is_empty() {
                target["global"] = implicit([("env", Item::Table(env))]);
            }
            (format!("{name}.toml"), target.to_string())
        });
        if let Some(forget) = forget.filter(|_| !self.immutable(Role::Primary)) {
            primary["forget"] = forget;
        }
        let mut vars = Table::new();
        vars.decor_mut().set_prefix("\n");
        for (setting, step, every, question) in self.schedules() {
            if let Some(cron) = self.cron(step, every) {
                vars[setting.key] = value(cron);
                answering(&mut vars, setting.key, question);
            }
        }
        // Written so that users find how much each check reads.
        if let (true, Some(subsets)) = (vars.contains_key(settings::CHECK.key), settings::CHECK_SUBSETS.default) {
            vars[settings::CHECK_SUBSETS.key] = value(subsets);
        }
        if !self.notify.trim().is_empty() {
            vars[settings::NOTIFY.key] = value(self.notify.trim());
            answering(&mut vars, settings::NOTIFY.key, Question::WhereNotify);
        }
        // Rustic sets only the backup profile's variables for a copy, and the target's for its own commands.
        vars.extend(self.env(Role::Primary).chain(self.env(Role::Target)));
        primary["global"] = implicit([("env", Item::Table(vars))]);
        let primary = format!("# {}\n{primary}", crate::i18n::t!("first-run-guide", url = RUSTIC_CONFIG_DOCS));
        std::iter::once((format!("{}.toml", self.profile_name(Role::Primary)), primary)).chain(target).collect()
    }
}

#[cfg(feature = "server")]
impl Setup {
    /// Refuses a repository folder that a source holds or that holds a source. Refuses one with other files but no
    /// repository's `config` too. The folder of an existing repository is kept to back up into it again.
    fn check_folders(&self) -> Result<(), Error> {
        use std::path::{Path, PathBuf};
        // As written and as the disk resolves it, up to its nearest folder that exists: `/tmp` is a link to
        // `/private/tmp` on macOS, and `/users/new` is `/Users/new` on a disk that ignores case.
        let resolved = |p: &Path| -> Option<PathBuf> {
            let base = p.ancestors().find_map(|a| Some((a, a.canonicalize().ok()?)))?;
            Some(base.1.join(p.strip_prefix(base.0).ok()?))
        };
        let forms = |p: &str| [Some(Path::new(p).to_path_buf()), resolved(Path::new(p))];
        let sources = self.sources();
        for folder in <Role as strum::IntoEnumIterator>::iter().filter_map(|role| self.folder(role)) {
            for source in &sources {
                let nested = forms(&folder)
                    .iter()
                    .flatten()
                    .any(|f| forms(source).iter().flatten().any(|s| f.starts_with(s) || s.starts_with(f)));
                if nested {
                    return Err(Error::Inside);
                }
            }
            let dir = Path::new(&folder);
            let others = std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some());
            if others && !dir.join("config").is_file() {
                return Err(Error::NotEmpty { folder });
            }
        }
        Ok(())
    }
}

/// The folder the form saves to, if this process may write in it.
#[cfg(feature = "server")]
pub(crate) fn dir() -> Option<PathBuf> {
    profile::own_dir().filter(|d| sys::writable(d))
}

/// Saves the profiles in [`dir`], and returns it. Never overwrites a file.
#[cfg(feature = "server")]
pub(crate) fn save(setup: &Setup) -> anyhow::Result<PathBuf> {
    use std::{fs, io::Write};
    setup.check()?;
    setup.check_folders()?;
    let dir = dir().ok_or(Error::NoDir)?;
    fs::create_dir_all(&dir)?;
    let profiles = setup.profiles();
    if let Some(path) = profiles.iter().map(|(file, _)| dir.join(file)).find(|p| p.exists()) {
        return Err(Error::Exists { path }.into());
    }
    // The target first: if the primary exists, the target left behind backs up nothing.
    for (file, text) in profiles.iter().rev() {
        let path = dir.join(file);
        match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(Error::Exists { path }.into()),
            opened => opened?.write_all(text.as_bytes())?,
        }
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::{Choice, Error, Every, Keep, Kind, Role, Setup};
    use crate::{profile, settings};

    /// The default answers, with a target of `kind` and its values.
    fn target(kind: Kind, values: &[(&str, &str)]) -> Setup {
        let mut setup = Setup { password: "pw".into(), target_password: "pw".into(), target: kind, ..Setup::default() };
        setup.values.extend(values.iter().map(|(k, v)| (format!("target.{}.{k}", kind.id()), (*v).to_owned())));
        setup
    }

    /// rustic would read a relative folder from wherever it runs, and back up nothing there without failing.
    /// Every field and choice of every kind has its message.
    #[test]
    fn every_field_has_text() {
        for field in <Kind as strum::IntoEnumIterator>::iter().flat_map(Kind::fields) {
            assert!(!super::Field::text_of(field.label).is_empty(), "no message field-{}", field.label);
            for choice in field.choices {
                assert!(crate::i18n::find(&format!("field-choice-{choice}"), []).is_some(), "no message for {choice}");
            }
        }
    }

    #[test]
    fn folders_are_full_paths_with_tilde_as_the_home_folder() {
        let home = etcetera::home_dir().unwrap().display().to_string();
        let mut setup = target(Kind::Local, &[("repository", "~/usb")]);
        setup.values.insert("source.0".into(), "~/Documents".into());
        assert_eq!(setup.sources(), [format!("{home}/Documents")]);
        assert!(setup.profiles()[1].1.contains(&format!("{home}/usb")), "{}", setup.profiles()[1].1);
        assert!(setup.check().is_ok());
        let prefixed = target(Kind::Local, &[("repository", "local:~/usb")]);
        let saved = profile::parse("Documents-local-copy", &prefixed.profiles()[1].1).repository;
        assert_eq!(saved, Some(format!("{home}/usb")));
        let with = |key: &str, path: &str| {
            let mut setup = setup.clone();
            setup.values.insert(key.into(), path.into());
            setup.check()
        };
        assert!(matches!(with("source.1", "Documents"), Err(Error::Relative { path }) if path == "Documents"));
        assert!(matches!(with("source.1", "~me/x"), Err(Error::Relative { .. })));
        assert!(matches!(with("target.local.repository", "backup"), Err(Error::Relative { .. })));
    }

    /// rustic backs up a repository inside its sources into itself. It mixes a new one in with other files.
    #[cfg(feature = "server")]
    #[test]
    #[expect(clippy::disallowed_methods, reason = "lays out folders to back up into")]
    fn a_backup_folder_is_neither_nested_nor_shared() {
        let dir = crate::sys::test_dir("folders");
        let path = |parts: &[&str]| parts.iter().fold(dir.clone(), |p, part| p.join(part)).display().to_string();
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::create_dir_all(dir.join("full")).unwrap();
        std::fs::write(dir.join("full").join("notes.txt"), "mine").unwrap();
        std::fs::create_dir_all(dir.join("repo")).unwrap();
        std::fs::write(dir.join("repo").join("config"), "").unwrap();
        let into = |folder: &str| {
            let mut setup = target(Kind::None, &[]);
            setup.values.insert("source.0".into(), path(&["docs"]));
            setup.values.insert("primary.local.repository".into(), folder.to_owned());
            setup.check_folders()
        };
        assert!(matches!(into(&path(&["docs", "backup"])), Err(Error::Inside)));
        assert!(matches!(into(&dir.display().to_string()), Err(Error::Inside)), "it would hold the source");
        assert!(matches!(into(&path(&["full"])), Err(Error::NotEmpty { .. })));
        assert!(into(&path(&["repo"])).is_ok(), "an existing repository");
        assert!(into(&path(&["new", "backup"])).is_ok());
        assert!(
            matches!(into(&format!("local:{}", path(&["docs", "backup"]))), Err(Error::Inside)),
            "as rustic reads it"
        );
        // On a disk that ignores case, a new folder written in another case is still in the source.
        if dir.join("DOCS").exists() {
            assert!(matches!(into(&path(&["DOCS", "new", "backup"])), Err(Error::Inside)));
        }
    }

    /// Each folder is its own entry, with its own snapshots. One given twice is backed up once.
    #[test]
    fn each_folder_is_its_own_entry() {
        let mut setup = target(Kind::None, &[]);
        setup.values.insert("source.0".into(), "/home/me/Documents".into());
        setup.values.insert("source.1".into(), "/home/me/Desktop".into());
        setup.values.insert("source.2".into(), "/home/me/Documents".into());
        let text = &setup.profiles()[0].1;
        let entries: Vec<Vec<String>> =
            profile::parse("Documents", text).sources.into_iter().map(|s| s.sources).collect();
        assert_eq!(entries, [["/home/me/Documents"], ["/home/me/Desktop"]], "{text}");
        assert_eq!(text.matches("# What do you want to back up?").count(), 1, "{text}");
    }

    #[test]
    fn paths_copied_from_file_explorer_lose_their_quotes() {
        let values = [("source.0", r#""C:\Users\me\Documents""#), ("source.10", ""), ("source.2", "  /home/me  ")];
        let setup =
            Setup { values: values.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(), ..Setup::default() };
        assert_eq!(setup.sources(), [r"C:\Users\me\Documents", "/home/me"]);
        let local = target(Kind::Local, &[("repository", r#""D:\backup""#)]);
        let (_, text) = &local.profiles()[1];
        assert!(text.contains(r"'D:\backup'"), "{text}");
    }

    #[test]
    fn a_repository_keeps_as_long_as_the_backups_unless_it_refuses_deletes() {
        let keeps = |setup: &Setup| profile::parse("Documents-rest", &setup.profiles()[1].1).has_keep;
        let rest = target(Kind::Rest, &[("repository", "http://h/r")]);
        assert!(rest.profiles()[1].1.contains("keep-last = 10"), "{:?}", rest.profiles());
        assert!(!keeps(&Setup { keep: Keep::All, ..rest }));
        assert!(!keeps(&target(Kind::Rest, &[("repository", "http://h/r"), ("immutable", "on")])));
        let object_lock = target(Kind::S3, &[("endpoint", "https://s3"), ("immutable", "on")]);
        let text = &object_lock.profiles()[1].1;
        assert!(!text.contains("forget") && !text.contains("immutable"), "{text}");
        let mut locked = target(Kind::Rest, &[("repository", "http://h/r")]);
        locked.primary = Kind::S3;
        locked.values.insert("primary.s3.immutable".into(), "on".into());
        let profiles = locked.profiles();
        assert!(!profiles[0].1.contains("keep-") && profiles[1].1.contains("keep-last"), "{profiles:?}");
    }

    #[test]
    fn profiles_read_back_as_written() {
        let setup = target(Kind::Sftp, &[("user", "me"), ("endpoint", "ssh://nas:22")]);
        let profiles = setup.profiles();
        let [(file, primary), (target_file, target_text)] = profiles.as_slice() else { panic!("{profiles:?}") };
        assert_eq!((file.as_str(), target_file.as_str()), ("Documents-local.toml", "Documents-sftp.toml"));
        assert_eq!(Setup { target: Kind::None, ..setup.clone() }.profiles()[0].0, "Documents.toml");
        let same = Setup { target: Kind::Local, ..setup.clone() };
        assert_eq!(same.profile_name(Role::Target), "Documents-local-copy");
        let host = crate::sys::hostname().split('.').next().unwrap_or_default();
        assert!(primary.contains(&format!("[backup]\nhost = \"{host}\"")), "{primary}");
        let p = profile::parse("Documents-local", primary);
        assert!(p.is_backup_profile() && p.has_keep && p.targets == ["Documents-sftp"]);
        assert!(primary.contains("keep-last = 10"), "{primary}");
        let month = &Setup { keep: Keep::Month, ..setup.clone() }.profiles()[0].1;
        assert!(month.contains("keep-daily = 30") && !month.contains("keep-monthly"), "{month}");
        let all = &Setup { keep: Keep::All, ..setup.clone() }.profiles()[0].1;
        assert!(!profile::parse("Documents-local", all).has_keep, "{all}");
        let t = profile::parse("Documents-sftp", target_text);
        assert_eq!(t.repository.as_deref(), Some("opendal:sftp"));
        assert!(target_text.contains("endpoint = \"ssh://nas:22\"") && !target_text.contains("root"), "{target_text}");
        assert!(target_text.contains("known_hosts_strategy = \"accept\""), "{target_text}");
        let picked =
            target(Kind::Sftp, &[("user", "me"), ("endpoint", "ssh://nas:22"), ("known_hosts_strategy", "maybe")]);
        assert!(matches!(picked.check(), Err(Error::NotAChoice { role: Role::Target, .. })));
        assert!(primary.contains(&format!("{} = \"@daily\"", settings::BACKUP.key)), "{primary}");
        assert!(primary.contains(&format!("{} = \"10\"", settings::CHECK_SUBSETS.key)), "{primary}");
        // Prunes follow what is kept: monthly for a year, weekly for a month, never for everything.
        assert!(primary.contains(&format!("{} = \"@monthly\"", settings::PRUNE.key)), "{primary}");
        let month = Setup { keep: Keep::Month, ..setup.clone() }.profiles();
        assert!(month[0].1.contains(&format!("{} = \"@weekly\"", settings::PRUNE.key)), "{}", month[0].1);
        let manual = Setup { keep: Keep::All, every: Every::Manual, ..setup.clone() }.profiles();
        assert!(!manual[0].1.contains(settings::PRUNE.key), "{}", manual[0].1);
        assert!(!manual[0].1.contains(settings::CHECK_SUBSETS.key), "{}", manual[0].1);
        let mut custom = Setup { every: Every::Custom, ..setup.clone() };
        assert!(matches!(custom.check(), Err(Error::NoCustom)));
        let signal = Setup { notify: "signal://localhost/+1 555 123 4567 macosx://".into(), ..setup.clone() };
        assert!(signal.check().is_ok(), "{:?}", signal.check());
        for notify in ["tgram://{bot_token}", "tgram://only-a-token"] {
            assert!(
                matches!(Setup { notify: notify.into(), ..setup.clone() }.check(), Err(Error::Notify(_))),
                "{notify}"
            );
        }
        custom.values.insert("every.custom".into(), "0 3 * * 1".into());
        let custom = &custom.profiles()[0].1;
        assert!(custom.contains(&format!("{} = \"0 3 * * 1\"", settings::BACKUP.key)), "{custom}");
        // Checks follow the backups: weekly for a custom schedule and monthly for weekly backups.
        assert!(custom.contains(&format!("{} = \"@weekly\"", settings::CHECK.key)), "{custom}");
        let weekly = &Setup { every: Every::Weekly, ..setup.clone() }.profiles()[0].1;
        assert!(weekly.contains(&format!("{} = \"@monthly\"", settings::CHECK.key)), "{weekly}");
        assert!(<Every as strum::IntoEnumIterator>::iter().all(|e| Every::try_from(e.id()).is_ok_and(|c| c == e)));
        assert!(setup.check().is_ok());
        assert!(matches!(
            Setup { target: Kind::Rclone, ..setup.clone() }.check(),
            Err(Error::Incomplete { role: Role::Target, field: "remote" })
        ));
        assert!(matches!(
            Setup { primary: Kind::S3, ..setup.clone() }.check(),
            Err(Error::Incomplete { role: Role::Primary, field: "server" })
        ));
        assert!(matches!(Setup { name: "../x".into(), ..setup.clone() }.check(), Err(Error::Name)));
        assert!(matches!(Setup { name: "-rf".into(), ..setup }.check(), Err(Error::Name)));
        assert!(<Kind as strum::IntoEnumIterator>::iter().all(|k| Kind::try_from(k.id()).is_ok_and(|c| c == k)));
        let rest = target(Kind::Rest, &[("repository", "http://h/r")]);
        assert!(rest.profiles()[1].1.contains("repository = \"rest:http://h/r\""));
        let rclone = target(Kind::Rclone, &[("repository", "gd:b"), ("RCLONE_CONFIG_PASS", "x")]);
        let pass = "RCLONE_CONFIG_PASS = \"x\"\n";
        let profiles = rclone.profiles();
        assert!(profiles.iter().all(|(_, text)| text.contains(pass) && !text.contains("options")), "{profiles:?}");
        // A remote primary, with the target's values left over from another choice.
        let mut remote = Setup { primary: Kind::Rclone, ..rclone };
        remote.values.insert("primary.rclone.repository".into(), "gd:primary".into());
        let profiles = remote.profiles();
        assert!(profiles[0].1.contains("repository = \"rclone:gd:primary\""), "{profiles:?}");
        assert!(remote.check().is_ok());
        let other = target(Kind::Rclone, &[("repository", "my nas:backup")]);
        assert_eq!(other.profile_name(Role::Target), "Documents-my-nas");
        let unnamed = target(Kind::Rclone, &[("repository", ":local:/backup")]);
        assert_eq!(unnamed.profile_name(Role::Target), "Documents-rclone");
        let gdrive = target(Kind::Gdrive, &[("repository", "gdrive:backup")]);
        let profiles = gdrive.profiles();
        assert_eq!(profiles[1].0, "Documents-gdrive.toml");
        assert!(profiles[1].1.contains("repository = \"rclone:gdrive:backup\""), "{profiles:?}");
    }
}
