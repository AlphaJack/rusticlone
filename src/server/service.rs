//! Running as a service: signals, systemd, and the Windows service manager.

use std::future::Future;

use super::Logged;
use crate::log;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
pub(super) mod windows;
#[cfg(windows)]
use windows as imp;

pub(super) use imp::Hangups;

/// Whether the server runs as the system service: an administrator's, on Windows.
#[cfg_attr(test, mutants::skip)] // Asks the Windows service manager.
pub(super) fn as_service() -> bool {
    imp::as_service()
}

/// Runs `serve` as the system service.
#[cfg_attr(test, mutants::skip)] // Hands this process to the Windows service manager.
pub(super) fn run_service(serve: Box<dyn FnOnce() -> anyhow::Result<()> + Send>) -> anyhow::Result<()> {
    imp::run_service(serve)
}

/// Stops the server when the system asks, as when Windows ends the user's session.
pub(super) fn stop_on_request() {
    imp::stop_on_request();
}

/// Asks the server to stop, as SIGTERM does: the tray's Quit, and the Windows service manager.
#[cfg(feature = "desktop")]
pub(super) static STOP: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// SIGINT or SIGTERM, Ctrl-C or Ctrl-Break on Windows, or [`STOP`].
pub(super) async fn interrupted() {
    #[cfg(feature = "desktop")]
    tokio::select! {
        () = imp::signal() => {}
        () = STOP.notified() => {}
    }
    #[cfg(not(feature = "desktop"))]
    imp::signal().await;
}

#[derive(Clone, Copy)]
pub(super) enum Phase {
    Ready,
    Reloading,
    Stopping,
}

/// Tells systemd (`Type=notify-reload`) or the Windows service manager, whichever started the server.
pub(super) fn report(phase: Phase) {
    imp::report(phase);
}

pub(super) async fn serve_web(
    listener: tokio::net::TcpListener,
    router: axum::Router,
    stop: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    // With each request's peer, which tells a browser on this computer.
    let router = router.into_make_service_with_connect_info::<std::net::SocketAddr>();
    let served = axum::serve(listener, router).with_graceful_shutdown(stop).await;
    if let Err(e) = &served {
        log::event(None, &Logged::WebUiFailed { error: e.to_string() });
    }
    served
}
