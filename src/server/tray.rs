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
    engine::Unconfigured,
    log::{self, Level, Logged, Message as _},
    sys,
};

/// Told when the tray must be drawn again although the health is the same: a switch was flipped, or the bar
/// turned light or dark on a system that does not redraw the icon itself.
static REDRAW: tokio::sync::Notify = tokio::sync::Notify::const_new();
/// How long to gather changes, as runs report progress, before the health is computed again.
const SETTLE: Duration = Duration::from_secs(1);

/// A line in the menu: a collection, or why there is none.
#[derive(Clone, Debug, PartialEq)]
struct Row {
    label: String,
    level: Level,
    /// The page it opens, or none for a profile that cannot be read.
    url: Option<String>,
}

/// Whether its user runs it: from a terminal, by the service manager of their session, or to `open` the
/// web UI, as the desktop entry does. Not as root, nor under another program, such as the tests.
#[cfg_attr(test, mutants::skip)] // Reads this process's terminal and session, which no test has: see `attended_by`.
fn attended(open: bool) -> bool {
    use std::io::IsTerminal;
    attended_by(sys::is_root(), open, std::io::stdout().is_terminal(), managed())
}

/// [`attended`], from whether it runs as root, to open the web UI, from a terminal, or by the session's service
/// manager.
#[expect(clippy::fn_params_excessive_bools, reason = "each is a yes or no that the system gives")]
fn attended_by(root: bool, open: bool, terminal: bool, managed: bool) -> bool {
    !root && (open || terminal || managed)
}

/// Runs `serve` beside the tray icon, which shows only for a server its user runs: see [`attended`].
pub(super) fn beside(
    open: bool,
    serve: impl FnOnce(Option<Tray>) -> anyhow::Result<()> + Send + 'static,
) -> anyhow::Result<()> {
    imp::beside(attended(open), serve)
}

/// A setting that the menu switches on and off. Its item's id is its name, which parses back to it.
#[derive(
    Clone, Copy, Debug, PartialEq, crate::i18n::Fluent, strum::EnumString, strum::IntoStaticStr, strum::EnumIter,
)]
enum Switch {
    AtLogin,
    #[cfg(feature = "self-update")]
    AutoUpdate,
}

impl Switch {
    /// Each switch with its state. Auto-update is left out when the package manager updates rusticlone.
    #[cfg_attr(test, mutants::skip)] // Asks the service manager and reads the settings file.
    fn states() -> Vec<(Self, Option<bool>)> {
        Self::iter().filter(|&s| s.shown()).map(|s| (s, s.state())).collect()
    }

    #[cfg_attr(test, mutants::skip)] // Tells how this executable was installed.
    fn shown(self) -> bool {
        match self {
            Self::AtLogin => true,
            #[cfg(feature = "self-update")]
            Self::AutoUpdate => crate::install::update::replaceable(),
        }
    }

    /// `None` when it cannot be switched: a server that is not installed does not start at login.
    #[cfg_attr(test, mutants::skip)] // Asks the service manager and reads the settings file.
    fn state(self) -> Option<bool> {
        match self {
            Self::AtLogin => crate::install::autostart::at_login(),
            #[cfg(feature = "self-update")]
            Self::AutoUpdate => Some(crate::settings::on_in_env_file(&crate::settings::AUTO_UPDATE)),
        }
    }

    /// Flips it, then has the menu drawn again.
    #[cfg_attr(test, mutants::skip)] // Changes the service manager's units or the settings file.
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

/// The menu's lines: each collection's health, which opens its page, in the order of the pages; the
/// profiles that cannot be read follow, greyed out. Without any, why, which opens the setup.
fn rows(base: &str, health: Vec<(String, Level)>, pages: Vec<String>, unconfigured: Option<Unconfigured>) -> Vec<Row> {
    let pages = pages.into_iter().map(Some).chain(std::iter::repeat(None));
    let rows = health.into_iter().zip(pages).map(|((title, level), name)| Row {
        label: format!("{title}: {}", level.word()),
        level,
        url: name.map(|n| format!("{base}{}", Route::Collection(&n))),
    });
    let unconfigured = unconfigured.map(|u| Row {
        label: u.to_string(),
        level: u.level(),
        url: Some(format!("{base}{}", Route::Setup)),
    });
    rows.chain(unconfigured).collect()
}

impl Tray {
    /// Shows the health whenever pages are told it may have changed. `url`: the web UI; `login`: the
    /// same, logged in.
    #[expect(clippy::disallowed_methods, reason = "a burst of changes settles before the icon is drawn again")]
    #[cfg_attr(test, mutants::skip)] // Waits for the pages' changes, and draws the icon on the desktop.
    pub(super) async fn follow(mut self, app: Arc<App>, url: String, login: String) {
        let base = url.trim_end_matches('/').to_owned();
        let mut changes = app.pages.changes.subscribe();
        let (mut shown, mut switches) = (None, Vec::new());
        loop {
            let (engine, base) = (app.engine(), base.clone());
            let rows: Vec<Row> = tokio::task::spawn_blocking(move || {
                let health = engine.status().into_iter().map(|c| (c.title, c.level)).collect();
                let pages = engine.collections().iter().map(|c| c.name().to_owned()).collect();
                rows(&base, health, pages, engine.unconfigured())
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

    /// The icon shows for a server its user runs: to open the web UI, from a terminal, or by the service manager of
    /// their session. Never as root, nor under another program.
    #[test]
    fn the_icon_shows_for_a_server_its_user_runs() {
        let user = [(true, false, false), (false, true, false), (false, false, true)];
        for (open, terminal, managed) in user {
            assert!(attended_by(false, open, terminal, managed), "{open} {terminal} {managed}");
            assert!(!attended_by(true, open, terminal, managed), "as root");
        }
        assert!(!attended_by(false, false, false, false), "under another program");
    }

    /// A click names the item by its id, which is not its label: that one is translated.
    #[test]
    fn each_switch_is_found_by_its_items_id() {
        for switch in Switch::iter() {
            let id: &str = switch.into();
            assert!(id.parse::<Switch>().ok() == Some(switch), "{id}");
        }
    }

    /// Each collection's health opens its page, and a profile that cannot be read follows, greyed out.
    /// Without any collection, the line says why and opens the setup.
    #[test]
    fn each_collection_is_a_line_that_opens_its_page() {
        let row = |label: &str, level, url: Option<&str>| Row { label: label.into(), level, url: url.map(Into::into) };
        let health = vec![("Docs".into(), Level::Success), ("broken.toml".into(), Level::Failure)];
        let lines = [
            row("Docs: healthy", Level::Success, Some("http://web/collections/Docs")),
            row("broken.toml: failing", Level::Failure, None),
        ];
        assert_eq!(rows("http://web", health, vec!["Docs".into()], None), lines);
        let new = "No collection found: open Rusticlone to create your first one";
        let setup = [row(new, Level::Warning, Some("http://web/setup"))];
        assert_eq!(rows("http://web", vec![], vec![], Some(Unconfigured::New)), setup);
    }
}
