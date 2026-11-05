//! Unix's signals, and systemd's notifications.

use super::Phase;

/// SIGINT or SIGTERM, caught from the call: one that comes before the wait starts still stops the server as asked,
/// rather than killing it.
pub(super) fn signal() -> impl Future<Output = ()> {
    use tokio::signal::unix::{Signal, SignalKind, signal};
    async fn next(signal: Option<&mut Signal>) {
        match signal {
            Some(signal) => _ = signal.recv().await,
            None => std::future::pending().await,
        }
    }
    let (mut term, mut int) = (signal(SignalKind::terminate()).ok(), signal(SignalKind::interrupt()).ok());
    async move {
        tokio::select! {
            () = next(term.as_mut()) => {}
            () = next(int.as_mut()) => {}
        }
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
            _ = sd_notify::notify(&std::iter::once(NotifyState::Reloading).chain(now).collect::<Vec<_>>());
            return;
        }
        Phase::Stopping => NotifyState::Stopping,
    };
    _ = sd_notify::notify(&[state]);
}

/// Windows's alone: systemd and launchd run the server as any process.
#[cfg_attr(test, mutants::skip)] // Windows's alone.
pub(super) fn as_service() -> bool {
    false
}

/// Windows's alone: [`as_service`] is never true here.
#[cfg_attr(test, mutants::skip)] // Windows's alone.
pub(super) fn run_service(serve: Box<dyn FnOnce() -> anyhow::Result<()> + Send>) -> anyhow::Result<()> {
    serve()
}

/// Signals stop the server here: see [`signal`].
pub(super) fn stop_on_request() {}
