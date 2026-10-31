//! The emergency kit's download: a zip of every profile with its `info.txt` in the page's language. See [`crate::kit`].

#![expect(clippy::disallowed_methods, reason = "reads the profiles into the kit")]

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderName, StatusCode, header},
    response::{IntoResponse, Response},
};

use super::{App, Base, Error, Notice, Route, blocking, flash, now};
use crate::{
    request::{RunKind, Selection},
    state::Event,
};

/// How long the upload waits for the server to read the saved profiles before it leaves their restore to the
/// schedule.
const READ_WITHIN: std::time::Duration = std::time::Duration::from_secs(15);
use crate::{first_run, fmt, profile, sys};

/// Downloads the kit always under [`crate::kit::FILE`] so that a newer one replaces it. Its `info.txt` says where
/// and when it was made.
pub(super) async fn kit_download() -> Response {
    blocking(|| {
        let profiles: Vec<(String, String)> = (profile::profile_files()?.into_iter())
            .filter_map(|(name, dir)| {
                let file = format!("{name}.toml");
                Some((file.clone(), std::fs::read_to_string(dir.join(file)).ok()?))
            })
            .collect();
        let host = sys::hostname();
        let (time, user) = (fmt::local(now()), sys::user());
        let made = crate::kit::Made { host, time: &time, user: &user, root: sys::is_root() };
        let zip = crate::kit::render(&profiles, &made)?;
        let disposition = format!("attachment; filename=\"{}\"", crate::kit::FILE);
        let headers =
            [(header::CONTENT_TYPE, "application/zip".to_owned()), (header::CONTENT_DISPOSITION, disposition)];
        Ok((headers, zip).into_response())
    })
    .await
}

/// Saves the profiles of an uploaded kit where the setup saves them, which the watcher then reads: a fresh
/// install gets its collections back. The body is the zip itself. A profile that is there with other contents
/// refuses the whole kit, as `--kit` does.
///
/// Its new collections are held first, so that nothing backs up the empty folders of this computer: see
/// [`Event::RestoreFirst`]. Once the server reads them, it restores them, and the page opens the restore when
/// there is one.
pub(super) async fn kit_upload(State(app): State<Arc<App>>, base: Base, body: Bytes) -> Response {
    blocking(move || {
        let refused = |error: String| Error::Refused(Notice::KitNotSaved { error });
        let dir = first_run::dir().ok_or_else(|| refused(first_run::Error::NoDir.to_string()))?;
        let profiles = crate::kit::parse(std::io::Cursor::new(body)).map_err(|e| refused(format!("{e:#}")))?;
        let new: Vec<String> = (crate::kit::primaries(&profiles).into_iter())
            .filter(|name| !dir.join(format!("{name}.toml")).exists())
            .collect();
        let store = &app.engine().store;
        let hold = Event::RestoreFirst { collections: new.clone(), asked: Some(store.now()) };
        store.apply(&[hold]).map_err(|e| refused(format!("{e:#}")))?;
        match crate::kit::unpack(profiles, &dir) {
            Ok(saved) => {
                let saved = Notice::KitSaved { saved: saved as u64, dir: dir.display().to_string() };
                Ok(restore(&app, &base, &new).unwrap_or_else(|| flash(&saved)))
            }
            Err(e) => {
                // Nothing was saved: a collection made later under the same name is not held.
                let at = store.now();
                let released: Vec<Event> =
                    new.into_iter().map(|collection| Event::Restored { collection, at, ok: true }).collect();
                _ = store.apply(&released);
                Err(refused(format!("{e:#}")))
            }
        }
    })
    .await
}

/// Starts the restore of each of `collections` once the server reads them, and opens it if it is the only one.
/// `None` if there is no restore to open. Those it could not start are left to the schedule.
fn restore(app: &Arc<App>, base: &Base, collections: &[String]) -> Option<Response> {
    let deadline = std::time::Instant::now() + READ_WITHIN;
    let read = || collections.iter().all(|c| app.engine().collection(c).is_some());
    while !read() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let mut started = Vec::new();
    let mut unasked = Vec::new();
    for collection in collections {
        let selection = Selection::Collection(collection.clone());
        match app.submit(&selection, RunKind::Restore, &crate::engine::kit_restore()) {
            Ok(submitted) if !submitted.started.is_empty() => started.extend(submitted.started),
            _ => unasked.push(collection.clone()),
        }
    }
    if !unasked.is_empty() {
        // Asked for by the schedule instead, once the server reads them.
        _ = app.engine().store.apply(&[Event::RestoreFirst { collections: unasked, asked: None }]);
    }
    let [(collection, id, _)] = started.as_slice() else { return None };
    let run = base.to(Route::Run(collection, *id));
    let location = [(header::LOCATION, run.clone()), (HeaderName::from_static("hx-location"), run)];
    Some((StatusCode::CREATED, location).into_response())
}
