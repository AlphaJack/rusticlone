//! The tray icon, in macOS's menu bar, Linux's panels and Windows's taskbar: the logo in black or white, with a
//! colored dot when a collection needs attention, and a menu that opens the web UI. It shows only for a server
//! that its user runs.

#[cfg(any(target_vendor = "apple", windows))]
mod icon;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_vendor = "apple")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub(super) use linux::{Tray, beside};
#[cfg(target_vendor = "apple")]
pub(super) use macos::{Tray, beside};
#[cfg(windows)]
pub(super) use windows::{Tray, beside};

use std::{sync::Arc, time::Duration};

use strum::IntoEnumIterator as _;

use super::{App, web::Route};
use crate::{
    APP_NAME,
    log::{self, Level, Logged, Message as _},
    sys,
};

/// The logo's opacity, rendered by `cargo xtask logo`: the tray draws it in black or white to suit the theme.
const LOGO: &[u8] = include_bytes!("../../assets/images/logo.alpha");
/// The side of the square that `LOGO` fills, in pixels.
#[expect(clippy::cast_possible_truncation, reason = "a few dozen pixels")]
const SIDE: u32 = LOGO.len().isqrt() as u32;
/// The dot's radius, and the gap cut around it, in pixels.
const DOT: f32 = 5.0;
const GAP: f32 = 2.0;
/// Red, green and blue.
type Rgb = [u8; 3];
/// The dot's colors, as `--warning` and `--failure` in `style.css`.
const WARNING: Rgb = [0xca, 0x8a, 0x04];
const FAILURE: Rgb = [0xdc, 0x26, 0x26];
const WHITE: Rgb = [u8::MAX; 3];
const BLACK: Rgb = [0; 3];
/// Told when the tray must be drawn again although the health is the same: a switch was flipped, or the bar
/// turned light or dark on a system that does not redraw the icon itself.
static REDRAW: tokio::sync::Notify = tokio::sync::Notify::const_new();
/// From a pixel's corner to its center.
const HALF_PIXEL: f32 = 0.5;
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

/// launchd starts its agents.
#[cfg(target_vendor = "apple")]
fn managed() -> bool {
    std::os::unix::process::parent_id() == 1
}

/// Started from the `Run` key, or by `install`.
#[cfg(windows)]
fn managed() -> bool {
    sys::started_alone()
}

/// systemd tells its services where to report readiness.
#[cfg(target_os = "linux")]
#[expect(clippy::disallowed_methods, reason = "systemd's variable, not a setting")]
fn managed() -> bool {
    std::env::var_os("NOTIFY_SOCKET").is_some()
}

fn level(rows: &[Row]) -> Level {
    rows.iter().map(|r| r.level).max().unwrap_or_default()
}

fn tooltip(level: Level) -> String {
    format!("{APP_NAME}: {}", level.word())
}

fn open_label() -> String {
    format!("Open {APP_NAME}")
}

fn version_label() -> String {
    format!("{APP_NAME} {}", super::web::VERSION)
}

/// The server stops as on SIGTERM. Installed, it starts again at the next login.
fn quit_label() -> String {
    crate::i18n::t!("tray-quit")
}

fn quit() {
    super::service::STOP.notify_one();
}

/// A setting that the menu switches on and off. Its label is also its item's id.
#[derive(Clone, Copy, PartialEq, crate::i18n::Fluent, strum::EnumString, strum::EnumIter)]
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

/// The logo in black on a light bar, in white on a dark one.
fn ink(dark: bool) -> Rgb {
    if dark { WHITE } else { BLACK }
}

/// The dot's color, as the web UI shows it, unless all is well.
fn dot(level: Level) -> Option<Rgb> {
    match level {
        Level::Success => None,
        Level::Warning => Some(WARNING),
        Level::Failure => Some(FAILURE),
    }
}

/// The logo's RGBA pixels in `ink`, cut around the level's dot in its top right corner, which is drawn unless `cut_only`,
/// as macOS draws it apart.
fn pixels(level: Level, ink: Rgb, cut_only: bool) -> Vec<u8> {
    let ([red, green, blue], color) = (ink, dot(level));
    // Top right, as the badges of the Dock and of other menu bar icons.
    #[expect(clippy::cast_precision_loss, reason = "a few pixels")]
    let (cx, cy) = (SIDE as f32 - DOT, DOT);
    LOGO.iter()
        .zip(0..)
        .flat_map(|(&opacity, i)| {
            let Some([dot_red, dot_green, dot_blue]) = color else { return [red, green, blue, opacity] };
            #[expect(clippy::cast_precision_loss, reason = "a few pixels")]
            let (x, y) = ((i % SIDE) as f32 + HALF_PIXEL, (i / SIDE) as f32 + HALF_PIXEL);
            let distance = (x - cx).hypot(y - cy);
            // Antialiased: an edge pixel is as opaque as the shape covers it.
            let cover = |radius: f32| (radius + HALF_PIXEL - distance).clamp(0.0, 1.0);
            let (dot, logo) = (if cut_only { 0.0 } else { cover(DOT) }, 1.0 - cover(DOT + GAP));
            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "within 0..=255")]
            let alpha = |a: f32| (a * f32::from(u8::MAX)).round() as u8;
            if dot > 0.0 {
                [dot_red, dot_green, dot_blue, alpha(dot)]
            } else {
                [red, green, blue, alpha(f32::from(opacity) / f32::from(u8::MAX) * logo)]
            }
        })
        .collect()
}
