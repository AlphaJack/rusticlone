//! Windows's defaults for the first collection.

/// What root's or an administrator's first collection backs up: every account's files, which the system service reads.
pub(super) const ROOT_SOURCE: &str = r"C:\Users";

/// The notification URL of this desktop.
pub(super) const DESKTOP: &str = "windows://";

/// The kinds of repository that rustic does not support here.
#[cfg(feature = "server")]
pub(super) const UNSUPPORTED: &[super::Kind] = &[super::Kind::Sftp];

/// What a user types here, of what they would type on Windows, macOS, or another system.
pub(super) const fn native<'a>(windows: &'a str, macos: &'a str, other: &'a str) -> &'a str {
    _ = (windows, macos, other);
    windows
}
