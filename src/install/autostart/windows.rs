use std::time::Duration;

use windows_service::{
    service::{
        Service, ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl, ServiceFailureActions,
        ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState, ServiceType,
    },
    service_manager::{ServiceManager, ServiceManagerAccess},
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SUCCESS},
    System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_MULTI_SZ, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW,
    },
};

use super::{Path, PathBuf, Removed, Result};
use crate::{APP_NAME, NAME, settings, sys, sys::wide};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const SERVICES: &str = r"SYSTEM\CurrentControlSet\Services";
/// After the third failure within the reset period, restarts wait the longest.
const RESTARTS: [Duration; 3] = [Duration::from_secs(3), Duration::from_secs(30), Duration::from_mins(10)];
const RESET: Duration = Duration::from_mins(15);
/// How often a stopping service is asked whether it stopped, and for how long.
const POLL: Duration = Duration::from_millis(200);
const STOP_TIMEOUT: Duration = Duration::from_secs(30);
/// Windows 11 24H2, the first to read the console policy of a manifest.
const CONSOLE_POLICY_BUILD: u32 = 26100;
/// Longer than any path of an executable that Windows runs.
const MAX_PATH: usize = 1024;

fn os_error(status: u32) -> std::io::Error {
    std::io::Error::from_raw_os_error(status.cast_signed())
}

/// Sets `value` of `key`, as `kind`, from its UTF-16 data.
#[expect(unsafe_code, reason = "writes a registry value from NUL-terminated strings")]
fn set(root: HKEY, key: &str, value: &str, kind: u32, data: &[u16]) -> Result<()> {
    let size = u32::try_from(size_of_val(data))?;
    // SAFETY: NUL-terminated strings, and the data's size in bytes.
    let status =
        unsafe { RegSetKeyValueW(root, wide(key).as_ptr(), wide(value).as_ptr(), kind, data.as_ptr().cast(), size) };
    if status != ERROR_SUCCESS {
        return Err(os_error(status).into());
    }
    Ok(())
}

fn manager(access: ServiceManagerAccess) -> Result<ServiceManager> {
    Ok(ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | access)?)
}

fn not_installed(error: &windows_service::Error) -> bool {
    matches!(error, windows_service::Error::Winapi(e)
        if e.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST.cast_signed()))
}

/// Asks the service to stop, and waits until it has: runs get [`crate::server`]'s time to record that.
#[expect(clippy::disallowed_methods, reason = "writes the service files and runs the service manager")]
fn stop(service: &Service) -> Result<()> {
    if service.query_status()?.current_state == ServiceState::Stopped {
        return Ok(());
    }
    service.stop()?;
    let deadline = std::time::Instant::now() + STOP_TIMEOUT;
    while service.query_status()?.current_state != ServiceState::Stopped && std::time::Instant::now() < deadline {
        std::thread::sleep(POLL);
    }
    Ok(())
}

#[expect(clippy::disallowed_methods, reason = "writes the service files and runs the service manager")]
fn install_service(exe: &Path, serve: &[String]) -> Result<PathBuf> {
    let info = ServiceInfo {
        name: NAME.into(),
        display_name: APP_NAME.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe.to_owned(),
        launch_arguments: serve.iter().map(Into::into).collect(),
        dependencies: Vec::new(),
        // LocalSystem.
        account_name: None,
        account_password: None,
    };
    let access =
        ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::CHANGE_CONFIG;
    let manager = manager(ServiceManagerAccess::CREATE_SERVICE)?;
    // Installed before: updated.
    let service = manager.create_service(&info, access).or_else(|_| {
        let service = manager.open_service(NAME, access)?;
        service.change_config(&info)?;
        Ok::<_, windows_service::Error>(service)
    })?;
    let actions = RESTARTS.map(|delay| ServiceAction { action_type: ServiceActionType::Restart, delay });
    service.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(RESET),
        reboot_msg: None,
        command: None,
        actions: Some(actions.into()),
    })?;
    // Also after an exit with an error, not only after a crash.
    service.set_failure_actions_on_non_crash_failures(true)?;
    // This `PATH` and rustic, so that it runs the same rustic, even one downloaded into this user's
    // directory rather than the system account's.
    let rustic = crate::rustic::bin::path();
    let environment = std::iter::once(format!("PATH={}", std::env::var("PATH").unwrap_or_default()))
        .chain(rustic.is_absolute().then(|| format!("{}={}", settings::RUSTIC.key, rustic.display())));
    // A NUL-terminated string each, then one more NUL.
    let data: Vec<u16> = environment.flat_map(wide).chain([0]).collect();
    let key = format!(r"{SERVICES}\{NAME}");
    set(HKEY_LOCAL_MACHINE, &key, "Environment", REG_MULTI_SZ, &data)?;
    // Restarted, so that a running server takes the new executable.
    stop(&service)?;
    service.start::<&str>(&[])?;
    Ok(PathBuf::from(format!(r"HKEY_LOCAL_MACHINE\{key}")))
}

/// Stops the running server, so that `exe` can be replaced: the service, or the server in this session.
/// Then waits, at most [`STOP_TIMEOUT`], until every process that runs `exe` has exited, which is after
/// the server answers no more and its threads end.
pub(super) fn stop_running(exe: &Path) -> Result<()> {
    if sys::is_root() {
        let access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP;
        match manager(ServiceManagerAccess::empty())?.open_service(NAME, access) {
            Err(e) if not_installed(&e) => {}
            service => stop(&service?)?,
        }
    } else {
        ask_to_stop();
    }
    wait_for(exe);
    Ok(())
}

/// Asks the server in this session, if any, to stop.
#[expect(unsafe_code, reason = "opens a named event, sets it and closes it")]
fn ask_to_stop() {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{EVENT_MODIFY_STATE, OpenEventW, SetEvent},
    };
    // SAFETY: a NUL-terminated name; the handle is closed once set.
    unsafe {
        let event = OpenEventW(EVENT_MODIFY_STATE, 0, stop_event().as_ptr());
        if !event.is_null() {
            SetEvent(event);
            CloseHandle(event);
        }
    }
}

/// The event that asks the server in this session to stop, as the service manager asks the service, so
/// that `install` can replace its executable.
pub(crate) fn stop_event() -> Vec<u16> {
    wide(format!(r"Local\{NAME}-stop"))
}

/// Waits until the processes that run `exe` have exited, at most [`STOP_TIMEOUT`].
#[expect(unsafe_code, reason = "lists the processes, and waits on those that run `exe`")]
fn wait_for(exe: &Path) {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
            },
            Threading::{
                OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
                QueryFullProcessImageNameW, WaitForMultipleObjects,
            },
        },
    };
    let mut running = Vec::new();
    // SAFETY: a snapshot read entry by entry, each into a struct of the size it states; each process opened
    // is queried into a buffer of the size given, and closed unless kept to wait on.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = u32::try_from(size_of::<PROCESSENTRY32W>()).unwrap_or_default();
        let mut more = Process32FirstW(snapshot, &raw mut entry) != 0;
        while more {
            let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
            let process = OpenProcess(access, 0, entry.th32ProcessID);
            if !process.is_null() {
                let mut name = [0u16; MAX_PATH];
                let mut size = u32::try_from(name.len()).unwrap_or_default();
                let named = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, name.as_mut_ptr(), &raw mut size);
                let image = String::from_utf16_lossy(name.get(..size as usize).unwrap_or_default());
                if named != 0
                    && image.eq_ignore_ascii_case(&exe.to_string_lossy())
                    && entry.th32ProcessID != std::process::id()
                {
                    running.push(process);
                } else {
                    CloseHandle(process);
                }
            }
            more = Process32NextW(snapshot, &raw mut entry) != 0;
        }
        CloseHandle(snapshot);
        if !running.is_empty() {
            let count = u32::try_from(running.len()).unwrap_or_default();
            let timeout = u32::try_from(STOP_TIMEOUT.as_millis()).unwrap_or(u32::MAX);
            WaitForMultipleObjects(count, running.as_ptr(), 1, timeout);
        }
        for process in running {
            CloseHandle(process);
        }
    }
}

fn uninstall_service() -> Result<Option<Removed>> {
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE;
    let service = match manager(ServiceManagerAccess::empty())?.open_service(NAME, access) {
        Err(e) if not_installed(&e) => return Ok(None),
        service => service?,
    };
    stop(&service)?;
    service.delete()?;
    Ok(Some(Removed { file: format!(r"HKEY_LOCAL_MACHINE\{SERVICES}\{NAME}"), running: false }))
}

fn run_key() -> String {
    format!(r"HKEY_CURRENT_USER\{RUN}\{NAME}")
}

/// The Start menu entry. Unlike a desktop entry or the macOS app, it has no keywords: Start menu search
/// matches only its name.
fn start_menu() -> Result<PathBuf> {
    use etcetera::BaseStrategy;
    let programs = etcetera::choose_base_strategy()?.config_dir().join(r"Microsoft\Windows\Start Menu\Programs");
    Ok(programs.join(format!("{APP_NAME}.lnk")))
}

/// What runs `exe` with `args` without a console window, so that none opens at login or from the Start
/// menu: `exe` itself from Windows 11 24H2 on, which reads the console policy of its manifest, else Windows's
/// console host, which gives it a console that has no window.
#[expect(clippy::disallowed_methods, reason = "writes the service files and runs the service manager")]
fn without_window(exe: &str, args: &str) -> (PathBuf, String) {
    if sys::windows_build() >= CONSOLE_POLICY_BUILD {
        return (PathBuf::from(exe), args.into());
    }
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    (PathBuf::from(root).join(r"System32\conhost.exe"), format!("--headless \"{exe}\" {args}"))
}

#[expect(clippy::disallowed_methods, reason = "writes the service files and runs the service manager")]
pub(super) fn install(listen: &str, exe: &Path, serve: &[String]) -> Result<PathBuf> {
    use std::os::windows::process::CommandExt;
    if sys::is_root() {
        return install_service(exe, serve);
    }
    let exe = exe.display().to_string();
    set_run(&exe, serve)?;
    let (host, args) = without_window(&exe, "serve --open");
    let mut link = mslnk::ShellLink::new(host)?;
    link.set_arguments(Some(args));
    link.set_icon_location(Some(exe.clone()));
    link.set_name(Some(crate::i18n::t!("app-description")));
    link.create_lnk(start_menu()?)?;
    // Started now without a console, as at login, unless it runs already. Without this one's output
    // either, which would otherwise keep printing into the terminal after install returns.
    if crate::health::check(listen).is_err() {
        use std::process::Stdio;
        keep_own_handles();
        let flags = windows_sys::Win32::System::Threading::DETACHED_PROCESS;
        let mut server = std::process::Command::new(exe);
        server.args(serve).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        server.creation_flags(flags).spawn()?;
    }
    Ok(PathBuf::from(run_key()))
}

/// The `Run` key's value that starts `exe` with `serve` at login.
fn set_run(exe: &str, serve: &[String]) -> Result<()> {
    let (host, args) = without_window(exe, &serve.join(" "));
    set(HKEY_CURRENT_USER, RUN, NAME, REG_SZ, &wide(format!("\"{}\" {args}", host.display())))
}

/// Installed with its Start menu entry; started at login by the `Run` key.
#[expect(unsafe_code, reason = "reads a registry value named by NUL-terminated strings")]
#[expect(clippy::disallowed_methods, reason = "writes the service files and runs the service manager")]
pub(super) fn at_login() -> Option<bool> {
    use windows_sys::Win32::System::Registry::{RRF_RT_REG_SZ, RegGetValueW};
    start_menu().ok().filter(|link| link.exists())?;
    // SAFETY: NUL-terminated strings, and no buffer: it only asks whether the value exists.
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(RUN).as_ptr(),
            wide(NAME).as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    Some(read == ERROR_SUCCESS)
}

#[expect(unsafe_code, reason = "deletes a registry value named by NUL-terminated strings")]
pub(super) fn set_at_login(on: bool) -> Result<()> {
    if on {
        return set_run(&exe()?, &super::serve_args());
    }
    // SAFETY: NUL-terminated strings.
    match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(RUN).as_ptr(), wide(NAME).as_ptr()) } {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
        status => Err(os_error(status).into()),
    }
}

/// Makes this process's standard input, output and error not inheritable. Rust lets a child inherit every
/// inheritable handle, and the server would hold the pipe of a script that reads install's output, which
/// then waits until the server stops.
#[expect(unsafe_code, reason = "changes a flag of this process's own standard handles")]
fn keep_own_handles() {
    use windows_sys::Win32::{
        Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation},
        System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
    };
    for handle in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: this process's own handle; one that is missing is refused.
        unsafe { SetHandleInformation(GetStdHandle(handle), HANDLE_FLAG_INHERIT, 0) };
    }
}

/// `None` if not installed. The server started at login has no service to stop it: it runs until logout.
#[expect(unsafe_code, reason = "deletes a registry value named by NUL-terminated strings")]
#[expect(clippy::disallowed_methods, reason = "writes the service files and runs the service manager")]
pub(super) fn uninstall() -> Result<Option<Removed>> {
    if sys::is_root() {
        return uninstall_service();
    }
    _ = std::fs::remove_file(start_menu()?);
    // SAFETY: NUL-terminated strings.
    match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(RUN).as_ptr(), wide(NAME).as_ptr()) } {
        ERROR_SUCCESS => Ok(Some(Removed { file: run_key(), running: true })),
        ERROR_FILE_NOT_FOUND => Ok(None),
        status => Err(os_error(status).into()),
    }
}

fn exe() -> Result<String> {
    Ok(std::env::current_exe()?.to_string_lossy().into_owned())
}

/// macOS's alone.
#[cfg(feature = "self-update")]
#[expect(clippy::unnecessary_wraps, reason = "macOS's copies the executable, which may fail")]
pub(super) fn refresh_app() -> Result<Option<String>> {
    Ok(None)
}

/// macOS's alone.
#[cfg(feature = "self-update")]
#[expect(clippy::unnecessary_wraps, reason = "macOS's rewrites the app, which may fail")]
pub(super) fn rewrite_app(_: &str) -> Result<()> {
    Ok(())
}

/// macOS's alone: Login Items.
pub(super) fn blocked() -> bool {
    false
}

/// macOS's alone: an app opened from Finder.
pub(super) fn opened_as_app() -> bool {
    false
}

/// Started by hand or from the `Run` key: nothing supervises it to start.
#[expect(clippy::unnecessary_wraps, reason = "Linux's and macOS's ask their service manager, which may fail")]
pub(super) fn start() -> Result<bool> {
    Ok(false)
}

/// Linux's alone: a user without a desktop.
pub(super) fn headless() -> bool {
    false
}
