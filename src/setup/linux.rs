//! Linux's defaults for the first collection.

/// What root's or an administrator's first collection backs up: the system's settings.
pub(super) const ROOT_SOURCE: &str = "/etc";

/// The notification URL of this desktop.
pub(super) const DESKTOP: &str = "dbus://";

/// Whether rustic's sftp runs here.
#[cfg(feature = "server")]
pub(super) const SFTP: bool = true;

/// What a user types here, of what they would type on Windows, macOS, or another system.
pub(super) const fn native<'a>(windows: &'a str, macos: &'a str, other: &'a str) -> &'a str {
    _ = (windows, macos, other);
    other
}
