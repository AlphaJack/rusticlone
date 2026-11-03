//! Browsing, searching, comparing and downloading snapshots.

use std::{process::Stdio, sync::Arc};

use askama::Template;
use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
};
use futures_util::{StreamExt, stream};
use quick_cache::{
    DefaultHashBuilder, OptionsBuilder, Weighter,
    sync::{Cache, DefaultLifecycle},
};
use serde::Deserialize;
use tokio::io::AsyncBufReadExt;

use super::{
    Access, App, Base, Button, Error, Heading, Hint, Label, Missing, Result, Route, Size, blocking, layout, render_str,
    repo::with_repo,
};
use crate::{
    engine::Engine,
    fmt, log,
    profile::RusticProfile,
    rustic::{self, Ctx, Diff, Dirs, Found, NodeKind, SnapshotId},
    server::Fault,
};

/// Snapshots the cache is sized for, before entries count.
const BROWSED_SNAPSHOTS: usize = 64;
/// Directory entries cached, about 100 bytes each.
const BROWSED_ENTRIES: u64 = 500_000;

/// Browsed snapshots, by repository. A snapshot never changes, so they need no invalidation.
pub(in crate::server) struct Browsed(Cache<(String, SnapshotId), Arc<Dirs>, Entries>);

/// A snapshot's cache weight: its entry count, capped at half of [`BROWSED_ENTRIES`] so that a large
/// snapshot is still cached.
#[derive(Clone)]
struct Entries;

impl Weighter<(String, SnapshotId), Arc<Dirs>> for Entries {
    fn weight(&self, _: &(String, SnapshotId), dirs: &Arc<Dirs>) -> u64 {
        (dirs.values().map(Vec::len).sum::<usize>() as u64).min(BROWSED_ENTRIES / 2)
    }
}

impl Browsed {
    pub(in crate::server) fn new() -> anyhow::Result<Self> {
        let options = OptionsBuilder::new()
            .estimated_items_capacity(BROWSED_SNAPSHOTS)
            .weight_capacity(BROWSED_ENTRIES)
            .shards(1)
            .build()?;
        Ok(Self(Cache::with_options(options, Entries, DefaultHashBuilder::default(), DefaultLifecycle::default())))
    }

    /// Every directory of a snapshot. Rustic lists it once, also for concurrent requests; later
    /// requests read the cache. Failures are not cached.
    fn get(&self, engine: &Engine, repo: &RusticProfile, snapshot: &SnapshotId) -> anyhow::Result<Arc<Dirs>> {
        let key = (repo.name.clone(), snapshot.clone());
        self.0.get_or_insert_with(&key, || engine.rustic.ls(repo, snapshot).map(Arc::new))
    }
}

#[derive(Deserialize, Default)]
pub(super) struct PathQuery {
    #[serde(default)]
    path: String,
    #[serde(default)]
    dir: bool,
}

struct Entry {
    name: String,
    kind: NodeKind,
    path: String,
    /// `None` for a directory.
    size: Option<Size>,
    modified: String,
}

#[derive(Template)]
#[template(path = "browse.html")]
struct BrowseMain {
    collection: String,
    title: String,
    repo: String,
    id: SnapshotId,
    short: String,
    at: String,
    /// Each part of the path, and its link.
    crumbs: Vec<(String, String)>,
    entries: Vec<Entry>,
    error: Option<String>,
}

pub(super) async fn browse(
    State(app): State<Arc<App>>,
    access: Access,
    base: Base,
    Path((collection, repo, id)): Path<(String, String, SnapshotId)>,
    Query(q): Query<PathQuery>,
) -> Response {
    blocking(move || {
        let Some(result) = with_repo(&app, &collection, &repo, |e, _, p| {
            // From the stored listing, without a rustic call.
            let snapshot = match e.store.snapshots(&collection, &p.name).into_iter().find(|s| s.id == id) {
                Some(snapshot) => snapshot,
                None => e.rustic.snapshot(&Ctx::default(), p, &id)?,
            };
            let path = opened(&q.path, &snapshot.paths);
            let dirs = app.browsed.get(e, p, &id);
            let nodes = dirs.and_then(|dirs| {
                let dir = dirs.get(path.trim_matches('/')).ok_or_else(|| Fault::NoDirectory { path: path.clone() })?;
                Ok(dir.clone())
            });
            anyhow::Ok((snapshot, path, nodes))
        }) else {
            return Err(Error::NotFound(Missing::Repository));
        };
        let (snapshot, path, nodes) = result.map_err(|e| Error::NotFound(Missing::Named(format!("{e:#}"))))?;
        let crumbs = crumbs(&path);
        let (entries, error) = match nodes {
            Ok(nodes) => (entries(&path, nodes), None),
            Err(e) => (Vec::new(), Some(format!("{e:#}"))),
        };
        let browse = BrowseMain {
            title: app.engine().title(&collection),
            short: snapshot.id.short().to_owned(),
            at: fmt::local(snapshot.time),
            collection,
            repo,
            id,
            crumbs,
            entries,
            error,
        };
        let title = format!("{} | {} | {}", browse.short, browse.repo, browse.title);
        Ok(layout(&title, None, access, &base, render_str(&browse, &base)))
    })
    .await
}

/// Where a snapshot opens: at `asked`, else in its one directory, as the tree names it.
fn opened(asked: &str, paths: &[String]) -> String {
    match (asked.is_empty(), paths) {
        (true, [only]) if std::path::Path::new(only).is_absolute() => rustic::in_tree(only),
        _ => asked.to_owned(),
    }
}

/// Each part of `path`, after the root, with the path it links to.
fn crumbs(path: &str) -> Vec<(String, String)> {
    let mut crumbs = vec![("/".to_owned(), String::new())];
    let mut at = String::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        at = format!("{at}/{part}");
        crumbs.push((part.to_owned(), at.clone()));
    }
    crumbs
}

/// The entries of the directory `path`: directories first, each sorted by name, and the size of files only.
fn entries(path: &str, mut nodes: Vec<rustic::Node>) -> Vec<Entry> {
    nodes.sort_by(|a, b| (b.kind == NodeKind::Dir).cmp(&(a.kind == NodeKind::Dir)).then(a.name.cmp(&b.name)));
    (nodes.into_iter())
        .map(|n| Entry {
            path: format!("{}/{}", path.trim_end_matches('/'), n.name),
            kind: n.kind,
            size: (n.kind != NodeKind::Dir).then_some(Size(Some(n.size))),
            modified: n.mtime.map(fmt::local).unwrap_or_default(),
            name: n.name,
        })
        .collect()
}

/// The name a download is saved as: the last part of `path`, else `whole` for the snapshot, `.zip` when zipped,
/// with `_` for what a header's quoted name cannot hold.
fn file_name(path: &str, whole: &str, zip: bool) -> String {
    let base = path.rsplit('/').find(|p| !p.is_empty()).unwrap_or(whole);
    let name = if zip { format!("{base}.zip") } else { base.to_owned() };
    name.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' { c } else { '_' }).collect()
}

#[derive(Deserialize)]
pub(super) struct FindQuery {
    #[serde(default)]
    name: String,
}

#[derive(Template)]
#[template(path = "find.html")]
struct FindMain {
    collection: String,
    title: String,
    repo: String,
    name: String,
    found: Vec<Found>,
    error: Option<String>,
}

/// Finds a name in the collection's snapshots in a repository.
pub(super) async fn find(
    State(app): State<Arc<App>>,
    access: Access,
    base: Base,
    Path((collection, repo)): Path<(String, String)>,
    Query(q): Query<FindQuery>,
) -> Response {
    blocking(move || {
        let name = q.name.trim().to_owned();
        let result = with_repo(&app, &collection, &repo, |e, j, p| {
            (!name.is_empty()).then(|| e.rustic.find(p, j.primary.groups(), &name))
        })
        .ok_or(Error::NotFound(Missing::Repository))?;
        let (found, error) = match result {
            Some(Ok(found)) => (found, None),
            Some(Err(e)) => (Vec::new(), Some(format!("{e:#}"))),
            None => (Vec::new(), None),
        };
        let main = FindMain { title: app.engine().title(&collection), collection, repo, name, found, error };
        let title = format!("{} | {} | {}", Button::Find, main.repo, main.title);
        Ok(layout(&title, None, access, &base, render_str(&main, &base)))
    })
    .await
}

#[derive(Template)]
#[template(path = "diff.html")]
struct DiffMain {
    collection: String,
    title: String,
    repo: String,
    id: SnapshotId,
    short: String,
    /// The previous snapshot of the same sources.
    before: Option<(SnapshotId, String)>,
    /// Each change (`+`, `-`, `M`...) and its path.
    changes: Vec<(String, String)>,
    totals: Vec<String>,
    error: Option<String>,
}

/// What changed since the previous snapshot of the same sources in that repository.
pub(super) async fn diff(
    State(app): State<Arc<App>>,
    access: Access,
    base: Base,
    Path((collection, repo, id)): Path<(String, String, SnapshotId)>,
) -> Response {
    blocking(move || {
        let listed = app.engine().store.snapshots(&collection, &repo);
        let this = listed.iter().find(|s| s.id == id).ok_or(Error::NotFound(Missing::Snapshot))?;
        let before = listed.iter().filter(|s| s.source() == this.source() && s.time < this.time).max_by_key(|s| s.time);
        let diff = before.map(|b| with_repo(&app, &collection, &repo, |e, _, p| e.rustic.diff(p, &b.id, &id)));
        let (Diff { changes, totals }, error) = match diff.flatten() {
            Some(Ok(diff)) => (diff, None),
            Some(Err(e)) => (Diff::default(), Some(format!("{e:#}"))),
            None => (Diff::default(), None),
        };
        let main = DiffMain {
            title: app.engine().title(&collection),
            short: this.id.short().to_owned(),
            before: before.map(|b| (b.id.clone(), fmt::local(b.time))),
            collection,
            repo,
            id,
            changes,
            totals,
            error,
        };
        let title = format!("{} | {} | {} | {}", Button::Changes, main.short, main.repo, main.title);
        Ok(layout(&title, None, access, &base, render_str(&main, &base)))
    })
    .await
}

/// Kills rustic's process group on drop, unless it exited: the download was abandoned.
struct Dump(tokio::process::Child);

impl Drop for Dump {
    #[cfg_attr(test, mutants::skip)] // Kills a process.
    fn drop(&mut self) {
        // `None` once it exited and was reaped.
        if let Some(pid) = self.0.id() {
            rustic::kill(pid);
        }
    }
}

/// Streams a file as it is, or a directory as a zip archive, from `rustic dump`.
pub(super) async fn download(
    State(app): State<Arc<App>>,
    Path((collection, repo, id)): Path<(String, String, SnapshotId)>,
    Query(q): Query<PathQuery>,
) -> Result {
    let zip = q.dir || q.path.is_empty();
    let command = with_repo(&app, &collection, &repo, |e, _, p| e.rustic.dump(p, &id, &q.path, zip))
        .ok_or(Error::NotFound(Missing::Repository))?;
    let mut command = tokio::process::Command::from(command);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(Fault::CannotRunRustic)?;
    rustic::tie_child(&child);
    let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(Fault::NotPiped.into());
    };
    let rustic_profile = repo;
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(err).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            log::rustic(&rustic_profile, &line);
        }
    });
    let mut dump = Dump(child);
    // Graceful shutdown waits for every response, so a stopping server ends the stream.
    let mut stopping = app.stopping.clone();
    let data = tokio_util::io::ReaderStream::new(out).take_until(async move {
        _ = stopping.wait_for(|s| *s).await;
    });
    // After the headers, a failure can only cut the download short, so that it is not taken as complete.
    let stopping = app.stopping.clone();
    let end = stream::once(async move {
        if *stopping.borrow() {
            return Err(std::io::Error::other(Fault::Stopping));
        }
        match dump.0.wait().await {
            Ok(status) if status.success() => Ok(Bytes::new()),
            _ => Err(std::io::Error::other(Fault::DumpFailed)),
        }
    });
    let body = Body::from_stream(data.chain(end));
    let name = file_name(&q.path, &format!("{collection}-{}", id.short()), zip);
    let mime = if zip { "application/zip" } else { "application/octet-stream" };
    Ok((
        [
            (header::CONTENT_TYPE, mime.to_owned()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
        ],
        body,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rustic::Node;

    fn node(name: &str, kind: NodeKind, size: u64) -> Node {
        Node { name: name.into(), kind, size, mtime: None }
    }

    /// A snapshot of one absolute directory opens there, as the tree names it. One of several, or one asked for a
    /// path, opens where asked.
    #[test]
    fn a_snapshot_opens_in_its_one_directory() {
        let one = ["/home/me/docs".to_owned()];
        assert_eq!(opened("", &one), "/home/me/docs");
        assert_eq!(opened("/home", &one), "/home");
        assert_eq!(opened("", &["/a".to_owned(), "/b".to_owned()]), "");
        assert_eq!(opened("", &["docs".to_owned()]), "", "a relative path is not in the tree as it is");
    }

    /// The crumbs lead to each directory above, from the root.
    #[test]
    fn crumbs_lead_to_each_directory_above() {
        let crumbs = crumbs("/home//me/");
        let expected = [("/", ""), ("home", "/home"), ("me", "/home/me")].map(|(a, b)| (a.to_owned(), b.to_owned()));
        assert_eq!(crumbs, expected);
    }

    /// Directories come first, each by name, and only files have a size.
    #[test]
    fn directories_are_listed_first_and_files_with_their_size() {
        let nodes = vec![
            node("b.txt", NodeKind::File, 3),
            node("z", NodeKind::Dir, 0),
            node("a.txt", NodeKind::File, 5),
            node("c", NodeKind::Dir, 0),
        ];
        let listed: Vec<(String, String, bool)> =
            entries("/docs/", nodes).into_iter().map(|e| (e.name, e.path, e.size.is_some())).collect();
        let expected = [
            ("c", "/docs/c", false),
            ("z", "/docs/z", false),
            ("a.txt", "/docs/a.txt", true),
            ("b.txt", "/docs/b.txt", true),
        ];
        assert_eq!(listed, expected.map(|(n, p, s)| (n.to_owned(), p.to_owned(), s)));
    }

    /// A download is saved under the last part of its path, the snapshot's name without one, and nothing a quoted
    /// header name cannot hold.
    #[test]
    fn a_download_is_named_after_what_it_holds() {
        assert_eq!(file_name("/home/me/docs/", "Docs-12345678", true), "docs.zip");
        assert_eq!(file_name("", "Docs-12345678", true), "Docs-12345678.zip");
        assert_eq!(file_name("/a/b.txt", "Docs-12345678", false), "b.txt");
        assert_eq!(file_name("/a/my \"file\\é.txt", "x", false), "my__file__.txt");
    }

    /// A snapshot weighs as many entries as it lists, at most half the cache, so that a large one is still kept.
    #[test]
    fn a_snapshot_weighs_its_entries() {
        let key = ("Docs".to_owned(), "1".repeat(64).parse().unwrap());
        let dirs = |n: usize| {
            Arc::new(Dirs::from([
                ("/".to_owned(), vec![node("a", NodeKind::File, 0); n]),
                ("/b".to_owned(), vec![node("c", NodeKind::File, 0)]),
            ]))
        };
        assert_eq!(Entries.weight(&key, &dirs(2)), 3);
        let large = usize::try_from(BROWSED_ENTRIES).unwrap();
        assert_eq!(Entries.weight(&key, &dirs(large)), BROWSED_ENTRIES / 2);
    }
}
