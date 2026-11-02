//! What the tray's menu says, and what its items do.

use super::Row;
use crate::{APP_NAME, log::Level};

pub(super) fn level(rows: &[Row]) -> Level {
    rows.iter().map(|r| r.level).max().unwrap_or_default()
}

pub(super) fn tooltip(level: Level) -> String {
    format!("{APP_NAME}: {}", level.word())
}

pub(super) fn open_label() -> String {
    format!("Open {APP_NAME}")
}

pub(super) fn version_label() -> String {
    format!("{APP_NAME} {}", crate::server::web::VERSION)
}

/// The server stops as on SIGTERM. Installed, it starts again at the next login.
pub(super) fn quit_label() -> String {
    crate::i18n::t!("tray-quit")
}

pub(super) fn quit() {
    crate::server::service::STOP.notify_one();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The menu's worst collection colors the icon.
    #[test]
    fn the_worst_row_sets_the_level() {
        let row = |level| Row { label: String::new(), level, url: None };
        assert_eq!(level(&[]), Level::Success);
        assert_eq!(level(&[row(Level::Warning), row(Level::Failure), row(Level::Success)]), Level::Failure);
    }
}
