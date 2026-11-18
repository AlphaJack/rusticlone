//! Windows keeps the computer awake while the thread that asked runs and until it asks no more.

/// Held until dropped: see the module.
pub(crate) struct Awake(());

#[expect(unsafe_code, reason = "SetThreadExecutionState takes flags only")]
pub(crate) fn stay_awake() -> anyhow::Result<Awake> {
    use windows_sys::Win32::System::Power::{ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState};
    // SAFETY: flags only. It returns the previous state, or 0 when refused.
    match unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) } {
        0 => Err(std::io::Error::last_os_error().into()),
        _ => Ok(Awake(())),
    }
}

#[expect(unsafe_code, reason = "SetThreadExecutionState takes flags only")]
impl Drop for Awake {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Power::{ES_CONTINUOUS, SetThreadExecutionState};
        // SAFETY: flags only.
        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
    }
}

#[cfg(test)]
mod tests {
    use super::stay_awake;

    #[test]
    #[expect(unsafe_code, reason = "SetThreadExecutionState takes flags only")]
    fn the_thread_requires_the_system_until_dropped() {
        use windows_sys::Win32::System::Power::{ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState};
        // Each call returns the state before it: each sets the same state again but the last.
        let state = |flags| unsafe { SetThreadExecutionState(flags) };
        let awake = stay_awake().unwrap();
        assert_eq!(state(ES_CONTINUOUS | ES_SYSTEM_REQUIRED), ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
        drop(awake);
        assert_eq!(state(ES_CONTINUOUS), ES_CONTINUOUS);
    }
}
