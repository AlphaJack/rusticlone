//! Waiting for wall clock times, which schedules use, across suspends and clock changes.

#![expect(clippy::disallowed_methods, reason = "reads the wall clock")]

use std::time::Duration;

use jiff::Timestamp;

use super::Logged;
use crate::log;

/// How often a plain sleep checks the wall clock, because it stops during a suspend.
const SUSPEND_CHECK: Duration = Duration::from_secs(60);

/// Waits until `at` by the wall clock, also across a suspend. Forever if `None`.
pub(super) async fn wall_clock(at: Option<Timestamp>) {
    let Some(at) = at else { return std::future::pending().await };
    if let Err(e) = wall_timer(at).await {
        log::event(None, &Logged::NoWallClock { error: e.to_string() });
        // A sleep stops during a suspend: check at least every minute.
        let left = Duration::try_from(at.duration_since(Timestamp::now())).unwrap_or_default();
        tokio::time::sleep(left.min(SUSPEND_CHECK)).await;
    }
}

/// Also wakes on a clock change, so that what is due is checked again.
#[cfg(target_os = "linux")]
pub(super) async fn wall_timer(at: Timestamp) -> std::io::Result<()> {
    use nix::sys::{
        time::TimeSpec,
        timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags},
    };
    let timer = TimerFd::new(ClockId::CLOCK_REALTIME, TimerFlags::TFD_NONBLOCK | TimerFlags::TFD_CLOEXEC)?;
    let flags = TimerSetTimeFlags::TFD_TIMER_ABSTIME | TimerSetTimeFlags::TFD_TIMER_CANCEL_ON_SET;
    timer.set(Expiration::OneShot(TimeSpec::new(at.as_second(), at.subsec_nanosecond().into())), flags)?;
    readable(timer.into()).await
}

/// Uses the clock that keeps running during sleep, unlike the one tokio uses.
#[cfg(target_vendor = "apple")]
#[expect(unsafe_code, reason = "create and configure an owned kqueue timer")]
pub(super) async fn wall_timer(at: Timestamp) -> std::io::Result<()> {
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

#[cfg(any(target_os = "linux", target_vendor = "apple"))]
#[expect(unsafe_code, reason = "register an owned timer descriptor with the runtime")]
pub(super) async fn readable(fd: std::os::fd::OwnedFd) -> std::io::Result<()> {
    // Readable only: macOS refuses to watch a kqueue for writing.
    // SAFETY: an owned descriptor stays open, and the same, until the AsyncFd drops it.
    let fd = unsafe { tokio::io::unix::AsyncFd::register_with_interest(fd, tokio::io::Interest::READABLE) }?;
    fd.readable().await.map(drop)
}

/// An absolute due time, which Windows keeps through clock changes and signals after a sleep.
#[cfg(windows)]
#[expect(unsafe_code, reason = "own and wait for Windows kernel timer handles")]
pub(super) async fn wall_timer(at: Timestamp) -> std::io::Result<()> {
    use std::{ptr::null, sync::Arc};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
        System::Threading::{
            CreateEventW, CreateWaitableTimerW, INFINITE, SetEvent, SetWaitableTimer, WaitForMultipleObjects,
        },
    };
    /// In units of 100 ns since 1601.
    const UNIX_EPOCH: i64 = 116_444_736_000_000_000;
    /// The timer, and an event that ends the wait early.
    struct Handles([HANDLE; 2]);
    // SAFETY: kernel handles work from any thread.
    unsafe impl Send for Handles {}
    unsafe impl Sync for Handles {}
    impl Drop for Handles {
        fn drop(&mut self) {
            // SAFETY: theirs, closed once.
            for h in self.0.iter().filter(|h| !h.is_null()) {
                unsafe { CloseHandle(*h) };
            }
        }
    }
    /// Ends the wait when the awaiting future is dropped.
    struct Ends(Arc<Handles>);
    impl Drop for Ends {
        fn drop(&mut self) {
            // SAFETY: an event still open, `Handles` closes it after the wait.
            unsafe { SetEvent(self.0.0[1]) };
        }
    }
    // SAFETY: plain calls; `Handles` closes what they open.
    let handles = unsafe { Handles([CreateWaitableTimerW(null(), 1, null()), CreateEventW(null(), 1, 0, null())]) };
    if handles.0.iter().any(|h| h.is_null()) {
        return Err(std::io::Error::last_os_error());
    }
    let due = i64::try_from(at.as_nanosecond() / 100).unwrap_or(i64::MAX).saturating_add(UNIX_EPOCH);
    // SAFETY: a timer just created, an absolute due time, no completion routine.
    if unsafe { SetWaitableTimer(handles.0[0], &raw const due, 0, None, null(), 0) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let handles = Arc::new(handles);
    let ends = Ends(handles.clone());
    // SAFETY: both handles stay open while waited for: the thread holds them.
    let waited =
        tokio::task::spawn_blocking(move || unsafe { WaitForMultipleObjects(2, handles.0.as_ptr(), 0, INFINITE) });
    let waited = waited.await.map_err(std::io::Error::other)?;
    drop(ends);
    if waited == WAIT_OBJECT_0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

/// On other systems, a sleep may stop during a suspend.
#[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
pub(super) async fn wall_timer(_: Timestamp) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

#[cfg(all(test, any(target_os = "linux", target_vendor = "apple", windows)))]
mod tests {
    use super::*;

    #[test]
    fn wall_timer_fires() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let at = Timestamp::now() + Duration::from_millis(50);
        runtime.block_on(wall_timer(at)).unwrap();
        assert!(Timestamp::now() >= at);
    }
}
