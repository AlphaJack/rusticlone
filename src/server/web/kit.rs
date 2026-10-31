//! The emergency kit's download: a zip of every profile with its `info.txt` in the page's language. See [`crate::kit`].

#![expect(clippy::disallowed_methods, reason = "reads the profiles into the kit")]

use axum::{
    http::header,
    response::{IntoResponse, Response},
};

use super::{blocking, now};
use crate::{fmt, profile, sys};

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
