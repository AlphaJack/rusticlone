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
    let timer = TimerFd::new(ClockId::CLOCK_REALTIME, TimerFlags::TFD_NONBLOCK | TimerFlags::TFD_CLOEXEC)?;
    let flags = TimerSetTimeFlags::TFD_TIMER_ABSTIME | TimerSetTimeFlags::TFD_TIMER_CANCEL_ON_SET;
    // Through a Duration: `TimeSpec::new` takes the target's `time_t`, 32 bits wide on armv7.
    let since_epoch = Duration::try_from(at.as_duration()).map_err(std::io::Error::other)?;
    timer.set(Expiration::OneShot(TimeSpec::from_duration(since_epoch)), flags)?;
    readable(timer.into()).await
}
