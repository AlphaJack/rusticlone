//! Scheduled runs that wait for the power cable or for a network that is not metered when settings ask. Runs
//! started by hand still run.

use jiff::SignedDuration;

use crate::{settings, sys};

/// What scheduled runs wait for when [`settings::SKIP_ON_BATTERY`] and [`settings::SKIP_ON_METERED`] ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, crate::i18n::Fluent)]
pub(crate) enum Waiting {
    Power,
    UnmeteredNetwork,
}

/// How often a run that waits checks again.
pub(super) const WAIT_CHECK: SignedDuration = SignedDuration::from_mins(5);

/// What scheduled runs wait for now. A setting that does not read as true or false is off.
pub(super) fn waiting() -> Option<Waiting> {
    let on = |s: &settings::Setting| s.enabled().unwrap_or_default();
    if on(&settings::SKIP_ON_BATTERY) && sys::on_battery() {
        Some(Waiting::Power)
    } else if on(&settings::SKIP_ON_METERED) && sys::metered() {
        Some(Waiting::UnmeteredNetwork)
    } else {
        None
    }
}
