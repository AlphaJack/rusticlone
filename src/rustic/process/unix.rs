//! What Linux and macOS do alike with rustic's processes: they end a process group with a signal.

/// Kills rustic. On Unix, also the processes it started, which share its process group.
#[expect(unsafe_code, reason = "signals a process group by its id")]
pub(crate) fn kill(pid: u32) {
    // SAFETY: a plain syscall.
    unsafe {
        libc::kill(-pid.cast_signed(), libc::SIGKILL);
    }
}

/// rustic inherits the run's locks here, so it ends with them: see `cli::inherit_locks`.
#[cfg(feature = "server")]
pub(crate) fn tie_child(_: &tokio::process::Child) {}

/// Rustic runs at its normal priority here: systemd's `Nice` and launchd's `Background` lower it.
pub(crate) fn quiet(_: &mut std::process::Command) {}

/// Its own process group, which [`kill`] ends whole.
pub(crate) fn own_group(cmd: &mut std::process::Command) {
    std::os::unix::process::CommandExt::process_group(cmd, 0);
}

/// Rustic inherits the run's locks, `locks`, so that they stay held while it runs, even if this process is
/// killed.
pub(crate) fn inherit_locks(cmd: &mut std::process::Command, locks: &[i32]) {
    let locks = locks.to_vec();
    // SAFETY: only fcntl, async-signal-safe, between fork and exec.
    #[expect(unsafe_code, reason = "pass lock descriptors through fork and exec with fcntl")]
    unsafe {
        std::os::unix::process::CommandExt::pre_exec(cmd, move || {
            for fd in &locks {
                libc::fcntl(*fd, libc::F_SETFD, 0);
            }
            Ok(())
        })
    };
}

/// Rustic holds the run's locks itself here: see [`inherit_locks`].
pub(crate) fn tie_spawned(_: &std::process::Child) {}
