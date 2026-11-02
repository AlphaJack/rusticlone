//! What Windows's server does its own way: restarting as a new process.

/// Starts the updated executable as a new process, after which this one ends.
#[cfg(feature = "self-update")]
pub(super) fn restart(command: &mut std::process::Command) -> anyhow::Result<()> {
    let flags = windows_sys::Win32::System::Threading::DETACHED_PROCESS;
    std::os::windows::process::CommandExt::creation_flags(command, flags).spawn()?;
    Ok(())
}

/// Windows lets another process take an address it reuses, so it is not asked for.
#[expect(clippy::unnecessary_wraps, reason = "Unix's may fail")]
pub(super) fn reuse_address(_: &socket2::Socket) -> std::io::Result<()> {
    Ok(())
}
