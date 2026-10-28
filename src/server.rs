//! The server: the scheduler, a thread per run, and the web UI.

mod listing;
mod notice;
mod service;
mod timer;
#[cfg(tray)]
mod tray;
mod watcher;
mod web;
mod workers;

use listing::Listings;
use notice::Notice;
use service::{Hangups, Phase, interrupted, report, serve_web};
use timer::wall_clock;
#[cfg(tray)]
use tray::Tray;
use watcher::{Reload, watch_files};
use web::{Browsed, Tokens, login_tokens};
use workers::Workers;

use std::{collections::BTreeMap, net::SocketAddr, sync::Arc, time::Duration};

use anyhow::Result;
use futures_util::FutureExt;
use parking_lot::RwLock;
use serde::Serialize;
use tokio::sync::{Notify, broadcast, watch};

use crate::{
    engine::Engine,
    live::Runs,
    log::{self, Level, Message},
    request::Trigger,
    settings,
};

/// Run failures kept for a page that has not read them yet.
const FAILURES_QUEUED: usize = 16;
/// How long runs get to stop and record that, at shutdown.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait after a collection change, so that one reload covers an editor's several writes.
const RELOAD_DELAY: Duration = Duration::from_millis(200);
/// Connections the system queues before the server accepts them, as std does.
const LISTEN_BACKLOG: i32 = 128;

/// Reads the collections into an engine, at start and when they change.
type Build = Arc<dyn Fn(Arc<Runs>) -> Result<Engine> + Send + Sync>;

/// What the scheduler and the web UI share.
pub(crate) struct App {
    engine: RwLock<Arc<Engine>>,
    runs: Arc<Runs>,
    workers: Workers,
    pages: Pages,
    /// What is due may have changed: a run ended, or another process changed the state.
    wake: Notify,
    tokens: Tokens,
    listing: Listings,
    /// Set when the server stops, so that event streams end and shutdown does not wait for them.
    stopping: watch::Receiver<bool>,
    browsed: Browsed,
    started: jiff::Timestamp,
}

impl App {
    fn engine(&self) -> Arc<Engine> {
        self.engine.read().clone()
    }
}

/// `error` fields hold the error's text. `key`: the admin token's setting.
#[derive(Debug, Serialize, crate::i18n::Fluent)]
#[fluent(prefix = "server")]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum Logged {
    Requested {
        label: String,
        by: String,
    },
    CancelledFromWeb,
    Serving {
        url: String,
        key: &'static str,
    },
    ServingOpened {
        url: String,
    },
    ServingSavedToken {
        url: String,
        file: String,
    },
    Healthy {
        listen: String,
    },
    /// Not what: a page may carry the login token.
    NotOpened {
        error: String,
    },
    Reloaded {
        changed: Changed,
    },
    NotReloaded {
        error: String,
    },
    NotWatching {
        error: String,
    },
    NotWatchingDir {
        dir: String,
        error: String,
    },
    DirNotCreated {
        dir: String,
        error: String,
    },
    WebUiFailed {
        error: String,
    },
    NoWallClock {
        error: String,
    },
    /// Runs stored in a shape this version cannot read: `3 runs`.
    RunsUnread {
        runs: u64,
    },
}

/// The files whose change asked for a reload: none when a signal did.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub(crate) struct Changed(pub(super) Vec<String>);

impl std::fmt::Display for Changed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_empty() { Ok(()) } else { write!(f, ": {} changed", self.0.join(", ")) }
    }
}

impl Logged {
    /// `file`: the generated token's.
    fn serving(url: String, file: Option<&std::path::Path>) -> Self {
        match file {
            None => Self::Serving { url, key: settings::ADMIN_TOKEN.key },
            Some(file) => Self::ServingSavedToken { url, file: file.display().to_string() },
        }
    }
}

impl Message for Logged {
    fn level(&self) -> Level {
        match self {
            Self::NotReloaded { .. } | Self::WebUiFailed { .. } => Level::Failure,
            Self::Requested { .. }
            | Self::Serving { .. }
            | Self::Healthy { .. }
            | Self::ServingOpened { .. }
            | Self::ServingSavedToken { .. }
            | Self::Reloaded { .. } => Level::Success,
            Self::CancelledFromWeb
            | Self::NotWatching { .. }
            | Self::NotWatchingDir { .. }
            | Self::DirNotCreated { .. }
            | Self::NoWallClock { .. }
            | Self::NotOpened { .. }
            | Self::RunsUnread { .. } => Level::Warning,
        }
    }
}

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
pub(crate) enum Fault {
    #[fluent(key = settings::LISTEN.key)]
    CannotListen {
        listen: String,
        source: std::io::Error,
    },
    /// Ends the Windows service after an update, so that the service manager starts the new executable.
    #[cfg(all(windows, feature = "desktop", feature = "self-update"))]
    Restarting,
    NoSession(#[source] getrandom::Error),
    NoToken(#[source] getrandom::Error),
    SessionNotKept(#[source] Box<dyn std::error::Error + Send + Sync>),
    NoDirectory {
        path: String,
    },
    CannotRunRustic(#[source] std::io::Error),
    NotPiped,
    Stopping,
    DumpFailed,
    NoLongerConfigured,
}

/// Notifies open pages that what they show changed, and why a run they started failed.
struct Pages {
    changes: watch::Sender<Changes>,
    failures: broadcast::Sender<Notice>,
}

impl Pages {
    /// Something every page may show changed.
    fn changed(&self) {
        self.changes.send_modify(|c| c.all = c.next());
    }

    /// Something about `collection` changed.
    fn changed_of(&self, collection: &str) {
        self.changes.send_modify(|c| c.of(collection));
    }

    fn failed(&self, why: Notice) {
        // Fails when no page is open, which is fine.
        let _unheard = self.failures.send(why);
    }
}

/// A count of changes, with the count at the last change of all collections and of each collection.
#[derive(Default)]
struct Changes {
    count: u64,
    all: u64,
    collections: BTreeMap<String, u64>,
}

impl Changes {
    fn next(&mut self) -> u64 {
        self.count += 1;
        self.count
    }

    fn of(&mut self, collection: &str) {
        let n = self.next();
        self.collections.insert(collection.to_owned(), n);
    }

    /// Whether a page about `collection` (all collections if `None`) changed after change number `seen`.
    fn since(&self, seen: u64, collection: Option<&str>) -> bool {
        match collection {
            None => self.count > seen,
            Some(collection) => self.all > seen || self.collections.get(collection).is_some_and(|&n| n > seen),
        }
    }
}

/// `open`: opens the web UI in the browser, logged in. With [`settings::AUTO_UPDATE`], first updates
/// rusticlone and rustic, and starts the new rusticlone in place of this one.
pub(crate) fn serve(
    build: impl Fn(Arc<Runs>) -> Result<Engine> + Send + Sync + 'static,
    listen: &str,
    open: bool,
) -> Result<()> {
    #[cfg(windows)]
    crate::sys::release_own_console();
    let listen = listen.to_owned();
    let serve = move |tray| {
        #[cfg(feature = "self-update")]
        if settings::AUTO_UPDATE.enabled()? {
            // Read before it is replaced: Linux then names the old file, with ` (deleted)`.
            let exe = std::env::current_exe()?;
            if crate::install::update::at_start() {
                return restart(&exe);
            }
        }
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        runtime.block_on(run(Arc::new(build), &listen, open, tray))
    };
    // The system service runs without a desktop, so without a tray.
    #[cfg(all(windows, feature = "desktop"))]
    if crate::sys::is_root() {
        return service::windows::run(Box::new(move || serve(None)));
    }
    #[cfg(all(windows, feature = "desktop"))]
    service::windows::stop_on_request();
    #[cfg(tray)]
    return tray::beside(open, serve);
    #[cfg(not(tray))]
    serve(None)
}

/// Starts the executable, just replaced, in place of this process, with the same arguments: on Unix as the
/// same process, which the service manager keeps following; as the Windows service, by ending with an
/// error, after which the service manager starts it again; else as a new process, and this one ends.
#[cfg(feature = "self-update")]
#[expect(clippy::disallowed_methods, reason = "starts the updated rusticlone, not rustic")]
fn restart(exe: &std::path::Path) -> Result<()> {
    let mut command = std::process::Command::new(exe);
    command.args(std::env::args_os().skip(1));
    #[cfg(unix)]
    return Err(std::os::unix::process::CommandExt::exec(&mut command).into());
    #[cfg(windows)]
    {
        #[cfg(feature = "desktop")]
        if service::windows::running() {
            return Err(Fault::Restarting.into());
        }
        let flags = windows_sys::Win32::System::Threading::DETACHED_PROCESS;
        std::os::windows::process::CommandExt::creation_flags(&mut command, flags).spawn()?;
        Ok(())
    }
}

/// No tray icon on other systems, nor without the desktop.
#[cfg(not(tray))]
struct Tray(std::convert::Infallible);

#[cfg(not(tray))]
impl Tray {
    fn follow(self, _: Arc<App>, _: String, _: String) -> std::future::Ready<()> {
        match self.0 {}
    }
}

fn local_url(addr: SocketAddr) -> String {
    format!("http://{}/", crate::health::loopback(addr))
}

/// Listens on the first address of `listen` that it can. An IPv6 one also accepts IPv4, which Windows
/// does not by default.
fn bind(listen: &str) -> std::io::Result<tokio::net::TcpListener> {
    use socket2::{Domain, Socket, Type};
    let on = |addr: SocketAddr| {
        let socket = Socket::new(Domain::for_address(addr), Type::STREAM, None)?;
        if addr.is_ipv6() {
            socket.set_only_v6(false)?;
        }
        // As std does, so that a restart need not wait for the old connections to time out.
        #[cfg(unix)]
        socket.set_reuse_address(true)?;
        socket.set_nonblocking(true)?;
        socket.bind(&addr.into())?;
        socket.listen(LISTEN_BACKLOG)?;
        tokio::net::TcpListener::from_std(socket.into())
    };
    let mut tried = std::net::ToSocketAddrs::to_socket_addrs(listen)?.map(on);
    let first = tried.next().unwrap_or_else(|| Err(std::io::ErrorKind::AddrNotAvailable.into()));
    first.or_else(|e| tried.find_map(Result::ok).ok_or(e))
}

/// Says where the web UI is, and with `open` opens it logged in, and whether macOS keeps the server from
/// starting at login. The token is in the fragment, which stays in the browser: the login page sends it as
/// the form would.
fn announce(url: String, tokens: &Tokens, open: bool) {
    let opened = open && open_on_desktop(login_url(&url, tokens));
    log::event(
        None,
        &if opened { Logged::ServingOpened { url } } else { Logged::serving(url, tokens.file.as_deref()) },
    );
    #[cfg(feature = "desktop")]
    if let Some(blocked) = crate::install::autostart::blocked() {
        log::event(None, &blocked);
    }
}

fn login_url(url: &str, tokens: &Tokens) -> String {
    format!("{url}login#{}", tokens.admin.reveal())
}

/// Whether the desktop opened `target`: a page in the browser, or a folder in the file manager.
fn open_on_desktop(target: String) -> bool {
    open::that_detached(target).map_err(|e| log::event(None, &Logged::NotOpened { error: e.to_string() })).is_ok()
}

/// With a server already on `listen`, as after `install`, opens it logged in, with the token it keeps
/// in the state directory, and returns true.
pub(crate) fn open_running(listen: &str) -> Result<bool> {
    let Some(addr) = std::net::ToSocketAddrs::to_socket_addrs(listen)?.next() else { return Ok(false) };
    if crate::health::check(listen).is_err() {
        return Ok(false);
    }
    let tokens = login_tokens(&settings::state_dir()?)?;
    open_on_desktop(login_url(&local_url(addr), &tokens));
    Ok(true)
}

async fn run(build: Build, listen: &str, open: bool, tray: Option<Tray>) -> Result<()> {
    let pages =
        Pages { changes: watch::channel(Changes::default()).0, failures: broadcast::channel(FAILURES_QUEUED).0 };
    let runs = Runs::new({
        let changes = pages.changes.clone();
        move |collection| changes.send_modify(|c| c.of(collection))
    });
    let engine = {
        let (build, runs) = (build.clone(), runs.clone());
        tokio::task::spawn_blocking(move || build(runs)).await??
    };
    let store = engine.store.clone();
    let listener = bind(listen).map_err(|source| Fault::CannotListen { listen: listen.to_owned(), source })?;
    let url = local_url(listener.local_addr()?);
    let tokens = login_tokens(&store.dir)?;
    let (stop, stopping) = watch::channel(false);
    let app = Arc::new(App {
        engine: RwLock::new(Arc::new(engine)),
        runs,
        workers: Workers::default(),
        pages,
        wake: Notify::new(),
        tokens,
        listing: Listings::default(),
        stopping: stopping.clone(),
        browsed: Browsed::new()?,
        started: store.now(),
    });
    // Watch before reporting ready, so that a collection saved right after is seen.
    let reload = Arc::new(Reload::default());
    let _watcher = watch_files(&store.dir, &app, &reload);
    if let Some(tray) = tray {
        tokio::spawn(tray.follow(app.clone(), url.clone(), login_url(&url, &app.tokens)));
    }
    announce(url, &app.tokens, open);
    resume(&app)?;

    let web = tokio::spawn(serve_web(listener, web::router(app.clone()), async move {
        let mut stopping = stopping;
        _ = stopping.wait_for(|s| *s).await;
    }));
    let mut hangups = Hangups::catch();
    report(Phase::Ready);

    let mut interrupted = std::pin::pin!(interrupted());
    loop {
        let scheduler = app.clone();
        let next = tokio::task::spawn_blocking(move || {
            let next = scheduler.ask_due();
            // Notifies stale collections too, not only failed runs, and retries failed notifications.
            let engine = scheduler.engine();
            engine.collections().iter().filter_map(|j| engine.notify(j)).chain(next).min()
        })
        .await
        .unwrap_or_default();
        tokio::select! {
            // A collection may have become stale, a failure reported, or a notification due again.
            () = wall_clock(next) => app.pages.changed(),
            () = app.wake.notified() => {}
            () = reload.asked.notified() => reload_profiles(&app, &build, &reload).await,
            () = hangups.next() => reload_profiles(&app, &build, &reload).await,
            () = &mut interrupted => break,
        }
    }

    report(Phase::Stopping);
    crate::clock::interrupt();
    drain(&app).await;
    _ = stop.send(true);
    _ = web.await;
    Ok(())
}

/// Says what the state could not keep, lists the collections it does not know, and queues again the web
/// UI's requests that were queued when the server stopped.
fn resume(app: &Arc<App>) -> Result<()> {
    let engine = app.engine();
    engine.report_broken();
    match engine.store.unreadable_runs() {
        0 => {}
        unread => log::event(None, &Logged::RunsUnread { runs: unread as u64 }),
    }
    list_unknown(app);
    for run in engine.store.orphans()? {
        if let Some(request) = &run.request {
            app.ask(&engine, &run.record.collection, request, Trigger::Web)?;
        }
    }
    Ok(())
}

/// Gives runs a moment to stop their rustic calls and record that. A second signal ends the wait.
async fn drain(app: &App) {
    let mut changes = app.pages.changes.subscribe();
    let drained = async {
        while app.workers.busy() {
            if changes.changed().await.is_err() {
                break;
            }
        }
    };
    tokio::select! {
        () = drained => {}
        () = tokio::time::sleep(DRAIN_TIMEOUT) => {}
        () = interrupted() => {}
    }
}

/// Reads the collections again. Reading runs rustic and the collections' hooks, so this happens only when
/// they change.
async fn reload_profiles(app: &Arc<App>, build: &Build, reload: &Reload) {
    report(Phase::Reloading);
    // An editor saves in several steps: reload once for all of them.
    tokio::time::sleep(RELOAD_DELAY).await;
    reload.asked.notified().now_or_never();
    let changed = reload.take();
    let (build, runs) = (build.clone(), app.runs.clone());
    match tokio::task::spawn_blocking(move || build(runs)).await {
        Ok(Ok(new)) => {
            log::event(None, &Logged::Reloaded { changed });
            new.report_broken();
            *app.engine.write() = Arc::new(new);
            app.pages.changed();
            list_unknown(app);
        }
        Ok(Err(e)) => log::event(None, &Logged::NotReloaded { error: format!("{e:#}") }),
        Err(e) => log::event(None, &Logged::NotReloaded { error: e.to_string() }),
    }
    report(Phase::Ready);
}

/// Lists the collections the state does not know, as `Refresh all` does, so that their pages show it.
fn list_unknown(app: &Arc<App>) {
    for j in app.engine().unknown() {
        app.list_repos(j.name(), j.repos().map(|p| p.name.clone()).collect());
    }
}

#[cfg(test)]
mod tests {
    use super::Changes;

    #[test]
    fn page_updates_only_for_its_own_changes() {
        let mut changes = Changes::default();
        changes.of("Docs");
        let seen = changes.count;
        changes.of("Photos");
        assert!(!changes.since(seen, Some("Docs")) && changes.since(seen, Some("Photos")));
        assert!(changes.since(seen, None), "the overview shows every collection");
        changes.all = changes.next();
        assert!(changes.since(seen, Some("Docs")));
    }
}
