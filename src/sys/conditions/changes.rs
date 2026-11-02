//! Who hears that this computer went on or off battery, or that its connection's cost changed.

use std::sync::OnceLock;

/// Called on each change.
static CHANGED: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Sets who hears the changes, if no one does yet: true when it did.
pub(super) fn listen(on_change: Box<dyn Fn() + Send + Sync>) -> bool {
    CHANGED.set(on_change).is_ok()
}

/// Tells of a change.
pub(super) fn changed() {
    if let Some(changed) = CHANGED.get() {
        changed();
    }
}
