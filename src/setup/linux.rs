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

#[cfg(test)]
mod tests {
    /// A user types here what they would on other systems than Windows and macOS.
    #[test]
    fn a_user_types_what_other_systems_take() {
        assert_eq!(super::native(r"D:\backup", "/Volumes/USB/backup", "/mnt/usb/backup"), "/mnt/usb/backup");
    }
}
