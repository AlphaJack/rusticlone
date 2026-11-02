//! macOS's wall clock timer: a kqueue timer on the calendar clock.

use super::unix::readable;
use jiff::Timestamp;

/// Uses the clock that keeps running during sleep, unlike the one tokio uses.
#[expect(unsafe_code, reason = "create and configure an owned kqueue timer")]
pub(in crate::server) async fn wall_timer(at: Timestamp) -> std::io::Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    // SAFETY: a new descriptor, owned at once.
    let queue = match unsafe { libc::kqueue() } {
        -1 => return Err(std::io::Error::last_os_error()),
        fd => unsafe { OwnedFd::from_raw_fd(fd) },
    };
    let timer = libc::kevent {
        ident: 0,
        filter: libc::EVFILT_TIMER,
        flags: libc::EV_ADD | libc::EV_ONESHOT,
        fflags: libc::NOTE_ABSOLUTE | libc::NOTE_NSECONDS | libc::NOTE_MACH_CONTINUOUS_TIME,
        data: at.as_nanosecond() as libc::intptr_t,
        udata: std::ptr::null_mut(),
    };
    // SAFETY: one change from a valid event, none read back.
    if unsafe { libc::kevent(queue.as_raw_fd(), &raw const timer, 1, std::ptr::null_mut(), 0, std::ptr::null()) } == -1
    {
        return Err(std::io::Error::last_os_error());
    }
    readable(queue).await
}
