//! Scheduled runs that wait for the power cable, or for a network that is not metered to reach a remote
//! repository, when settings ask. Runs started by hand still run.

use std::time::Duration;

use super::Run;
use crate::{request::Trigger, result::Doing, settings, state::Holds, sys};

/// What scheduled runs wait for when [`settings::SKIP_ON_BATTERY`] and [`settings::SKIP_ON_METERED`] ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, crate::i18n::Fluent)]
pub(crate) enum Waiting {
    PowerSupply,
    UnmeteredNetwork,
}

impl Waiting {
    /// Its name as runs show what they wait for, untranslated like the other resources: `cpu`, `upload`.
    pub(super) fn resource(self) -> &'static str {
        match self {
            Self::PowerSupply => "power",
            Self::UnmeteredNetwork => "unmetered",
        }
    }
}

/// How often a run that waits asks the system again.
const WAIT_CHECK: Duration = Duration::from_mins(1);

/// What scheduled runs wait for now, and with `remote` those that reach a remote repository. A setting that does
/// not read as true or false is off.
pub(crate) fn waiting(remote: bool) -> Option<Waiting> {
    let on = |s: &settings::Setting| s.enabled().unwrap_or_default();
    waiting_for(
        (on(&settings::SKIP_ON_BATTERY), sys::on_battery),
        (remote && on(&settings::SKIP_ON_METERED), sys::metered),
    )
}

/// What to wait for with each setting on or off and how to read the system. The system is asked only for what a
/// setting turns on: reading a connection's cost starts a process on Windows.
fn waiting_for(battery: (bool, impl FnOnce() -> bool), metered: (bool, impl FnOnce() -> bool)) -> Option<Waiting> {
    if battery.0 && battery.1() {
        Some(Waiting::PowerSupply)
    } else if metered.0 && metered.1() {
        Some(Waiting::UnmeteredNetwork)
    } else {
        None
    }
}

/// Holds a scheduled `run` back while it waits, shown as waiting for `power` or `unmetered` as
/// for a lock. Returns whether it waited.
pub(super) fn wait(run: &Run<'_>, remote: bool) -> bool {
    wait_while(run, WAIT_CHECK, || waiting(remote))
}

/// [`wait`] for what `now` says, asking it again `every` so often.
pub(super) fn wait_while(run: &Run<'_>, every: Duration, mut now: impl FnMut() -> Option<Waiting>) -> bool {
    let Some(live) = run.live.as_deref().filter(|l| l.trigger() == Trigger::Schedule) else { return false };
    let (step, mut waited) = (run.cx.step(), None);
    while let Some(now) = now().filter(|_| !run.cx.stopping()) {
        if waited != Some(now) {
            let resource = now.resource().to_owned();
            run.activity(Some(Doing::WaitingFor { lock: resource.clone(), by: Vec::new() }), false);
            live.set_holds(step, Holds { using: Vec::new(), waiting_for: Some(resource) });
            waited = Some(now);
        }
        crate::clock::sleep(every, &|| run.cx.stopping());
    }
    waited.is_some()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{Waiting, waiting_for};

    /// The settings to skip on battery and on a metered network, what the system says, and what runs wait for.
    #[test]
    fn runs_wait_only_for_what_a_setting_turns_on() {
        let (power, network) = (Some(Waiting::PowerSupply), Some(Waiting::UnmeteredNetwork));
        let cases = [
            ((false, false), (true, true), None),
            ((true, false), (true, true), power),
            ((true, false), (false, true), None),
            ((false, true), (true, true), network),
            ((false, true), (true, false), None),
            ((true, true), (true, true), power),
            ((true, true), (false, true), network),
            ((true, true), (false, false), None),
        ];
        for ((skip_battery, skip_metered), (on_battery, metered), expected) in cases {
            let got = waiting_for((skip_battery, || on_battery), (skip_metered, || metered));
            assert_eq!(got, expected, "skip on battery {skip_battery}, on metered {skip_metered}");
        }
    }

    /// A setting that is off never asks the system.
    #[test]
    fn the_system_is_asked_only_when_a_setting_is_on() {
        let asked = Cell::new(0);
        let ask = || {
            asked.set(asked.get() + 1);
            true
        };
        assert_eq!(waiting_for((false, ask), (false, ask)), None);
        assert_eq!(asked.get(), 0);
    }
}
