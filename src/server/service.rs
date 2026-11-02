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
