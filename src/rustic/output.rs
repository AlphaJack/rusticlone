//! What rustic printed: progress, failures, and skipped files.

use serde::{Deserialize, Serialize};

use super::SnapshotId;
use crate::{fmt, i18n::t};

/// A `--json-progress` status line.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub(crate) struct Progress {
    pub percent_done: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_done: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds_remaining: Option<u64>,
}

impl Progress {
    /// `43% (1.2 GiB of 2.9 GiB, about 3m left)`.
    pub(crate) fn summary(&self) -> String {
        let detail = self.detail();
        let detail = if detail.is_empty() { detail } else { format!(" ({detail})") };
        format!("{:.0}%{detail}", self.percent_done * 100.0)
    }

    /// `1.2 GiB of 2.9 GiB, about 3m left`, as far as known.
    pub(crate) fn detail(&self) -> String {
        let size = self
            .bytes_done
            .zip(self.total_bytes)
            .map(|(done, total)| format!("{} of {}", fmt::human_size(done), fmt::human_size(total)));
        let left = self.seconds_remaining.filter(|s| *s > 0).map(|s| format!("about {} left", fmt::human_duration(s)));
        size.into_iter().chain(left).collect::<Vec<_>>().join(", ")
    }
}

#[derive(Deserialize)]
pub(super) struct Message {
    pub(super) message_type: String,
    #[serde(flatten)]
    pub(super) progress: Progress,
}

/// The first absolute path in a message: quoted (`` `/a/b` ``), or up to a colon. On Windows, from a
/// drive (`C:\a`) or a share (`\\server\a`).
fn first_path(message: &str) -> Option<String> {
    let absolute = |rest: &str| {
        rest.starts_with('/')
            || rest.starts_with(r"\\")
            || matches!(rest.as_bytes(), [d, b':', b'\\', ..] if d.is_ascii_alphabetic())
    };
    let start = (0..message.len()).find(|&i| {
        (i == 0 || matches!(message.as_bytes()[i - 1], b' ' | b'`' | b'\'' | b'"'))
            && message.get(i..).is_some_and(absolute)
    })?;
    let rest = message.get(start..)?;
    let end = rest.find(['`', '\'', '"', '\n']).or_else(|| rest.find(": ")).unwrap_or(rest.len());
    Some(rest.get(..end)?.trim_end_matches([':', '.']).to_owned())
}

pub(crate) struct Output {
    pub success: bool,
    pub stdout: String,
    pub(super) command: String,
    /// `None` if a signal killed it.
    pub(super) code: Option<i32>,
    pub(super) stderr: Log,
}

/// The parts of rustic's log that are read: `[WARN]` and `[ERROR]` entries with their
/// continuation lines, and the explanation of a failure (the line after `Message:`, else the
/// `error:` line).
#[derive(Default)]
pub(super) struct Log {
    error: Option<String>,
    /// Details of the error: `- path: ...` and similar lines, and its cause.
    context: Vec<String>,
    cause: Option<String>,
    /// The section of the error report being read.
    section: Option<Section>,
    warnings: Vec<String>,
    errors: Vec<String>,
    /// `[INFO]` entries, where some commands report what they did.
    info: Vec<String>,
    /// The entry that an unmarked line continues.
    continued: Option<Entry>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Message,
    Context,
    Cause,
}

#[derive(Clone, Copy)]
enum Entry {
    Warning,
    Error,
}

impl Log {
    pub(super) fn line(&mut self, line: &str) {
        let line = line.trim().to_owned();
        if let Some(rest) = line.strip_prefix("[WARN] ") {
            self.warnings.push(rest.to_owned());
            self.continued = Some(Entry::Warning);
        } else if let Some(rest) = line.strip_prefix("[ERROR] ") {
            self.errors.push(rest.to_owned());
            self.continued = Some(Entry::Error);
        } else if let Some(rest) = line.strip_prefix("[INFO] ") {
            self.info.push(rest.to_owned());
            self.continued = None;
        } else if line.starts_with('[') || line.starts_with("error:") {
            // `error:` is rustic's final error, not part of the entry before it.
            self.continued = None;
        } else if line.is_empty() {
            // Between the sections of an error's report, which continues.
        } else if let Some(entry) = match self.continued {
            Some(Entry::Warning) => self.warnings.last_mut(),
            Some(Entry::Error) => self.errors.last_mut(),
            None => None,
        } {
            entry.push('\n');
            entry.push_str(&line);
        }
        match (self.section, line.as_str()) {
            (_, "Message:") => self.section = Some(Section::Message),
            (_, "Context:") => self.section = Some(Section::Context),
            (_, "Caused by:") => self.section = Some(Section::Cause),
            (Some(Section::Message | Section::Cause), "") => {}
            (Some(Section::Message), _) => (self.error, self.section) = (Some(line), None),
            (Some(Section::Context), entry) if entry.starts_with("- ") => {
                self.context.push(entry.trim_start_matches("- ").to_owned());
            }
            (Some(Section::Cause), _) => (self.cause, self.section) = (Some(line), None),
            (_, _) => {
                self.section = None;
                if let Some(e) = line.strip_prefix("error:").filter(|_| self.error.is_none()) {
                    self.error = Some(e.trim().to_owned());
                }
            }
        }
    }

    /// The error with its details: `... (path: ...; Permission denied)`.
    fn explanation(&self) -> Option<String> {
        let error = self.error.as_ref()?;
        let details: Vec<&str> = self.context.iter().chain(&self.cause).map(String::as_str).collect();
        Some(if details.is_empty() { error.clone() } else { format!("{error} ({})", details.join("; ")) })
    }
}

/// Rustic's warning when a backup cannot read an item.
const IGNORED: &str = "ignoring error";
/// Rustic's warning when a backup cannot read the snapshot it compares with.
const PARENT: &str = "ignoring error when loading parent tree";
/// Rustic's warning when a file system has no extended attributes, as a container's mount of a macOS
/// folder: the item is saved without them, so it is not left out.
const XATTRS: &str = "Error getting xattrs";
/// Rustic's error when a backup of one source fails: `error backing up <path>: <cause>`.
const NOT_BACKED_UP: &str = "error backing up ";
/// Rustic's cause when it cannot resolve a source's path.
const UNRESOLVED: &str = "error sanitizing source";
/// Rustic's sizes after a prune.
const UNUSED: &str = "unused size after prune:";
/// Windows's errors for a file that another program holds open: a sharing or a lock violation.
const IN_USE: [&str; 2] = ["(os error 32)", "(os error 33)"];

/// An `[ERROR]` entry as its first words and its explanation, as rustic's final error: without what rustic adds
/// for its developers, such as a request to report a bug and the backtrace, nor its name for the kind of error.
fn summary(report: &str) -> String {
    let mut log = Log::default();
    report.lines().skip(1).for_each(|l| log.line(l));
    let first = report.lines().next().unwrap_or_default();
    let what = first.split(RUSTIC_CORE).next().unwrap_or(first);
    log.explanation().map_or_else(|| first.to_owned(), |e| format!("{what}: {e}"))
}

/// How rustic names a kind of error after what failed: ``error copying to target: `rustic_core` experienced an
/// error related to `the backend`.``
const RUSTIC_CORE: &str = ": `rustic_core`";

/// The cause in an error's report: its innermost one, with the file it concerns, rather than rustic's
/// first line, which only names a kind of error.
fn innermost(report: &str) -> String {
    let after = |heading: &str| report.lines().skip_while(|l| l.trim() != heading).nth(1).map(str::trim);
    let first = report.lines().next().unwrap_or_default().trim();
    if first.starts_with(UNRESOLVED) {
        return t!("output-unresolved");
    }
    let Some(cause) = after("Caused by:") else { return first.to_owned() };
    let cause = cause.split(" (os error").next().unwrap_or(cause);
    match after("Message:").and_then(first_path) {
        Some(path) => format!("{cause}: {path}"),
        None => cause.to_owned(),
    }
}

/// The path in a warning, and its innermost cause without the OS error number, also from an error in
/// Rust's debug form (`` `Os { code: 13, kind: PermissionDenied, message: "Permission denied" }` ``).
fn path_and_cause(warning: &str) -> Option<(String, String)> {
    let path = first_path(warning)?;
    let reason = warning.rsplit(": ").next().unwrap_or_default();
    let reason = match reason.strip_prefix('"').and_then(|r| r.split_once('"')) {
        Some((message, _)) => message,
        None => reason.split(" (os error").next().unwrap_or(reason),
    };
    Some((path, reason.trim().to_owned()))
}

impl Output {
    /// The text after `key` in an `[INFO]` entry, such as prune's `remaining:`.
    fn info(&self, key: &str) -> Option<&str> {
        self.stderr.info.iter().find_map(|i| i.strip_prefix(key)).map(str::trim)
    }

    /// Parses `total prune: 3 blobs, 1.2 GiB`, `remaining: 9138 blobs, 2.5 GiB` and
    /// `unused size after prune: 483.9 MiB (18.61% of remaining size)`.
    pub(super) fn pruned(&self) -> Option<Pruned> {
        let size = |key| Some(self.info(key)?.rsplit(',').next()?.trim().to_owned());
        let (removed, remaining) = (size("total prune:")?, size("remaining:")?);
        let unused = self.info(UNUSED)?.split(" (").next()?.to_owned();
        Some(Pruned { removed, remaining, unused })
    }

    pub(super) fn ended(&self) -> String {
        self.code.map_or(Ended::Killed, |code| Ended::Exit { code }).to_string()
    }

    pub(crate) fn failure(&self) -> Failure {
        let explained = self.stderr.explanation().filter(|_| !self.success);
        let said = explained.or_else(|| self.stderr.errors.last().map(|e| summary(e)));
        Failure { command: self.command.clone(), ended: (!self.success).then(|| self.ended()), said }
    }

    /// What a backup left out, and why. Rustic only logs it as `[WARN] ignoring error ...`, and saves the
    /// snapshot anyway.
    pub(crate) fn skipped(&self) -> Vec<(String, String)> {
        let mut skipped: Vec<(String, String)> = Vec::new();
        let warnings = (self.stderr.warnings.iter())
            .filter(|w| w.starts_with(IGNORED) && !w.starts_with(PARENT) && !w.contains(XATTRS));
        for (warning, (path, reason)) in warnings.filter_map(|w| Some((w, path_and_cause(w)?))) {
            let reason = if IN_USE.iter().any(|e| warning.contains(e)) { t!("output-in-use") } else { reason };
            if !skipped.iter().any(|(p, _)| *p == path) {
                skipped.push((path, reason));
            }
        }
        skipped
    }

    /// The sources whose backup failed, with rustic's cause. Hooks may have prepared or removed them, so
    /// only rustic knows.
    pub(crate) fn not_backed_up(&self) -> Vec<(String, String)> {
        let failed = self.stderr.errors.iter().filter_map(|e| e.strip_prefix(NOT_BACKED_UP)?.split_once(": "));
        failed.map(|(path, report)| (path.to_owned(), innermost(report))).collect()
    }

    /// Data that a backup could not read from the previous snapshot, such as a missing pack. The backup
    /// then read every file again.
    pub(crate) fn unreadable_parent(&self) -> Option<(String, String)> {
        self.stderr.warnings.iter().filter(|w| w.starts_with(PARENT)).find_map(|w| path_and_cause(w))
    }

    /// With `--json-progress`, `backup` reports its snapshots only in summary lines.
    pub(crate) fn summary_snapshot_ids(&self) -> Vec<SnapshotId> {
        self.stdout
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v["message_type"] == "summary")
            .filter_map(|v| v["snapshot_id"].as_str()?.parse().ok())
            .collect()
    }
}

#[derive(crate::i18n::Fluent)]
pub(crate) enum Ended {
    Exit { code: i32 },
    Killed,
}

/// The sizes a prune reported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pruned {
    pub removed: String,
    pub remaining: String,
    pub unused: String,
}

/// Why a command failed, or the error it reported while exiting 0. `ended`: how a failed command
/// ended. `said`: its error.
#[derive(Debug, Clone, crate::i18n::Fluent)]
#[fluent(
    prefix = "rustic-failure",
    said = said.as_deref().unwrap_or_default(),
    ended = ended.as_deref().unwrap_or_default(),
    has_said = said.is_some(),
    has_ended = ended.is_some()
)]
pub(crate) struct Failure {
    pub command: String,
    pub ended: Option<String>,
    pub said: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rustic::fixtures;

    fn output(command: &str, stderr: &str, success: bool) -> Output {
        let mut log = Log::default();
        for line in stderr.lines() {
            log.line(line);
        }
        Output { success, stdout: String::new(), command: command.into(), code: Some(i32::from(!success)), stderr: log }
    }

    /// Rustic 0.11.4 with a pack of the previous snapshot deleted. This is not a skipped item.
    #[test]
    fn unreadable_parent_is_not_a_skipped_item() {
        let out = output(
            "backup",
            "[WARN] ignoring error when loading parent tree TreeId(942ffe4f): Error: Failed to open the file \
             `/r/data/57/57e1`. Please check the file and try again. (kind: related to the backend): caused by: No \
             such file or directory (os error 2)",
            true,
        );
        assert!(out.skipped().is_empty());
        assert_eq!(out.unreadable_parent(), Some(("/r/data/57/57e1".into(), "No such file or directory".into())));
    }

    #[test]
    fn what_a_prune_did() {
        let stderr = "[INFO] to repack:          0 packs,          0 blobs,        0 B
[INFO] total prune:                          0 blobs,        0 B
[INFO] remaining:                         9138 blobs,    2.5 GiB
[INFO] unused size after prune:  483.9 MiB (18.61% of remaining size)
[INFO] nothing to do!";
        let sizes = |removed: &str, remaining: &str, unused: &str| Pruned {
            removed: removed.into(),
            remaining: remaining.into(),
            unused: unused.into(),
        };
        assert_eq!(output("prune", stderr, true).pruned().unwrap(), sizes("0 B", "2.5 GiB", "483.9 MiB"));
        assert_eq!(output("prune", "", true).pruned(), None);
    }

    #[test]
    fn sources_not_backed_up_are_read_from_rustic() {
        for (_, stderr) in fixtures("backup-not-backed-up.stderr") {
            let not_backed_up = output("backup", &stderr, false).not_backed_up();
            assert_eq!(not_backed_up, [("/home/me/gone".into(), t!("output-unresolved"))]);
        }
        for (_, stderr) in fixtures("backup-read-only.stderr") {
            let file = "/backup/Docs/data/d0/d0296cff9dc416fd17915b28e6b232f0826fb9a4a592e1cb3b069a0c7ae3c6ec-tmp-";
            let not_backed_up = output("backup", &stderr, false).not_backed_up();
            assert_eq!(not_backed_up, [("/home/me/docs".into(), format!("Permission denied: {file}"))]);
        }
    }

    /// As rustic 0.11 says it on Windows, in the language of the system.
    #[test]
    fn a_file_in_use_on_windows_is_skipped_with_what_to_do() {
        let stderr = "[WARN] ignoring error: Error: Failed to open ReadSourceOpen at `C:\\Users\\me\\mail.pst`\n\
             Failed to open file at `C:\\Users\\me\\mail.pst`. (kind: related to input/output operations): caused by: \
             Der Prozess kann nicht auf die Datei zugreifen. (os error 32)\n";
        let skipped = output("backup", stderr, true).skipped();
        assert_eq!(skipped, [(r"C:\Users\me\mail.pst".to_owned(), t!("output-in-use"))]);
        assert_eq!(first_path(r"at \\nas\share\a.txt: denied").as_deref(), Some(r"\\nas\share\a.txt"));
    }

    #[test]
    fn what_rustic_skipped_and_why() {
        for (_, stderr) in fixtures("backup-skipped.stderr") {
            insta::assert_debug_snapshot!("backup_skipped", output("backup", &stderr, true).skipped());
        }
    }

    /// An item saved without extended attributes is not left out; an OS error in debug form gives its message.
    #[test]
    fn missing_xattrs_are_not_skipped_items() {
        let stderr = "[WARN] ignoring error obtaining xargs: Error getting xattrs for `\"/d/a\"`: `Os { code: 95, kind: \
                      Unsupported, message: \"Not supported\" }`\n\
                      [WARN] ignoring error: Failed to open `/d/b`: `Os { code: 13, kind: PermissionDenied, message: \
                      \"Permission denied\" }`\n";
        let skipped = output("backup", stderr, true).skipped();
        assert_eq!(skipped, [("/d/b".to_owned(), "Permission denied".to_owned())]);
    }

    #[test]
    fn failures_say_what_rustic_said() {
        for (_, stderr) in fixtures("copy-refused.stderr") {
            insta::assert_snapshot!("copy_refused", output("copy", &stderr, true).failure().to_string());
        }
        for (_, stderr) in fixtures("copy-unreachable.stderr") {
            let said = output("copy", &stderr, true).failure().to_string();
            assert!(said.contains("error copying to target: Path `config` does not exist."), "{said}");
            assert!(
                !said.contains("We believe") && !said.contains("Backtrace") && !said.contains("rustic_core"),
                "{said}"
            );
        }
        for (_, stderr) in fixtures("unreadable.stderr") {
            insta::assert_snapshot!("unreadable", output("snapshots", &stderr, false).failure().to_string());
        }
        assert_eq!(
            output("copy", "", true).failure().to_string(),
            "rustic copy exited 0 without doing it; see rustic's log"
        );
    }
}
