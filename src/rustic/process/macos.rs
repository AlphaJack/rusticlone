//! macOS's view of a process's work: its disk I/O in its resource usage.

pub(crate) use super::unix::*;

/// Bytes read from and written to disk so far. Unlike Linux, macOS does not count a remote's traffic.
#[expect(unsafe_code, reason = "fills a resource usage record of a process")]
pub(in crate::rustic) fn work(pid: u32) -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: a record of the size that the flavor asks for.
    let filled =
        unsafe { libc::proc_pid_rusage(pid.cast_signed(), libc::RUSAGE_INFO_V2, usage.as_mut_ptr().cast()) } == 0;
    // SAFETY: filled by the call, which returned 0.
    let usage = filled.then(|| unsafe { usage.assume_init() })?;
    Some(usage.ri_diskio_bytesread + usage.ri_diskio_byteswritten)
}
