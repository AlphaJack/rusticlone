//! Unix's signals, and systemd's notifications.

use super::Phase;

/// SIGINT or SIGTERM.
pub(super) async fn signal() {
    use tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut term) => tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        },
        Err(_) => _ = tokio::signal::ctrl_c().await,
    }
}

/// SIGHUP (`systemctl reload`), caught from creation. Systemd rejects a `Type=notify-reload` service
/// that reports ready before it handles SIGHUP.
pub(in crate::server) struct Hangups(Option<tokio::signal::unix::Signal>);

impl Hangups {
    pub(in crate::server) fn catch() -> Self {
        Self(tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()).ok())
    }

    pub(in crate::server) async fn next(&mut self) {
        if let Some(hup) = &mut self.0 {
            hup.recv().await;
            return;
        }
        std::future::pending::<()>().await;
    }
}

/// Tells systemd, if it started the server.
pub(super) fn report(phase: Phase) {
    use sd_notify::NotifyState;
    let state = match phase {
        Phase::Ready => NotifyState::Ready,
        // With a timestamp that systemd compares with its own.
        Phase::Reloading => {
            let now = NotifyState::monotonic_usec_now().ok();
            _ = sd_notify::notify(&[NotifyState::Reloading].into_iter().chain(now).collect::<Vec<_>>());
            return;
        }
        Phase::Stopping => NotifyState::Stopping,
    };
    _ = sd_notify::notify(&[state]);
}

/// Windows's alone.
#[cfg(feature = "self-update")]
pub(super) fn running() -> bool {
    false
}

/// Windows's alone: systemd and launchd run the server as any process.
pub(super) fn as_service() -> bool {
    false
}

/// Windows's alone: [`as_service`] is never true here.
pub(super) fn run_service(serve: Box<dyn FnOnce() -> anyhow::Result<()> + Send>) -> anyhow::Result<()> {
    serve()
}

/// Signals stop the server here: see [`signal`].
pub(super) fn stop_on_request() {}
