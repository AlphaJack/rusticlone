//! What macOS and Linux share: they tell the connection's cost on a change, which is kept until asked.

use std::sync::atomic::{AtomicBool, Ordering};

/// The connection's cost as the system last told it.
static METERED: AtomicBool = AtomicBool::new(false);

/// Keeps the connection's cost the system told, and tells of the change.
pub(super) fn told_metered(metered: bool) {
    METERED.store(metered, Ordering::Relaxed);
    super::changes::changed();
}

pub(super) fn metered() -> bool {
    METERED.load(Ordering::Relaxed)
}
