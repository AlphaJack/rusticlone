//! Linux's wall clock timer: a timerfd that also wakes on a clock change.

use super::unix::readable;
use jiff::Timestamp;
use std::time::Duration;

/// Also wakes on a clock change, so that what is due is checked again.
pub(in crate::server) async fn wall_timer(at: Timestamp) -> std::io::Result<()> {
    use nix::sys::{
        time::TimeSpec,
        timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags},
    };
    /// Read without blocking the runtime, and not passed on to the programs the server starts.
    const TIMER: TimerFlags = TimerFlags::TFD_NONBLOCK.union(TimerFlags::TFD_CLOEXEC);
    /// At a time of the wall clock, and woken when someone sets the clock.
    const AT: TimerSetTimeFlags =
        TimerSetTimeFlags::TFD_TIMER_ABSTIME.union(TimerSetTimeFlags::TFD_TIMER_CANCEL_ON_SET);
    let timer = TimerFd::new(ClockId::CLOCK_REALTIME, TIMER)?;
    // Through a Duration: `TimeSpec::new` takes the target's `time_t`, 32 bits wide on armv7.
    let since_epoch = Duration::try_from(at.as_duration()).map_err(std::io::Error::other)?;
    timer.set(Expiration::OneShot(TimeSpec::from_duration(since_epoch)), AT)?;
    readable(timer.into()).await
}
