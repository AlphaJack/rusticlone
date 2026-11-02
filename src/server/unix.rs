//! What Unix's server does its own way: restarting in place, and reusing its address.

/// Starts the updated executable as this same process, which the service manager keeps following.
#[cfg(feature = "self-update")]
pub(super) fn restart(command: &mut std::process::Command) -> anyhow::Result<()> {
    Err(std::os::unix::process::CommandExt::exec(command).into())
}

/// As std does, so that a restart need not wait for the old connections to time out.
pub(super) fn reuse_address(socket: &socket2::Socket) -> std::io::Result<()> {
    socket.set_reuse_address(true)
}
