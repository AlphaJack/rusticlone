//! The values each kind of repository needs.

#[cfg(feature = "server")]
use super::Group;
use super::Kind;

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
}

impl Key {
    pub(super) fn option(self) -> Option<&'static str> {
        if let Self::Option(key) = self { Some(key) } else { None }
    }

    pub(super) fn env(self) -> Option<&'static str> {
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
    pub(super) fn text_of(label: &str) -> String {
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
pub(super) const RUSTIC_CONFIG_DOCS: &str = "https://rustic.cli.rs/docs/commands/init/configuration_file.html";

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

/// `root`: where in the service the repository is.
const ROOT: Field = Field { required: false, ..field(Key::Option("root"), "path", "/backup") };

/// Each kind's fields go from where to who: server, bucket, path, user, then secrets.
impl Kind {
    /// What the repository starts with, followed by the [`Key::Repository`] field if it has one.
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
            Self::Rest => const { &[field(Key::Repository, "url", "https://me:password@backup.example.com:8000/me")] },
            Self::Gdrive => const { &rclone("gdrive:backup") },
            Self::Onedrive => const { &rclone("onedrive:backup") },
            Self::Dropbox => const { &rclone("dropbox:backup") },
            Self::Rclone => const { &rclone("remote:backup") },
        }
    }
}
