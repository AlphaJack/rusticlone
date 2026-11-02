//! The tray icon, in macOS's menu bar, Linux's panels and Windows's taskbar: the logo in black or white, with a
//! colored dot when a collection needs attention, and a menu that opens the web UI. It shows only for a server
//! that its user runs.

#[cfg(any(target_os = "macos", windows))]
mod icon;
#[cfg(target_os = "linux")]
mod linux;
mod logo;
#[cfg(target_os = "macos")]
mod macos;
mod menu;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(windows)]
use windows as imp;

pub(super) use imp::Tray;
use imp::managed;

use std::{sync::Arc, time::Duration};

use strum::IntoEnumIterator as _;

use super::{App, web::Route};
use crate::{
    log::{self, Level, Logged, Message as _},
    sys,
};

/// Told when the tray must be drawn again although the health is the same: a switch was flipped, or the bar
/// turned light or dark on a system that does not redraw the icon itself.
static REDRAW: tokio::sync::Notify = tokio::sync::Notify::const_new();
/// How long to gather changes, as runs report progress, before the health is computed again.
const SETTLE: Duration = Duration::from_secs(1);

/// A line in the menu: a collection, or why there is none.
#[derive(Clone, PartialEq)]
struct Row {
    label: String,
    level: Level,
    /// The page it opens, or none for a profile that cannot be read.
    url: Option<String>,
}

/// Whether its user runs it: from a terminal, by the service manager of their session, or to `open` the
/// web UI, as the desktop entry does. Not as root, nor under another program, such as the tests.
fn attended(open: bool) -> bool {
    use std::io::IsTerminal;
    !sys::is_root() && (open || std::io::stdout().is_terminal() || managed())
}

/// Runs `serve` beside the tray icon, which shows only for a server its user runs: see [`attended`].
pub(super) fn beside(
    open: bool,
    serve: impl FnOnce(Option<Tray>) -> anyhow::Result<()> + Send + 'static,
) -> anyhow::Result<()> {
    imp::beside(attended(open), serve)
}

/// A setting that the menu switches on and off. Its item's id is its name, which parses back to it.
#[derive(Clone, Copy, PartialEq, crate::i18n::Fluent, strum::EnumString, strum::IntoStaticStr, strum::EnumIter)]
enum Switch {
    AtLogin,
    #[cfg(feature = "self-update")]
    AutoUpdate,
}

impl Switch {
    /// Each switch with its state. Auto-update is left out when the package manager updates rusticlone.
    fn states() -> Vec<(Self, Option<bool>)> {
        Self::iter().filter(|&s| s.shown()).map(|s| (s, s.state())).collect()
    }

    /// Root's server starts with the computer, as a system service; Windows's always at the user's login.
    fn label(self) -> String {
        match self {
            Self::AtLogin => self.to_string(),
            #[cfg(feature = "self-update")]
            Self::AutoUpdate => self.to_string(),
        }
    }

    fn shown(self) -> bool {
        match self {
            Self::AtLogin => true,
            #[cfg(feature = "self-update")]
            Self::AutoUpdate => crate::install::update::replaceable(),
        }
    }

    /// `None` when it cannot be switched: a server that is not installed does not start at login.
    fn state(self) -> Option<bool> {
        match self {
            Self::AtLogin => crate::install::autostart::at_login(),
            #[cfg(feature = "self-update")]
            Self::AutoUpdate => Some(crate::settings::on_in_env_file(&crate::settings::AUTO_UPDATE)),
        }
    }

    /// Flips it, then has the menu drawn again.
    fn toggle(self) {
        let on = self.state() != Some(true);
        let done = match self {
            Self::AtLogin => crate::install::autostart::set_at_login(on),
            #[cfg(feature = "self-update")]
            Self::AutoUpdate => {
                crate::settings::write_env_file(&crate::settings::AUTO_UPDATE, &on.to_string()).map(drop)
            }
        };
        if let Err(e) = done {
            log::event(None, &Logged::Error { error: format!("{e:#}") });
        }
        REDRAW.notify_one();
    }
}

impl Tray {
    /// Shows the health whenever pages are told it may have changed. `url`: the web UI; `login`: the
    /// same, logged in.
    #[expect(clippy::disallowed_methods, reason = "a burst of changes settles before the icon is drawn again")]
    pub(super) async fn follow(mut self, app: Arc<App>, url: String, login: String) {
        let base = url.trim_end_matches('/').to_owned();
        let mut changes = app.pages.changes.subscribe();
        let (mut shown, mut switches) = (None, Vec::new());
        loop {
            let (engine, base) = (app.engine(), base.clone());
            let rows: Vec<Row> = tokio::task::spawn_blocking(move || {
                // Listed first, in the same order; the profiles that cannot be read follow.
                let collections = engine.collections();
                let pages = collections.iter().map(|c| Some(c.name())).chain(std::iter::repeat(None));
                let rows = engine.status().into_iter().zip(pages).map(|(c, name)| Row {
                    label: format!("{}: {}", c.title, c.level.word()),
                    level: c.level,
                    url: name.map(|n| format!("{base}{}", Route::Collection(n))),
                });
                let unconfigured = engine.unconfigured().map(|u| Row {
                    label: u.to_string(),
                    level: u.level(),
                    url: Some(format!("{base}{}", Route::Setup)),
                });
                rows.chain(unconfigured).collect()
            })
            .await
            .unwrap_or_default();
            // Read again only when flipped, as each read may run a service manager.
            if shown.is_none() {
                switches = tokio::task::spawn_blocking(Switch::states).await.unwrap_or_default();
            }
            if shown.as_ref() != Some(&rows) {
                self.show(&login, &rows, &switches).await;
                shown = Some(rows);
            }
            tokio::select! {
                changed = changes.changed() => if changed.is_err() { return },
                () = REDRAW.notified() => shown = None,
            }
            tokio::time::sleep(SETTLE).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A click names the item by its id, which is not its label: that one is translated.
    #[test]
    fn each_switch_is_found_by_its_items_id() {
        for switch in Switch::iter() {
            let id: &str = switch.into();
            assert!(id.parse::<Switch>().ok() == Some(switch), "{id}");
        }
    }
}
