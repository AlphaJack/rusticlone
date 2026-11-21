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

/// A progress line: only a status is one. Other lines, as a backup's summaries, are its output.
#[derive(Deserialize)]
pub(super) struct Message {
    #[expect(dead_code, reason = "only a status parses")]
    message_type: Status,
    #[serde(flatten)]
    pub(super) progress: Progress,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Status,
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
/// continuation lines, and the explanation of a failure: the line after `Message:` in rustic's final report.
#[derive(Default)]
pub(super) struct Log {
    error: Option<String>,
    /// The final `error:` line. It may only say that something failed, such as `Copying failed for at least one
    /// target!`, which an `[ERROR]` entry explains.
    last: Option<String>,
    /// Details of the error: `- path: ...` and similar lines, and its cause.
    context: Vec<String>,
    cause: Option<String>,
    /// The section of the error report being read.
    section: Option<Section>,
    warnings: Vec<String>,
    errors: Vec<String>,
    /// `[INFO]` entries, where some commands report what they did.
    info: Vec<String>,
    /// What a backup could not read, from `--json-progress`.
    unread: Vec<Unread>,
    /// The entry that an unmarked line continues.
    continued: Option<Entry>,
}

/// An item that a backup could not read, as `--json-progress` reports it on its own line.
#[derive(Deserialize)]
#[serde(tag = "message_type", rename = "error")]
struct Unread {
    item: String,
    error: Said,
}

#[derive(Deserialize)]
struct Said {
    message: String,
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
        // A JSON line, such as an unreadable item, ends the entry before it.
        if line.starts_with('{') {
            self.unread.extend(serde_json::from_str(&line).ok());
            self.continued = None;
            return;
        }
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
        // An entry's report is the entry's own, not the explanation of the command's failure.
        if self.continued.is_some() {
            return;
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
                if let Some(e) = line.strip_prefix("error:") {
                    self.last = Some(e.trim().to_owned());
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

/// Rustic's warning when a backup cannot read the snapshot it compares with.
const PARENT: &str = "ignoring error when loading parent tree";
/// Rustic's error when a backup of one source fails: `error backing up <path>: <cause>`.
const NOT_BACKED_UP: &str = "error backing up ";
/// Rustic's cause when it cannot resolve a source's path.
const UNRESOLVED: &str = "error sanitizing source";
/// Rustic's error when it asks for a password without a terminal.
const NO_TERMINAL: &str = "IO error: not a terminal";
/// Rustic's sizes after a prune.
const UNUSED: &str = "unused size after prune:";
/// Rustic's exit code when a backup saved its snapshot without the items it could not read.
pub(super) const INCOMPLETE: i32 = 3;
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

/// The path in a warning, and its innermost cause.
fn path_and_cause(warning: &str) -> Option<(String, String)> {
    Some((first_path(warning)?, cause(warning)))
}

/// The innermost cause of an error without the OS error number, also from an error in Rust's debug form
/// (`` `Os { code: 13, kind: PermissionDenied, message: "Permission denied" }` ``).
fn cause(error: &str) -> String {
    let reason = error.rsplit(": ").next().unwrap_or_default();
    let reason = match reason.strip_prefix('"').and_then(|r| r.split_once('"')) {
        Some((message, _)) => message,
        None => reason.split(" (os error").next().unwrap_or(reason),
    };
    reason.trim().to_owned()
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
        let failed = !self.success;
        let explained = self.stderr.explanation().filter(|_| failed);
        let said = explained.or_else(|| self.stderr.errors.last().map(|e| summary(e)));
        let said = said.or_else(|| self.stderr.last.clone().filter(|_| failed));
        // Rustic asks for a missing password, which fails without a terminal.
        let said = said.map(|s| if s.contains(NO_TERMINAL) { t!("output-no-password") } else { s });
        Failure { command: self.command.clone(), ended: failed.then(|| self.ended()), said }
    }

    /// What a backup left out, and why. Rustic saves the snapshot anyway, and exits [`INCOMPLETE`].
    pub(crate) fn skipped(&self) -> Vec<(String, String)> {
        let mut skipped: Vec<(String, String)> = Vec::new();
        for Unread { item, error } in &self.stderr.unread {
            let said = &error.message;
            let reason = if IN_USE.iter().any(|e| said.contains(e)) { t!("output-in-use") } else { cause(said) };
            if !skipped.iter().any(|(p, _)| p == item) {
                skipped.push((item.clone(), reason));
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    /// Progress as far as known: the size done of the total, and the time left once rustic knows it.
    #[test]
    fn progress_says_how_far_it_is() {
        let (done, total) = (1_288_490_189, 3_113_851_290);
        let progress = Progress {
            percent_done: 0.43,
            bytes_done: Some(done),
            total_bytes: Some(total),
            seconds_remaining: Some(180),
        };
        let size = format!("{} of {}", fmt::human_size(done), fmt::human_size(total));
        let left = format!("about {} left", fmt::human_duration(180));
        assert_eq!(progress.summary(), format!("43% ({size}, {left})"));
        let unknown = Progress { seconds_remaining: Some(0), ..progress };
        assert_eq!(unknown.detail(), size);
        assert_eq!(Progress { percent_done: 0.5, ..Progress::default() }.summary(), "50%");
    }

    /// A line that starts another entry, or rustic's final error, does not continue the warning before it.
    #[test]
    fn a_warning_ends_where_another_entry_starts() {
        let mut log = Log::default();
        for line in ["[WARN] cannot read /a", "caused by: denied", "error: failed", "after the error"] {
            log.line(line);
        }
        assert_eq!(log.warnings, ["cannot read /a\ncaused by: denied"]);
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

    /// As rustic 0.12 says it on Windows, in the language of the system.
    #[test]
    fn a_file_in_use_on_windows_is_skipped_with_what_to_do() {
        let stderr = r#"{"message_type":"error","error":{"message":"Error: Failed to open ReadSourceOpen at `C:\\Users\\me\\mail.pst`\nFailed to open file at `C:\\Users\\me\\mail.pst`. (kind: related to input/output operations): caused by: Der Prozess kann nicht auf die Datei zugreifen. (os error 32)"},"during":"archival","item":"C:\\Users\\me\\mail.pst"}"#;
        let skipped = output("backup", stderr, false).skipped();
        assert_eq!(skipped, [(r"C:\Users\me\mail.pst".to_owned(), t!("output-in-use"))]);
        assert_eq!(first_path(r"at \\nas\share\a.txt: denied").as_deref(), Some(r"\\nas\share\a.txt"));
    }

    #[test]
    fn what_rustic_skipped_and_why() {
        for (_, stderr) in fixtures("backup-skipped.stderr") {
            insta::assert_debug_snapshot!("backup_skipped", output("backup", &stderr, false).skipped());
        }
    }

    /// An item saved without extended attributes is only a warning, which does not leave it out. An OS error in
    /// debug form gives its message.
    #[test]
    fn missing_xattrs_are_not_skipped_items() {
        let stderr = r#"[WARN] ignoring error obtaining xargs: Error getting xattrs for `"/d/a"`: `Os { code: 95, kind: Unsupported, message: "Not supported" }`
{"message_type":"error","error":{"message":"Failed to open `/d/b`: `Os { code: 13, kind: PermissionDenied, message: \"Permission denied\" }`"},"during":"archival","item":"/d/b"}"#;
        let skipped = output("backup", stderr, false).skipped();
        assert_eq!(skipped, [("/d/b".to_owned(), "Permission denied".to_owned())]);
    }

    /// Rustic asks for a missing password, and without a terminal says only that there is none.
    #[test]
    fn a_missing_password_says_where_to_set_it() {
        let said = output("backup", "error: IO error: not a terminal\n", false).failure().to_string();
        assert!(said.contains("password-command") && !said.contains("not a terminal"), "{said}");
    }

    #[test]
    fn failures_say_what_rustic_said() {
        // The `[ERROR]` entry says why, where rustic's final error only says that the copy failed.
        for (_, stderr) in fixtures("copy-refused.stderr") {
            insta::assert_snapshot!("copy_refused", output("copy", &stderr, false).failure().to_string());
        }
        for (_, stderr) in fixtures("copy-unreachable.stderr") {
            let said = output("copy", &stderr, false).failure().to_string();
            let why = "error copying to target: Checking for the presence of `config` in the backend failed.";
            assert!(said.contains(why), "{said}");
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
