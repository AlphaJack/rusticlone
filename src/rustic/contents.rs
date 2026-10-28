//! Snapshot contents as rustic lists them, for browsing.

use std::{
    collections::BTreeMap,
    path::{Component, Path},
};

use jiff::Timestamp;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Node {
    pub name: String,
    pub kind: NodeKind,
    pub size: u64,
    /// To the minute.
    pub mtime: Option<Timestamp>,
}

/// Also the name of its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub(crate) enum NodeKind {
    Dir,
    File,
    Symlink,
}

/// Directory entries by path below the snapshot root (`""` for the root).
pub(crate) type Dirs = BTreeMap<String, Vec<Node>>;

/// Parses `ls --long --recursive` of rustic 0.11, because `--json` has neither sizes nor times. A
/// line is `MODE USER GROUP SIZE TIME "PATH"`, with the time in UTC (`15 Oct 2026 13:20`) or `?`.
pub(super) fn dirs(listing: &str) -> Dirs {
    let mut dirs = Dirs::new();
    for line in listing.lines() {
        let Some((info, path)) = quoted(line) else { continue };
        let fields: Vec<&str> = info.split_whitespace().collect();
        let [mode, _, _, size, time @ ..] = fields.as_slice() else { continue };
        let kind = match mode.chars().next() {
            Some('d') => NodeKind::Dir,
            Some('l') => NodeKind::Symlink,
            _ => NodeKind::File,
        };
        let mtime = jiff::civil::DateTime::strptime("%d %b %Y %H:%M", time.join(" ")).ok();
        let mtime = mtime.and_then(|t| t.to_zoned(jiff::tz::TimeZone::UTC).ok()).map(|t| t.timestamp());
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", &path));
        let node = Node { name: name.to_owned(), kind, size: size.parse().unwrap_or(0), mtime };
        dirs.entry(parent.to_owned()).or_default().push(node);
        // Lists empty directories too.
        if kind == NodeKind::Dir {
            dirs.entry(path.clone()).or_default();
        }
    }
    dirs
}

const HEX: u32 = 16;

/// Splits a line that ends with a path in Rust's `{:?}` format into its start and the path.
fn quoted(line: &str) -> Option<(&str, String)> {
    let (start, end) = (line.find('"')?, line.rfind('"')?);
    let mut path = String::new();
    let mut chars = line.get(start + 1..end)?.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            path.push(c);
            continue;
        }
        match chars.next() {
            None => {}
            Some('n') => path.push('\n'),
            Some('r') => path.push('\r'),
            Some('t') => path.push('\t'),
            Some('0') => path.push('\0'),
            // `\u{301}`.
            Some('u') => {
                let hex: String = chars.by_ref().skip(1).take_while(|c| *c != '}').collect();
                path.extend(u32::from_str_radix(&hex, HEX).ok().and_then(char::from_u32));
            }
            Some(escaped) => path.push(escaped),
        }
    }
    Some((line.get(..start)?, path))
}

/// Where a snapshot's source is in its tree: `/home/me` there too, `C:\Users\me` at `/C/Users/me`.
pub(crate) fn in_tree(path: &str) -> String {
    let mut tree = String::new();
    for c in Path::new(path).components() {
        match c {
            Component::Normal(part) => tree += &format!("/{}", part.to_string_lossy()),
            Component::Prefix(prefix) => {
                for part in prefix.as_os_str().to_string_lossy().split(['\\', '/', ':', '?']).filter(|p| !p.is_empty())
                {
                    tree += &format!("/{part}");
                }
            }
            Component::RootDir | Component::CurDir | Component::ParentDir => {}
        }
    }
    tree
}

/// Snapshots that contain the same matching files.
#[derive(Debug)]
pub(crate) struct Found {
    /// The first of these snapshots, and its time.
    pub id: String,
    pub time: String,
    /// How many other snapshots contain them. Rustic does not say which.
    pub more: usize,
    /// Mode, owner, size and time of each file, and its path.
    pub files: Vec<(String, String)>,
}

/// How `rustic find` starts each snapshot.
const FOUND_IN: &str = "found in ";

/// Parses `rustic find`. Snapshots with the same files appear once, as the first snapshot and a count
/// (`... (+2)`), followed by the files.
pub(super) fn found(report: &str) -> Vec<Found> {
    let mut found: Vec<Found> = Vec::new();
    for line in report.lines() {
        if let Some(rest) = line.strip_prefix(FOUND_IN) {
            let (id, time) = rest.split_once(" from ").unwrap_or((rest, ""));
            let (time, more) = match time.rsplit_once(" (+") {
                Some((time, more)) => (time, more.trim_end_matches(')').parse().unwrap_or(0)),
                None => (time, 0),
            };
            found.push(Found { id: id.to_owned(), time: time.to_owned(), more, files: Vec::new() });
        } else if let (Some(f), Some((info, path))) = (found.last_mut(), quoted(line)) {
            // Paths are relative to the snapshot root.
            f.files.push((info.trim().to_owned(), format!("/{path}")));
        }
    }
    found
}

#[derive(Debug, Default)]
pub(crate) struct Diff {
    /// The kind of change ("added", "changed"...) and the path.
    pub changes: Vec<(String, String)>,
    /// Rustic's non-zero counts: "files: 1 added, 2 changed".
    pub totals: Vec<String>,
}

pub(super) fn diff(report: &str) -> Diff {
    let mut diff = Diff::default();
    for line in report.lines() {
        match quoted(line) {
            Some((kind, path)) if kind.trim().len() == 1 => {
                diff.changes.push((change(kind.trim()), format!("/{path}")));
            }
            _ => diff.totals.extend(totals(line)),
        }
    }
    diff
}

/// A change in `rustic diff`, by its symbol.
#[derive(strum::EnumString, crate::i18n::Fluent)]
enum Change {
    #[strum(serialize = "=")]
    Identical,
    #[strum(serialize = "+")]
    Added,
    #[strum(serialize = "-")]
    Removed,
    #[strum(serialize = "M")]
    Changed,
    #[strum(serialize = "U")]
    Metadata,
    #[strum(serialize = "T")]
    Type,
}

/// The name of rustic's change symbol. Unknown symbols are returned unchanged.
fn change(symbol: &str) -> String {
    symbol.parse::<Change>().map_or_else(|_| symbol.to_owned(), |c| c.to_string())
}

/// `Files   : 0 =, 1 +, 1 -, 1 M, 0 U` becomes `files: 1 added, 1 removed, 1 changed`, and
/// `NodeType: 2 changed` becomes `2 type changed`.
fn totals(line: &str) -> Option<String> {
    let (what, counts) = line.split_once(':')?;
    let counts: Vec<String> = counts
        .split(',')
        .filter_map(|c| c.trim().split_once(' '))
        .filter(|(n, _)| *n != "0")
        .map(|(n, kind)| match what {
            "NodeType" => format!("{n} {}", Change::Type),
            _ => format!("{n} {}", change(kind)),
        })
        .collect();
    match what {
        _ if counts.is_empty() => None,
        "NodeType" => Some(counts.join(", ")),
        _ => Some(format!("{}: {}", what.trim().to_lowercase(), counts.join(", "))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sources_are_found_in_the_tree() {
        assert_eq!(in_tree("/home/me/docs"), "/home/me/docs");
        #[cfg(windows)]
        assert_eq!(in_tree(r"C:\Users\me\docs"), "/C/Users/me/docs");
    }

    #[test]
    fn what_changed() {
        for (_, report) in crate::rustic::fixtures("diff.stdout") {
            insta::assert_debug_snapshot!("diff", diff(&report));
        }
    }

    #[test]
    fn found_files_by_snapshot() {
        for (_, report) in crate::rustic::fixtures("find.stdout") {
            insta::assert_debug_snapshot!("find", found(&report));
        }
        for (_, report) in crate::rustic::fixtures("find-unchanged.stdout") {
            insta::assert_debug_snapshot!("find_unchanged", found(&report));
        }
    }

    #[test]
    fn a_long_listing_by_directory() {
        for (_, listing) in crate::rustic::fixtures("ls.stdout") {
            insta::assert_debug_snapshot!("ls", dirs(&listing));
        }
    }
}
