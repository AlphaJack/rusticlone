//! What Windows's server does its own way: restarting as a new process.

/// Starts the updated executable as a new process, after which this one ends. As the service, it ends with an error
/// instead, after which the service manager starts it again.
#[cfg(feature = "self-update")]
pub(super) fn restart(command: &mut std::process::Command) -> anyhow::Result<()> {
    if super::service::windows::running() {
        return Err(Fault::Restarting.into());
    }
    let flags = windows_sys::Win32::System::Threading::DETACHED_PROCESS;
    std::os::windows::process::CommandExt::creation_flags(command, flags).spawn()?;
    Ok(())
}

#[cfg(feature = "self-update")]
#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
enum Fault {
    /// Ends the service after an update, so that the service manager starts the new executable.
    Restarting,
}

/// Windows lets another process take an address it reuses, so it is not asked for.
#[expect(clippy::unnecessary_wraps, reason = "Unix's may fail")]
pub(super) fn reuse_address(_: &socket2::Socket) -> std::io::Result<()> {
    Ok(())
}
