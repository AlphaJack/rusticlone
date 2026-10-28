//! Rustic's process, and the processes it starts.

/// Bytes read and written so far, from `/proc`, network included. Not CPU time: rustic spins while it waits for a
/// stdin command.
#[cfg(target_os = "linux")]
#[expect(clippy::disallowed_methods, reason = "reads /proc")]
pub(super) fn work(pid: u32) -> Option<u64> {
    let io = std::fs::read_to_string(format!("/proc/{pid}/io")).ok()?;
    let io = io.lines().filter(|l| l.starts_with("rchar") || l.starts_with("wchar"));
    Some(io.filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).sum())
}

/// Bytes read from and written to disk so far. Unlike Linux, macOS does not count a remote's traffic.
#[cfg(target_vendor = "apple")]
#[expect(unsafe_code, reason = "fills a resource usage record of a process")]
pub(super) fn work(pid: u32) -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: a record of the size that the flavor asks for.
    let filled =
        unsafe { libc::proc_pid_rusage(pid.cast_signed(), libc::RUSAGE_INFO_V2, usage.as_mut_ptr().cast()) } == 0;
    // SAFETY: filled by the call, which returned 0.
    let usage = filled.then(|| unsafe { usage.assume_init() })?;
    Some(usage.ri_diskio_bytesread + usage.ri_diskio_byteswritten)
}

/// Bytes read and written so far, by every kind of I/O: files, pipes and network.
#[cfg(windows)]
#[expect(unsafe_code, reason = "reads the I/O counters of a process")]
pub(super) fn work(pid: u32) -> Option<u64> {
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

/// Unknown on other systems: only output shows activity.
#[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
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

#[cfg(all(test, any(target_os = "linux", target_vendor = "apple", windows)))]
mod tests {
    use std::io::Write;

    use super::*;

    /// Written to disk: macOS counts the disk's bytes only.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "writes a file to produce I/O")]
    fn work_is_counted() {
        let before = work(std::process::id()).unwrap();
        let path = std::env::temp_dir().join(format!("{}-work-{}", crate::NAME, std::process::id()));
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(&vec![1; 1 << 20]).unwrap();
        file.sync_all().unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(work(std::process::id()).unwrap() > before);
    }
}
