//! The size of a folder to back up and its largest folders. The setup form shows it next to the folder while
//! it is typed: the first backup reads all of it.

#![expect(clippy::disallowed_methods, reason = "measures the folder the setup form names")]

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use askama::Template;
use axum::{extract::Query, response::Html};

use super::{Base, render_str};
use crate::{first_run, fmt};

/// How long a folder is measured: a home folder may hold millions of files.
const BUDGET: Duration = Duration::from_secs(3);
/// The largest folders named.
const LARGEST: usize = 3;

/// The size of a folder as far as it was measured. `largest`: its largest folders after a colon or blank.
#[derive(crate::i18n::Fluent)]
enum Measured {
    /// No folder given.
    #[fluent(transparent = "")]
    Blank,
    /// Not a folder or not there: mistyped or on a disk that is not mounted.
    Missing,
    #[fluent(size = fmt::human_size(*bytes))]
    Bytes { bytes: u64, files: u64, largest: String },
    /// The budget ran out first.
    #[fluent(size = fmt::human_size(*bytes))]
    AtLeast { bytes: u64, files: u64, largest: String },
}

/// The bytes and files under `dir` until `until`, without following links. `false` if it ran out of time.
fn walk(dir: &Path, until: Instant, bytes: &mut u64, files: &mut u64) -> bool {
    let mut left = vec![dir.to_path_buf()];
    while let Some(dir) = left.pop() {
        if Instant::now() > until {
            return false;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                left.push(entry.path());
            } else if meta.is_file() {
                *bytes += meta.len();
                *files += 1;
            }
        }
    }
    true
}

impl Measured {
    fn of(folder: &str) -> Self {
        let path = PathBuf::from(folder);
        if folder.is_empty() {
            return Self::Blank;
        }
        if !path.is_dir() {
            return Self::Missing;
        }
        let until = Instant::now() + BUDGET;
        let (mut bytes, mut files, mut done) = (0, 0, true);
        let mut folders: BTreeMap<String, u64> = BTreeMap::new();
        for entry in std::fs::read_dir(&path).into_iter().flatten().flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                let mut size = 0;
                done &= walk(&entry.path(), until, &mut size, &mut files);
                bytes += size;
                folders.insert(entry.file_name().to_string_lossy().into_owned(), size);
            } else if meta.is_file() {
                bytes += meta.len();
                files += 1;
            }
        }
        let mut folders: Vec<(String, u64)> = folders.into_iter().filter(|(_, size)| *size > 0).collect();
        folders.sort_by_key(|(_, size)| std::cmp::Reverse(*size));
        let largest = (folders.iter().take(LARGEST))
            .map(|(name, size)| format!("{name} {}", fmt::human_size(*size)))
            .collect::<Vec<_>>()
            .join(", ");
        let largest = if largest.is_empty() { largest } else { format!(": {largest}") };
        if done { Self::Bytes { bytes, files, largest } } else { Self::AtLeast { bytes, files, largest } }
    }
}

#[derive(Template)]
#[template(
    source = "<span{% if let Measured::Bytes { .. } = measured %}{% else %} class=\"warning-text\"{% endif %}>{{ measured }}</span>",
    ext = "html"
)]
struct MeasuredView {
    measured: Measured,
}

/// The size of the folder of the field that asks whatever the field's name.
pub(super) async fn size(base: Base, Query(field): Query<BTreeMap<String, String>>) -> Html<String> {
    let folder = field.into_values().next().unwrap_or_default();
    let measured = tokio::task::spawn_blocking(move || Measured::of(&first_run::home(first_run::pasted(&folder))))
        .await
        .unwrap_or(Measured::Blank);
    Html(render_str(&MeasuredView { measured }, &base))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_a_folder_and_names_its_largest() {
        let dir = crate::sys::test_dir("size");
        std::fs::create_dir_all(dir.join("big")).unwrap();
        std::fs::write(dir.join("big").join("a"), vec![0; 4096]).unwrap();
        std::fs::write(dir.join("b"), vec![0; 10]).unwrap();
        let Measured::Bytes { bytes, files, largest } = Measured::of(&dir.display().to_string()) else { panic!() };
        assert_eq!((bytes, files), (4106, 2));
        assert!(largest.starts_with(": big "), "{largest}");
        assert!(matches!(Measured::of(""), Measured::Blank));
        assert!(matches!(Measured::of("/no/such/folder"), Measured::Missing));
    }
}
