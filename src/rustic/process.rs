//! Rustic's process, and the processes it starts.

/// Bytes read and written so far, from `/proc`. Not CPU time: rustic spins while it waits for a
/// stdin command.
#[cfg(target_os = "linux")]
#[expect(clippy::disallowed_methods, reason = "reads /proc")]
pub(super) fn work(pid: u32) -> Option<u64> {
    let io = std::fs::read_to_string(format!("/proc/{pid}/io")).ok()?;
    let io = io.lines().filter(|l| l.starts_with("rchar") || l.starts_with("wchar"));
    Some(io.filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).sum())
}

/// Unknown on other systems: only output shows activity.
#[cfg(not(target_os = "linux"))]
pub(super) fn work(_: u32) -> Option<u64> {
    None
}

/// Ends `process`, and what it starts, with this process. Windows releases locks with the process that took
/// them, so rustic cannot inherit the run's locks as on Unix: a killed run leaves no rustic running without them.
#[cfg(windows)]
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
#[expect(unsafe_code, reason = "signal or terminate a process by its id")]
pub(crate) fn kill(pid: u32) {
    #[cfg(unix)]
    // SAFETY: a plain syscall.
    unsafe {
        libc::kill(-pid.cast_signed(), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess},
        };
        // SAFETY: a handle opened, used and closed here.
        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if !handle.is_null() {
                TerminateProcess(handle, 1);
                CloseHandle(handle);
            }
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    #[expect(clippy::disallowed_methods, reason = "reads a file to produce I/O")]
    fn work_is_counted() {
        let before = work(std::process::id()).unwrap();
        _ = std::fs::read("/proc/self/stat");
        assert!(work(std::process::id()).unwrap() > before);
    }
}
