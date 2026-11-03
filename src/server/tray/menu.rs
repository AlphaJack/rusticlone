//! What the tray's menu says, and what its items do.

use super::{Row, Switch};
use crate::{APP_NAME, log::Level};

/// An item of the menu, which each system draws with its own widgets.
#[derive(Debug, PartialEq)]
pub(super) enum Item {
    /// Greyed out when a click does nothing.
    Entry {
        label: String,
        click: Option<Click>,
    },
    /// A switch, greyed out when it cannot be switched.
    Check {
        label: String,
        enabled: bool,
        checked: bool,
        click: Click,
    },
    Separator,
}

/// What a click on an item does.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Click {
    /// Opens the page in the browser.
    Open(String),
    Toggle(Switch),
    /// Stops the server as on SIGTERM. Installed, it starts again at the next login.
    Quit,
}

impl Click {
    #[cfg_attr(test, mutants::skip)] // Opens the browser, flips a setting or stops the server.
    pub(super) fn run(self) {
        match self {
            Self::Open(url) => _ = crate::server::open_on_desktop(&url),
            Self::Toggle(switch) => switch.toggle(),
            Self::Quit => crate::server::service::STOP.notify_one(),
        }
    }
}

/// The menu: the version, the web UI logged in, the collections, the switches, and quit.
pub(super) fn items(login: &str, rows: &[Row], switches: &[(Switch, Option<bool>)]) -> Vec<Item> {
    let entry = |label, url: Option<&str>| Item::Entry { label, click: url.map(|u| Click::Open(u.to_owned())) };
    let switches = switches.iter().map(|&(switch, state)| Item::Check {
        label: switch.label(),
        enabled: state.is_some(),
        checked: state == Some(true),
        click: Click::Toggle(switch),
    });
    let version = format!("{APP_NAME} {}", crate::server::web::VERSION);
    let quit = Item::Entry { label: crate::i18n::t!("tray-quit"), click: Some(Click::Quit) };
    [entry(version, None), entry(crate::i18n::t!("tray-open"), Some(login)), Item::Separator]
        .into_iter()
        .chain(rows.iter().map(|r| entry(r.label.clone(), r.url.as_deref())))
        .chain([Item::Separator])
        .chain(switches)
        .chain([Item::Separator, quit])
        .collect()
}

pub(super) fn level(rows: &[Row]) -> Level {
    rows.iter().map(|r| r.level).max().unwrap_or_default()
}

pub(super) fn tooltip(level: Level) -> String {
    format!("{APP_NAME}: {}", level.word())
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

    /// The version, the web UI, a page per collection, each switch as it stands, and quit, in that order.
    #[test]
    fn the_menu_lists_the_web_ui_the_collections_the_switches_and_quit() {
        let row = |label: &str, url: Option<&str>| Row {
            label: label.into(),
            level: Level::Success,
            url: url.map(Into::into),
        };
        let rows = [row("docs: healthy", Some("http://web/docs")), row("broken.toml", None)];
        let switches = [(Switch::AtLogin, Some(true)), (Switch::AtLogin, Some(false)), (Switch::AtLogin, None)];
        let entry = |label: &str, click| Item::Entry { label: label.into(), click };
        let open = |url: &str| Some(Click::Open(url.into()));
        let check = |enabled, checked| Item::Check {
            label: "Start at login".into(),
            enabled,
            checked,
            click: Click::Toggle(Switch::AtLogin),
        };
        let expected = [
            entry(&format!("Rusticlone {}", env!("CARGO_PKG_VERSION")), None),
            entry("Open Rusticlone", open("http://web/login")),
            Item::Separator,
            entry("docs: healthy", open("http://web/docs")),
            entry("broken.toml", None),
            Item::Separator,
            check(true, true),
            check(true, false),
            check(false, false),
            Item::Separator,
            entry("Quit (pause automatic backups)", Some(Click::Quit)),
        ];
        assert_eq!(items("http://web/login", &rows, &switches), expected);
    }

    #[test]
    fn the_tooltip_says_the_health() {
        assert_eq!(tooltip(Level::Warning), "Rusticlone: degraded");
    }
}
