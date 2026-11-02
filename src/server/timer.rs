//! Waiting for wall clock times, which schedules use, across suspends and clock changes.

#![expect(clippy::disallowed_methods, reason = "reads the wall clock")]

use std::time::Duration;

use jiff::Timestamp;

use super::Logged;
use crate::log;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(unix)]
mod unix;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

use imp::wall_timer;

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

#[cfg(test)]
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
