//! macOS's defaults for the first collection.

/// What root's or an administrator's first collection backs up: the system's settings: `/etc` is a symlink here, which a backup would save as one.
pub(super) const ROOT_SOURCE: &str = "/private/etc";

/// The notification URL of this desktop.
pub(super) const DESKTOP: &str = "macosx://";

/// Whether rustic's sftp runs here.
#[cfg(feature = "server")]
pub(super) const SFTP: bool = true;

/// What a user types here, of what they would type on Windows, macOS, or another system.
pub(super) const fn native<'a>(windows: &'a str, macos: &'a str, other: &'a str) -> &'a str {
    _ = (windows, macos, other);
    macos
}
