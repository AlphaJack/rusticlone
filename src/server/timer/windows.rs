//! Windows's wall clock timer: a waitable timer that wakes the computer's clock, not its uptime.

use jiff::Timestamp;

/// An absolute due time, which Windows keeps through clock changes and signals after a sleep.
#[expect(unsafe_code, reason = "own and wait for Windows kernel timer handles")]
pub(in crate::server) async fn wall_timer(at: Timestamp) -> std::io::Result<()> {
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
