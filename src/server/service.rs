//! Running as a service: signals, systemd, and the Windows service manager.

use std::future::Future;

use super::Logged;
use crate::log;

/// Asks the server to stop, as SIGTERM does: the tray's Quit, and the Windows service manager.
#[cfg(feature = "desktop")]
pub(super) static STOP: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// SIGINT or SIGTERM, Ctrl-C or Ctrl-Break on Windows, or [`STOP`].
pub(super) async fn interrupted() {
    let signal = async {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            match signal(SignalKind::terminate()) {
                Ok(mut term) => tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                },
                Err(_) => _ = tokio::signal::ctrl_c().await,
            }
        }
        // Ctrl-Break too, which unlike Ctrl-C can be sent to one process group.
        #[cfg(windows)]
        match tokio::signal::windows::ctrl_break() {
            Ok(mut ctrl_break) => tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = ctrl_break.recv() => {}
            },
            Err(_) => _ = tokio::signal::ctrl_c().await,
        }
        #[cfg(not(any(unix, windows)))]
        {
            _ = tokio::signal::ctrl_c().await;
        }
    };
    #[cfg(feature = "desktop")]
    tokio::select! {
        () = signal => {}
        () = STOP.notified() => {}
    }
    #[cfg(not(feature = "desktop"))]
    signal.await;
}

/// SIGHUP (`systemctl reload`), caught from creation. Systemd rejects a `Type=notify-reload` service
/// that reports ready before it handles SIGHUP. Never fires on other systems.
pub(super) struct Hangups(#[cfg(unix)] Option<tokio::signal::unix::Signal>);

impl Hangups {
    pub(super) fn catch() -> Self {
        Self(
            #[cfg(unix)]
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()).ok(),
        )
    }

    pub(super) async fn next(&mut self) {
        #[cfg(unix)]
        if let Some(hup) = &mut self.0 {
            hup.recv().await;
            return;
        }
        std::future::pending::<()>().await;
    }
}

#[derive(Clone, Copy)]
pub(super) enum Phase {
    Ready,
    Reloading,
    Stopping,
}

/// Tells systemd (`Type=notify-reload`) or the Windows service manager, whichever started the server.
pub(super) fn report(phase: Phase) {
    #[cfg(unix)]
    {
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
    #[cfg(all(windows, feature = "desktop"))]
    windows::report(phase);
    #[cfg(all(not(unix), not(all(windows, feature = "desktop"))))]
    {
        _ = phase;
    }
}

/// The system service that `install` installs from an elevated shell.
#[cfg(all(windows, feature = "desktop"))]
pub(super) mod windows {
    use std::{ffi::OsString, sync::OnceLock, time::Duration};

    use anyhow::Result;
    use parking_lot::Mutex;
    use windows_service::{
        service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType},
        service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
        service_dispatcher,
    };

    use super::Phase;
    use crate::NAME;

    /// How long reading the collections may take before the server is ready.
    const START_HINT: Duration = Duration::from_mins(1);
    /// How long runs may take to stop: [`super::super::DRAIN_TIMEOUT`], and a margin.
    const STOP_HINT: Duration = super::super::DRAIN_TIMEOUT.saturating_add(Duration::from_secs(5));

    type Serve = Box<dyn FnOnce() -> Result<()> + Send>;

    /// The server, handed to the service's thread, and what it returned.
    static SERVE: Mutex<Option<Serve>> = Mutex::new(None);
    static SERVED: Mutex<Option<Result<()>>> = Mutex::new(None);
    static STATUS: OnceLock<ServiceStatusHandle> = OnceLock::new();

    /// Whether the service manager started this process.
    pub(in crate::server) fn running() -> bool {
        STATUS.get().is_some()
    }

    /// Runs `serve` as the service if the service manager started this process, else as it is. Asking
    /// fails at once in a process that the service manager did not start.
    pub(in crate::server) fn run(serve: Serve) -> Result<()> {
        *SERVE.lock() = Some(serve);
        if service_dispatcher::start(NAME, ffi_main).is_err() {
            return SERVE.lock().take().map_or(Ok(()), |serve| serve());
        }
        SERVED.lock().take().unwrap_or(Ok(()))
    }

    windows_service::define_windows_service!(ffi_main, main);

    fn main(_: Vec<OsString>) {
        #[expect(clippy::wildcard_enum_match_arm, reason = "the other controls are not accepted")]
        let handler = |control| match control {
            // Shutting down, the system waits for a service that accepts this before it stops the others.
            ServiceControl::Stop | ServiceControl::Preshutdown => {
                super::STOP.notify_one();
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let Ok(status) = service_control_handler::register(NAME, handler) else { return };
        _ = STATUS.set(status);
        set(ServiceState::StartPending, START_HINT, ServiceExitCode::default());
        let served = SERVE.lock().take().map_or(Ok(()), |serve| serve());
        if let Err(e) = &served {
            log_error(&format!("{e:#}"));
        }
        // Not zero after a failure, so that the service manager restarts it.
        let code = if served.is_ok() { ServiceExitCode::default() } else { ServiceExitCode::ServiceSpecific(1) };
        *SERVED.lock() = Some(served);
        set(ServiceState::Stopped, Duration::ZERO, code);
    }

    /// Stops the server when `install` sets [`stop_event`].
    #[expect(unsafe_code, reason = "creates a named event and waits on it")]
    pub(in crate::server) fn stop_on_request() {
        use windows_sys::Win32::{
            Foundation::WAIT_OBJECT_0,
            System::Threading::{CreateEventW, INFINITE, WaitForSingleObject},
        };
        // SAFETY: a NUL-terminated name; the event resets itself, and is never closed.
        let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, crate::install::autostart::stop_event().as_ptr()) };
        if event.is_null() {
            return;
        }
        // A handle is a pointer, which threads do not share, but the event lives as long as the process.
        let event = event as usize;
        std::thread::spawn(move || {
            // SAFETY: the event created above, never closed.
            if unsafe { WaitForSingleObject(event as _, INFINITE) } == WAIT_OBJECT_0 {
                super::STOP.notify_one();
            }
        });
    }

    /// Writes `error` to the Application event log, as the service has no terminal to print it in.
    #[expect(unsafe_code, reason = "reports a NUL-terminated string to the event log")]
    fn log_error(error: &str) {
        use windows_sys::Win32::System::EventLog::{
            DeregisterEventSource, EVENTLOG_ERROR_TYPE, RegisterEventSourceW, ReportEventW,
        };
        let (source, text): (Vec<u16>, Vec<u16>) =
            (NAME.encode_utf16().chain([0]).collect(), error.encode_utf16().chain([0]).collect());
        // SAFETY: NUL-terminated strings, one of them passed, and the source closed after.
        unsafe {
            let log = RegisterEventSourceW(std::ptr::null(), source.as_ptr());
            if log.is_null() {
                return;
            }
            let strings = [text.as_ptr()];
            ReportEventW(
                log,
                EVENTLOG_ERROR_TYPE,
                0,
                0,
                std::ptr::null_mut(),
                1,
                0,
                strings.as_ptr(),
                std::ptr::null(),
            );
            DeregisterEventSource(log);
        }
    }

    pub(super) fn report(phase: Phase) {
        match phase {
            Phase::Ready => set(ServiceState::Running, Duration::ZERO, ServiceExitCode::default()),
            Phase::Stopping => set(ServiceState::StopPending, STOP_HINT, ServiceExitCode::default()),
            Phase::Reloading => {}
        }
    }

    fn set(state: ServiceState, wait_hint: Duration, exit_code: ServiceExitCode) {
        let Some(status) = STATUS.get() else { return };
        let controls_accepted = if state == ServiceState::Running {
            ServiceControlAccept::STOP | ServiceControlAccept::PRESHUTDOWN
        } else {
            ServiceControlAccept::empty()
        };
        // Each pending state is reported once: the first checkpoint.
        let checkpoint = u32::from(wait_hint > Duration::ZERO);
        _ = status.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted,
            exit_code,
            checkpoint,
            wait_hint,
            process_id: None,
        });
    }
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
