//! Linux's view of a process's work: its I/O in `/proc`.

pub(crate) use super::unix::*;

/// Bytes read and written so far, from `/proc`, network included. Not CPU time: rustic spins while it waits for a
/// stdin command.
#[expect(clippy::disallowed_methods, reason = "reads /proc")]
pub(in crate::rustic) fn work(pid: u32) -> Option<u64> {
    let io = std::fs::read_to_string(format!("/proc/{pid}/io")).ok()?;
    let io = io.lines().filter(|l| l.starts_with("rchar") || l.starts_with("wchar"));
    Some(io.filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).sum())
}
