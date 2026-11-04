//! Windows's signals, and the system service that `install` installs from an elevated shell.

use super::Phase;

/// Ctrl-C, or Ctrl-Break, which unlike Ctrl-C can be sent to one process group.
pub(super) fn signal() -> impl Future<Output = ()> {
    let (mut ctrl_c, mut ctrl_break) =
        (tokio::signal::windows::ctrl_c().ok(), tokio::signal::windows::ctrl_break().ok());
    async move {
        tokio::select! {
            () = async { if let Some(c) = ctrl_c.as_mut() { c.recv().await; } else { std::future::pending::<()>().await } } => {}
            () = async { if let Some(b) = ctrl_break.as_mut() { b.recv().await; } else { std::future::pending::<()>().await } } => {}
        }
    }
}

/// Unix's alone: Windows has no SIGHUP, so this never fires.
pub(in crate::server) struct Hangups;

impl Hangups {
    pub(in crate::server) fn catch() -> Self {
        Self
    }

    pub(in crate::server) async fn next(&mut self) {
        std::future::pending::<()>().await;
    }
}

/// Tells the service manager, if it started the server.
pub(super) fn report(phase: Phase) {
    #[cfg(feature = "desktop")]
    manager::report(phase);
    #[cfg(not(feature = "desktop"))]
    let _ = phase;
}

/// Whether the service manager started this process.
#[cfg(feature = "self-update")]
pub(in crate::server) fn running() -> bool {
    #[cfg(feature = "desktop")]
    return manager::running();
    #[cfg(not(feature = "desktop"))]
    false
}

/// An administrator's server is the system service, which only the desktop build installs.
pub(super) fn as_service() -> bool {
    cfg!(feature = "desktop") && crate::sys::is_root()
}

/// Runs `serve` as the service if the service manager started this process, else as it is.
pub(super) fn run_service(serve: Box<dyn FnOnce() -> anyhow::Result<()> + Send>) -> anyhow::Result<()> {
    #[cfg(feature = "desktop")]
    return manager::run(serve);
    #[cfg(not(feature = "desktop"))]
    serve()
}

/// Stops the server when Windows ends the user's session.
pub(super) fn stop_on_request() {
    #[cfg(feature = "desktop")]
    manager::stop_on_request();
}

/// The service manager's side, which only the desktop build installs.
#[cfg(feature = "desktop")]
mod manager {
    use std::{ffi::OsString, sync::OnceLock, time::Duration};

    use anyhow::Result;
    use parking_lot::Mutex;
    use windows_service::{
        service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType},
        service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
        service_dispatcher,
    };

    use crate::NAME;
    use crate::server::service::Phase;

    /// How long reading the collections may take before the server is ready.
    const START_HINT: Duration = Duration::from_mins(1);
    /// How long runs may take to stop: [`crate::server::DRAIN_TIMEOUT`], and a margin.
    const STOP_HINT: Duration = crate::server::DRAIN_TIMEOUT.saturating_add(Duration::from_secs(5));

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
                crate::server::service::STOP.notify_one();
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
        let event =
            unsafe { CreateEventW(std::ptr::null(), 0, 0, crate::install::autostart::windows::stop_event().as_ptr()) };
        if event.is_null() {
            return;
        }
        // A handle is a pointer, which threads do not share, but the event lives as long as the process.
        let event = event as usize;
        std::thread::spawn(move || {
            // SAFETY: the event created above, never closed.
            if unsafe { WaitForSingleObject(event as _, INFINITE) } == WAIT_OBJECT_0 {
                crate::server::service::STOP.notify_one();
            }
        });
    }

    /// Writes `error` to the Application event log, as the service has no terminal to print it in.
    #[expect(unsafe_code, reason = "reports a NUL-terminated string to the event log")]
    fn log_error(error: &str) {
        use windows_sys::Win32::System::EventLog::{
            DeregisterEventSource, EVENTLOG_ERROR_TYPE, RegisterEventSourceW, ReportEventW,
        };
        let (source, text) = (crate::sys::wide(NAME), crate::sys::wide(error));
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
