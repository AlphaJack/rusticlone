//! Other Unix systems, such as the BSDs: what rusticlone cannot tell there yet.

use std::path::Path;

pub(super) use super::unix::read_hostname;
pub(crate) use super::unix::*;

pub(crate) fn needs_disk_access(_: &str) -> bool {
    false
}

pub(super) fn solid_state(_: &Path) -> Option<bool> {
    None
}

pub(super) fn network_fs(_: &Path) -> bool {
    false
}
