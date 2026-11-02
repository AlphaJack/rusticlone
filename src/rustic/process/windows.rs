//! Windows's processes: their I/O counters, a job object that ends them with this process, and ending one.

/// Bytes read and written so far, by every kind of I/O: files, pipes and network.
#[expect(unsafe_code, reason = "reads the I/O counters of a process")]
pub(in crate::rustic) fn work(pid: u32) -> Option<u64> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{GetProcessIoCounters, IO_COUNTERS, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    let mut counters = IO_COUNTERS::default();
    // SAFETY: a handle opened, used and closed here, and a record of the type that the call fills.
    let read = unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let read = GetProcessIoCounters(handle, &raw mut counters);
        CloseHandle(handle);
        read != 0
    };
    read.then(|| counters.ReadTransferCount + counters.WriteTransferCount + counters.OtherTransferCount)
}

/// Ends `process`, and what it starts, with this process. Windows releases locks with the process that took
/// them, so rustic cannot inherit the run's locks as on Unix: a killed run leaves no rustic running without them.
#[expect(unsafe_code, reason = "puts a process in a job object that ends with this process")]
pub(crate) fn tie(process: std::os::windows::io::RawHandle) {
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    static JOB: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let job = *JOB.get_or_init(|| {
        // SAFETY: a new unnamed job, never closed here: it closes as this process ends, ending what is in it.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).unwrap_or_default();
            SetInformationJobObject(job, JobObjectExtendedLimitInformation, (&raw const limits).cast(), size);
            job as usize
        }
    });
    if job != 0 {
        // SAFETY: the job above, and a process that this one started.
        unsafe { AssignProcessToJobObject(job as _, process.cast()) };
    }
}

/// Kills rustic. On Unix, also the processes it started, which share its process group.
#[expect(unsafe_code, reason = "terminates a process by its id")]
pub(crate) fn kill(pid: u32) {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess},
    };
    // SAFETY: a handle opened here, closed once.
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !handle.is_null() {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
}

/// Ends `child`, and what it starts, with this process, as Windows needs: see `tie`.
#[cfg(feature = "server")]
pub(crate) fn tie_child(child: &tokio::process::Child) {
    if let Some(handle) = child.raw_handle() {
        tie(handle);
    }
}

/// Without a console window, since the server started at login has none and Windows would open one for each
/// run, and below normal priority, as systemd's `Nice` and launchd's `Background` run it elsewhere, so that a
/// backup does not slow down the user.
pub(crate) fn quiet(cmd: &mut std::process::Command) {
    use windows_sys::Win32::System::Threading::{BELOW_NORMAL_PRIORITY_CLASS, CREATE_NO_WINDOW};
    std::os::windows::process::CommandExt::creation_flags(cmd, CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
}

/// Unix's alone: [`tie_spawned`] ends rustic with this process instead.
pub(crate) fn own_group(_: &mut std::process::Command) {}

/// Windows releases locks with the process that took them, so rustic cannot inherit them: [`tie_spawned`] ends it
/// with this process instead.
pub(crate) fn inherit_locks(_: &mut std::process::Command, _: &[i32]) {}

/// Ends `child`, and what it starts, with this process.
pub(crate) fn tie_spawned(child: &std::process::Child) {
    tie(std::os::windows::io::AsRawHandle::as_raw_handle(child));
}
