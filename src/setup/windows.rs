//! Windows's defaults for the first collection.

/// What root's or an administrator's first collection backs up: every account's files, which the system service reads.
pub(super) const ROOT_SOURCE: &str = r"C:\Users";

/// The notification URL of this desktop.
pub(super) const DESKTOP: &str = "windows://";

/// Whether rustic's sftp runs here.
#[cfg(feature = "server")]
pub(super) const SFTP: bool = false;

/// What a user types here, of what they would type on Windows, macOS, or another system.
pub(super) const fn native<'a>(windows: &'a str, macos: &'a str, other: &'a str) -> &'a str {
    _ = (windows, macos, other);
    windows
}
