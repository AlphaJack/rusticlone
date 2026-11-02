//! What Linux's and macOS's timers share: a descriptor the runtime waits on.

#[expect(unsafe_code, reason = "register an owned timer descriptor with the runtime")]
pub(super) async fn readable(fd: std::os::fd::OwnedFd) -> std::io::Result<()> {
    // Readable only: macOS refuses to watch a kqueue for writing.
    // SAFETY: an owned descriptor stays open, and the same, until the AsyncFd drops it.
    let fd = unsafe { tokio::io::unix::AsyncFd::register_with_interest(fd, tokio::io::Interest::READABLE) }?;
    fd.readable().await.map(drop)
}
