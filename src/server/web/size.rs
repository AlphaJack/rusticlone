//! The size of a folder to back up. The setup form shows it next to the folder while it is typed: the first
//! backup reads all of it.

#![expect(clippy::disallowed_methods, reason = "measures the folder the setup form names")]

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use askama::Template;
use axum::{extract::Query, response::Html};

use super::{Base, render_str};
use crate::{first_run, fmt};

/// The size of a folder.
#[derive(crate::i18n::Fluent)]
enum Measured {
    /// No folder given, or no longer asked for.
    #[fluent(transparent = "")]
    Blank,
    /// Not a folder or not there: mistyped or on a disk that is not mounted.
    Missing,
    #[fluent(size = fmt::human_size(*bytes))]
    Bytes { bytes: u64, files: u64 },
}

impl Measured {
    /// All of `folder`, read on all cores without following links, until `stop` is set.
    fn of(folder: &str, stop: &Arc<AtomicBool>) -> Self {
        let path = Path::new(folder);
        if folder.is_empty() {
            return Self::Blank;
        }
        if !path.is_dir() {
            return Self::Missing;
        }
        let stopped = Arc::clone(stop);
        // Each file's size, read where its folder is listed, as that runs in parallel.
        let walk = jwalk::WalkDirGeneric::<((), u64)>::new(path).skip_hidden(false).process_read_dir(
            move |_, _, (), children| {
                let stop = stopped.load(Ordering::Relaxed);
                for child in children.iter_mut().flatten() {
                    if stop {
                        child.read_children = None;
                    } else if child.file_type.is_file() {
                        child.client_state = child.metadata().map_or(0, |m| m.len());
                    }
                }
            },
        );
        let (mut bytes, mut files) = (0, 0);
        for entry in walk.into_iter().flatten().filter(|e| e.file_type.is_file()) {
            bytes += entry.client_state;
            files += 1;
        }
        if stop.load(Ordering::Relaxed) { Self::Blank } else { Self::Bytes { bytes, files } }
    }
}

/// Stops the measuring on drop: the form asked for another folder, or was left.
struct Stop(Arc<AtomicBool>);

impl Drop for Stop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[derive(Template)]
#[template(
    source = "<span{% if let Measured::Missing = measured %} class=\"warning-text\"{% endif %}{% if let Measured::Bytes { bytes, .. } = measured %} data-bytes=\"{{ bytes }}\"{% endif %}>{{ measured }}</span>",
    ext = "html"
)]
struct MeasuredView {
    measured: Measured,
}

/// The size of the folder of the field that asks whatever the field's name.
pub(super) async fn size(base: Base, Query(field): Query<BTreeMap<String, String>>) -> Html<String> {
    let folder = field.into_values().next().unwrap_or_default();
    let stop = Stop(Arc::default());
    let flag = Arc::clone(&stop.0);
    let measured =
        tokio::task::spawn_blocking(move || Measured::of(&first_run::home(first_run::pasted(&folder)), &flag))
            .await
            .unwrap_or(Measured::Blank);
    Html(render_str(&MeasuredView { measured }, &base))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_all_of_a_folder() {
        let dir = crate::sys::test_dir("size");
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join(".hidden").join("a"), vec![0; 4096]).unwrap();
        std::fs::write(dir.join("b"), vec![0; 10]).unwrap();
        let measure = |folder: &str| Measured::of(folder, &Arc::default());
        let Measured::Bytes { bytes, files } = measure(&dir.display().to_string()) else { panic!() };
        assert_eq!((bytes, files), (4106, 2));
        assert!(matches!(measure(""), Measured::Blank));
        assert!(matches!(measure("/no/such/folder"), Measured::Missing));
        let stopped = Measured::of(&dir.display().to_string(), &Arc::new(AtomicBool::new(true)));
        assert!(matches!(stopped, Measured::Blank));
    }
}
