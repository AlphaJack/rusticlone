//! Threads this process starts.

/// What a thread returned, or its panic, raised again in the thread that joined it.
pub(crate) fn joined<T>(result: std::thread::Result<T>) -> T {
    result.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}
