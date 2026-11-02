//! Unix's archives are `.tar.gz`, and a file runs if its mode says so.

use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};

pub(crate) use super::untar as unpack;

/// The downloaded executable's mode.
const EXECUTABLE: u32 = 0o755;

pub(super) fn executable(options: &mut OpenOptions) {
    options.mode(EXECUTABLE);
}

#[cfg(test)]
#[expect(clippy::disallowed_methods, reason = "reads the installed file's mode")]
pub(super) fn is_executable(path: &std::path::Path) -> bool {
    std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(path).unwrap().permissions()) & 0o777 == EXECUTABLE
}
