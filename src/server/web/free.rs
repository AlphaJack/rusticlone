//! The free space of the local disk the first collection is saved to, which the setup form shows next to its
//! folder while it is typed: one system call, unlike the size of the folders to back up.

use std::{collections::BTreeMap, path::Path};

use askama::Template;
use axum::{extract::Query, response::Html};

use super::{Base, render_str};
use crate::{first_run, fmt, sys};

/// Where removable and network disks are mounted, and where each user's are: a folder whose nearest existing
/// ancestor is one of them is on a disk that is not mounted. So is one whose nearest is the root, or mistyped:
/// no one keeps backups in a new folder there.
const MOUNTS: [&str; 5] = ["/", "/Volumes", "/mnt", "/media", "/run/media"];
const USER_MOUNTS: [&str; 2] = ["/media", "/run/media"];

/// The space left where a folder is, or would be.
#[derive(crate::i18n::Fluent)]
enum Free {
    /// No folder given.
    #[fluent(transparent = "")]
    Blank,
    /// Its disk is not mounted, or the folder is mistyped.
    Missing,
    #[fluent(size = fmt::human_size(*bytes))]
    Bytes { bytes: u64 },
}

impl Free {
    fn of(folder: &str) -> Self {
        let mounted = |measured: &Path| {
            !MOUNTS.iter().any(|m| measured == Path::new(m))
                && !USER_MOUNTS.iter().any(|m| measured.parent() == Some(Path::new(m)))
        };
        match sys::free_space(folder.as_ref()) {
            _ if folder.is_empty() => Self::Blank,
            Some((measured, free)) if mounted(measured) => Self::Bytes { bytes: free },
            _ => Self::Missing,
        }
    }
}

#[derive(Template)]
#[template(path = "free.html")]
struct FreeView {
    free: Free,
}

/// The free space where the folder of the field that asks is, whatever the field's name.
pub(super) async fn free(base: Base, Query(field): Query<BTreeMap<String, String>>) -> Html<String> {
    let folder = field.into_values().next().unwrap_or_default();
    let free = tokio::task::spawn_blocking(move || Free::of(&first_run::repository_folder(&folder)))
        .await
        .unwrap_or(Free::Missing);
    Html(render_str(&FreeView { free }, &base))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_mounted_folder_has_free_space() {
        assert_eq!(Free::of("").to_string(), "");
        assert!(matches!(Free::of(&std::env::temp_dir().display().to_string()), Free::Bytes { .. }));
        for unmounted in ["/Volumes/Backup/rusticlone", "/media/someone/usb", "/not-a-folder/rusticlone"] {
            assert_eq!(Free::of(unmounted).to_string(), "folder or disk not found", "{unmounted}");
        }
    }
}
