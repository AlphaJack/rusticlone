//! Our messages go to stdout, rustic's own log to stderr. Text: `collection: message`. JSON: one object
//! per line, with `target` set to our name, `progress` or `rustic`. In a terminal, progress rewrites
//! its line, and rustic's own log is left out: the results already say what went wrong. Text outside a
//! terminal has no progress.

use std::{
    io::{IsTerminal, Write, stderr, stdout},
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use jiff::Timestamp;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Value, json};

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    strum::IntoStaticStr,
    strum::EnumString,
    strum::Display,
    strum::EnumIter,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum Level {
    #[default]
    Success,
    Warning,
    Failure,
}

impl Level {
    /// A collection's health: `degraded`.
    pub fn word(self) -> String {
        crate::i18n::t!("level-word", level = self)
    }

    /// Its colored circle, for text without styles.
    pub fn emoji(self) -> &'static str {
        match self {
            Self::Success => "🟢",
            Self::Warning => "🟡",
            Self::Failure => "🔴",
        }
    }

    pub fn dot(self) -> String {
        Tone::from(self).dot()
    }
}

/// A status's color, as the web UI and the CLI show it: a level's, muted without an outcome, or in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
pub enum Tone {
    Success,
    Warning,
    Failure,
    Muted,
    Running,
}

impl From<Level> for Tone {
    fn from(level: Level) -> Self {
        match level {
            Level::Success => Self::Success,
            Level::Warning => Self::Warning,
            Level::Failure => Self::Failure,
        }
    }
}

impl Tone {
    /// Its SGR attribute.
    fn code(self) -> &'static str {
        match self {
            Self::Success => "32",
            Self::Warning => "33",
            Self::Failure => "31",
            Self::Muted | Self::Running => "2",
        }
    }

    /// Its circle, painted. Running is a half circle that [`turn`] turns, as the web UI's spinner.
    pub fn dot(self) -> String {
        let dot = match self {
            Self::Running => SPINNER[FRAME.load(Ordering::Relaxed) % SPINNER.len()],
            Self::Success | Self::Warning | Self::Failure | Self::Muted => "●",
        };
        style(self.code(), dot)
    }
}

const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];
static FRAME: AtomicUsize = AtomicUsize::new(0);

/// Turns the spinners to their next frame.
pub(crate) fn turn() {
    FRAME.fetch_add(1, Ordering::Relaxed);
}

/// Text for users. `Display` writes it in English; its fields stay available for JSON.
pub(crate) trait Message: std::fmt::Display {
    fn level(&self) -> Level;
}

/// [`Message`] for types whose every message has one level: `log::level!(Warning: Blocked, Untested);`.
macro_rules! level {
    ($level:ident: $($ty:ty),+) => {
        $(impl $crate::log::Message for $ty {
            fn level(&self) -> $crate::log::Level {
                $crate::log::Level::$level
            }
        })+
    };
}
pub(crate) use level;

/// A message in JSON: its kind and fields, its level and its text. Scripts match the kind.
#[derive(Serialize)]
struct JsonMessage<'a, M> {
    #[serde(flatten)]
    message: &'a M,
    level: Level,
    text: String,
}

impl<'a, M: Message> JsonMessage<'a, M> {
    fn new(message: &'a M) -> Self {
        Self { message, level: message.level(), text: message.to_string() }
    }
}

/// For `#[serde(serialize_with)]`: messages with their level and text.
pub(crate) fn serialize_messages<M: Message + Serialize, S: Serializer>(
    messages: &[M],
    s: S,
) -> Result<S::Ok, S::Error> {
    s.collect_seq(messages.iter().map(JsonMessage::new))
}

/// For `#[serde(serialize_with)]`: an optional message with its level and text.
#[expect(clippy::ref_option, reason = "serde's serialize_with signature")]
pub(crate) fn serialize_message<M: Message + Serialize, S: Serializer>(
    message: &Option<M>,
    s: S,
) -> Result<S::Ok, S::Error> {
    message.as_ref().map(JsonMessage::new).serialize(s)
}

/// How lines are written, chosen at start.
#[derive(Clone, Copy, Default)]
struct Mode {
    json: bool,
    /// Text on a terminal: progress rewrites its line.
    live: bool,
    /// Text without `--verbose`: rustic's own log is left out, since each run keeps it and rusticlone's
    /// lines quote its causes.
    quiet: bool,
    /// Text on a terminal, without `NO_COLOR` set.
    color: bool,
}

static MODE: OnceLock<Mode> = OnceLock::new();

/// What a terminal shows below the printed lines, redrawn in place: the runs of this process, then
/// what is done outside runs.
struct Block {
    runs: Vec<String>,
    status: Option<String>,
    /// Rows on screen.
    drawn: usize,
}

static BLOCK: Mutex<Block> = Mutex::new(Block { runs: Vec::new(), status: None, drawn: 0 });

/// While set, the runs are not drawn: a question waits for its answer below the printed lines.
static HELD: AtomicBool = AtomicBool::new(false);

/// What a command draws in a terminal instead of lines above the runs.
#[cfg(any(feature = "desktop", feature = "self-update"))]
#[derive(Clone, Copy)]
pub(crate) struct Hooks {
    /// Takes what this process logs.
    pub logged: fn(Level, &str),
    /// Takes what the step that runs shows.
    pub shown: fn(Shown<'_>),
    /// Asks a question, in the step that runs.
    pub ask: fn(&str) -> std::io::Result<bool>,
}

/// What a command's step that runs shows as its line.
#[cfg(any(feature = "desktop", feature = "self-update"))]
#[derive(Clone, Copy)]
pub(crate) enum Shown<'a> {
    /// What it does now.
    #[cfg_attr(not(feature = "self-update"), expect(dead_code, reason = "only a download says what it does now"))]
    Now(&'a str),
    /// What it did, once it ends.
    Did(&'a str),
}

#[cfg(any(feature = "desktop", feature = "self-update"))]
static HOOKS: Mutex<Option<Hooks>> = Mutex::new(None);

impl Block {
    /// Erases it, prints `lines` above it, and draws it again.
    #[cfg_attr(test, mutants::skip)] // Writes to the terminal, which tests have none of.
    fn redraw(&mut self, lines: &[String]) {
        let mut out = stdout().lock();
        let held = HELD.load(Ordering::Relaxed);
        let rows: Vec<&String> = self.runs.iter().chain(&self.status).filter(|_| !held).collect();
        // Up to its first row, erasing to the end of the screen. Unwrapped, each line takes one row. A terminal
        // that synchronizes output shows the frame once drawn, never erased and half redrawn.
        let erase = if self.drawn > 0 { format!("\x1b[{}A\x1b[J", self.drawn) } else { String::new() };
        let mut text = "\x1b[?2026h".to_owned() + &erase;
        text += &lines.iter().map(|l| format!("{l}\n")).collect::<String>();
        text += "\x1b[?7l";
        text.extend(rows.iter().map(|r| format!("{r}\n")));
        text += "\x1b[?7h\x1b[?2026l";
        self.drawn = rows.len();
        // A closed pipe must not stop a backup.
        _ = out.write_all(text.as_bytes()).and_then(|()| out.flush());
    }
}

/// Shows the runs of this process as `runs`, below the printed lines, and prints `ended` above them.
#[cfg_attr(test, mutants::skip)] // Draws in a terminal, which tests have none of.
pub(crate) fn runs(runs: Vec<String>, ended: &[String]) {
    let mut block = BLOCK.lock();
    if redraws(&block.runs, &runs, ended) {
        block.runs = runs;
        block.redraw(ended);
    }
}

/// Whether the runs drawn as `shown` are drawn again: they changed, or lines are printed above them.
fn redraws(shown: &[String], runs: &[String], ended: &[String]) -> bool {
    shown != runs || !ended.is_empty()
}

/// Runs `f` with the runs erased and not redrawn, as while a question waits for its answer.
pub(crate) fn hold<T>(f: impl FnOnce() -> T) -> T {
    HELD.store(true, Ordering::Relaxed);
    BLOCK.lock().redraw(&[]);
    let out = f();
    HELD.store(false, Ordering::Relaxed);
    BLOCK.lock().redraw(&[]);
    out
}

/// Draws what this process logs in a terminal with `hooks`, as a command does in its tree, or, without, above the
/// runs again.
#[cfg(any(feature = "desktop", feature = "self-update"))]
#[cfg_attr(test, mutants::skip)] // Sets the process's hooks, which every other test would then share.
pub(crate) fn capture(hooks: Option<Hooks>) {
    *HOOKS.lock() = hooks;
}

/// What the step of a command that runs does now. Nothing outside one.
#[cfg(feature = "self-update")]
#[cfg_attr(test, mutants::skip)] // Shown only in a command's tree in a terminal, which tests have none of.
pub(crate) fn now(text: &str) {
    let hooks = *HOOKS.lock();
    if let Some(hooks) = hooks {
        (hooks.shown)(Shown::Now(text));
    }
}

/// What the step of a command that runs did, its line once it ends. Nothing outside one.
#[cfg(any(feature = "desktop", feature = "self-update"))]
#[cfg_attr(test, mutants::skip)] // Shown only in a command's tree in a terminal, which tests have none of.
pub(crate) fn did(text: &str) {
    let hooks = *HOOKS.lock();
    if let Some(hooks) = hooks {
        (hooks.shown)(Shown::Did(text));
    }
}

/// Asks `question` in a terminal, and whether the answer is yes. False without a terminal to ask in.
#[cfg_attr(test, mutants::skip)] // Waits for an answer in a terminal, which tests have none of.
pub(crate) fn confirm(question: &str) -> std::io::Result<bool> {
    ask(question, question)
}

/// [`confirm`], as `short` in the step of a command that runs.
#[cfg_attr(test, mutants::skip)] // Waits for an answer in a terminal, which tests have none of.
pub(crate) fn ask(question: &str, short: &str) -> std::io::Result<bool> {
    if !std::io::stdin().is_terminal() || !stderr().is_terminal() {
        return Ok(false);
    }
    #[cfg(any(feature = "desktop", feature = "self-update"))]
    {
        // Not locked while it waits for the answer.
        let hooks = *HOOKS.lock();
        if let Some(hooks) = hooks {
            return (hooks.ask)(short);
        }
    }
    #[cfg(not(any(feature = "desktop", feature = "self-update")))]
    let _ = short;
    hold(|| {
        eprint!("{question} [y/N] ");
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        Ok(answer.trim().eq_ignore_ascii_case("y"))
    })
}

/// Shows `rows` as the runs, the last one open for the answer, and reads it. Nothing is redrawn meanwhile.
#[cfg(any(feature = "desktop", feature = "self-update"))]
#[cfg_attr(test, mutants::skip)] // Waits for an answer in a terminal, which tests have none of.
pub(crate) fn prompt(rows: Vec<String>) -> std::io::Result<String> {
    let mut block = BLOCK.lock();
    let erase = if block.drawn > 0 { format!("\x1b[{}A\x1b[J", block.drawn) } else { String::new() };
    let mut out = stdout().lock();
    out.write_all(format!("{erase}{} ", rows.join("\n")).as_bytes())?;
    out.flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    // The answer ended the last row.
    block.drawn = rows.len();
    block.runs = rows;
    Ok(answer)
}

/// Erases the status line, so that what is printed next follows the printed lines.
#[cfg_attr(test, mutants::skip)] // Writes to the terminal, which tests have none of.
pub(crate) fn clear() {
    let mut block = BLOCK.lock();
    if block.status.take().is_some() {
        block.redraw(&[]);
    }
}

#[cfg_attr(test, mutants::skip)] // Sets the process's output mode once, which every other test would then share.
#[expect(clippy::disallowed_methods, reason = "timestamps log lines, and reads NO_COLOR")]
pub(crate) fn init(json: bool, verbose: bool) {
    let live = !json && stdout().is_terminal() && crate::sys::ansi();
    let color = live && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    _ = MODE.set(Mode { json, live, quiet: !json && !verbose, color });
}

/// `text` in the color of `level`, where colors are shown.
pub(crate) fn paint(level: Level, text: &str) -> String {
    painted(mode().color, level, text)
}

/// `text` in the color of `level` if `color`. Success keeps the terminal's own color.
fn painted(color: bool, level: Level, text: &str) -> String {
    if level == Level::Success {
        return text.to_owned();
    }
    styled(color, Tone::from(level).code(), text)
}

/// `text` in bold, where colors are shown.
pub(crate) fn bold(text: &str) -> String {
    style("1", text)
}

fn style(code: &str, text: &str) -> String {
    styled(mode().color, code, text)
}

fn styled(color: bool, code: &str, text: &str) -> String {
    if color { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_owned() }
}

fn mode() -> Mode {
    MODE.get().copied().unwrap_or_default()
}

pub(crate) fn is_json() -> bool {
    mode().json
}

/// Text on a terminal, redrawn in place.
#[cfg_attr(test, mutants::skip)] // True only on a terminal, which tests have none of.
pub(crate) fn is_live() -> bool {
    mode().live
}

/// Where a line goes.
#[derive(Debug, PartialEq)]
enum Out {
    /// Rustic's own log.
    Stderr,
    /// A line of its own, outside a terminal.
    Stdout,
    /// The status line below the runs.
    Status,
    /// What this process logs, in the tree of a command that draws one, else above the runs.
    Tree,
    /// Above the runs.
    Above,
}

/// Where a line of `target` goes, or nowhere. Progress is a live line in a terminal, and JSON elsewhere: as text
/// lines, it would flood logs.
fn route(mode: Mode, target: &str) -> Option<Out> {
    if (target == "rustic" && mode.quiet) || (target == "progress" && !mode.json && !mode.live) {
        return None;
    }
    Some(if target == "rustic" {
        Out::Stderr
    } else if !mode.live {
        Out::Stdout
    } else if target == "progress" {
        Out::Status
    } else if target == crate::NAME {
        Out::Tree
    } else {
        Out::Above
    })
}

/// The line that `mode` writes. `name`: the collection, or for rustic's own lines, the profile. `extra` fields
/// appear in JSON only.
#[expect(clippy::disallowed_methods, reason = "timestamps log lines, and reads NO_COLOR")]
fn line(mode: Mode, target: &str, level: Level, name: Option<&str>, message: &str, extra: Value) -> String {
    if mode.json {
        // The level names log processors expect.
        let level = match level {
            Level::Success => "INFO",
            Level::Warning => "WARN",
            Level::Failure => "ERROR",
        };
        let mut v = json!({"timestamp": Timestamp::now(), "level": level, "target": target, "message": message});
        if let Some(name) = name {
            v[if target == "rustic" { "profile" } else { "collection" }] = name.into();
        }
        if let (Value::Object(v), Value::Object(extra)) = (&mut v, extra) {
            v.extend(extra);
        }
        return v.to_string();
    }
    // A line of its own starts with a capital.
    let message = if name.is_none() { crate::fmt::capitalized(message) } else { message.to_owned() };
    let message = painted(mode.color, level, &message);
    match name {
        Some(profile) if target == "rustic" => format!("[{profile}] {message}"),
        Some(collection) => format!("{collection}: {message}"),
        None => message,
    }
}

/// Hands a line that goes to a command's tree to that command, if one draws it. Returns whether it did.
#[cfg_attr(test, mutants::skip)] // Only a command in a terminal draws a tree, and tests have none.
fn in_tree(out: &Out, level: Level, message: &str) -> bool {
    #[cfg(any(feature = "desktop", feature = "self-update"))]
    if *out == Out::Tree
        && let Some(hooks) = *HOOKS.lock()
    {
        (hooks.logged)(level, message);
        return true;
    }
    #[cfg(not(any(feature = "desktop", feature = "self-update")))]
    let _ = (out, level, message);
    false
}

fn write(target: &str, level: Level, name: Option<&str>, message: &str, extra: Value) {
    let mode = mode();
    let Some(out) = route(mode, target) else { return };
    if in_tree(&out, level, message) {
        return;
    }
    let line = line(mode, target, level, name, message, extra);
    // A closed pipe must not stop a backup.
    match out {
        Out::Stderr => _ = writeln!(stderr().lock(), "{line}"),
        Out::Stdout => _ = writeln!(stdout().lock(), "{line}"),
        Out::Status => {
            let mut block = BLOCK.lock();
            block.status = Some(line);
            block.redraw(&[]);
        }
        Out::Tree | Out::Above => BLOCK.lock().redraw(&[line]),
    }
}

/// In JSON, the message's kind and fields are its `event`.
pub(crate) fn event(collection: Option<&str>, message: &(impl Message + Serialize)) {
    let event = json!({ "event": serde_json::to_value(message).unwrap_or_default() });
    write(crate::NAME, message.level(), collection, &message.to_string(), event);
}

pub(crate) fn rustic(profile: &str, line: &str) {
    write("rustic", Level::Success, Some(profile), line, Value::Null);
}

/// What is being done, shown like progress, without a percentage.
pub(crate) fn doing(collection: &str, doing: &(impl Message + Serialize)) {
    let event = json!({ "event": serde_json::to_value(doing).unwrap_or_default() });
    write("progress", doing.level(), Some(collection), &doing.to_string(), event);
}

/// A step's progress. A terminal shows it with the run's steps instead.
pub(crate) fn progress(collection: &str, doing: &str, summary: &str, percent_done: f64) {
    if mode().live {
        return;
    }
    let message = format!("{doing} {summary}");
    let extra = json!({"doing": doing, "percent_done": percent_done});
    write("progress", Level::Success, Some(collection), &message, extra);
}

/// Events logged outside run reports. `error` fields hold the error's text.
#[derive(Debug, Serialize, crate::i18n::Fluent)]
#[fluent(prefix = "log")]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum Logged {
    Error {
        error: String,
    },
    /// The state database could not be read and was moved aside.
    StateAfresh {
        path: String,
        error: String,
    },
    StateUnread {
        error: String,
    },
    StateNotKept {
        error: String,
    },
    ListingNotKept {
        repo: String,
        error: String,
    },
    RunNotSaved {
        error: String,
    },
    NotMeasured {
        repo: String,
        error: String,
    },
    #[cfg(feature = "server")]
    NotScheduled {
        error: String,
    },
    NoProfiles {
        dirs: String,
    },
    NotCopied {
        error: String,
    },
    /// `--kit`: how many of its profiles were saved and where.
    KitUnpacked {
        saved: u64,
        dir: String,
    },
    /// After a restore, profiles whose folders are not on this computer.
    ProfilesElsewhere {
        files: String,
    },
}

impl Message for Logged {
    fn level(&self) -> Level {
        match self {
            Self::Error { .. } => Level::Failure,
            Self::StateAfresh { .. }
            | Self::StateUnread { .. }
            | Self::StateNotKept { .. }
            | Self::ListingNotKept { .. }
            | Self::RunNotSaved { .. }
            | Self::NotMeasured { .. }
            | Self::NoProfiles { .. }
            | Self::ProfilesElsewhere { .. } => Level::Warning,
            #[cfg(feature = "server")]
            Self::NotScheduled { .. } => Level::Warning,
            Self::NotCopied { .. } | Self::KitUnpacked { .. } => Level::Success,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::StepResult;

    /// Reading a stored line back ignores the extra JSON fields.
    #[test]
    fn stored_line_reads_back() {
        for line in [StepResult::RunningAlready, StepResult::InSync { target: "Docs-usb".into() }] {
            let json = serde_json::to_string(&JsonMessage::new(&line)).unwrap();
            assert_eq!(serde_json::from_str::<StepResult>(&json).unwrap(), line, "{json}");
        }
    }

    /// Each level has its own color, and the spinner turns a frame at a time. Colors stay off, and the frame is
    /// put back, for the other tests.
    #[test]
    fn levels_have_their_colors_and_the_spinner_turns() {
        let codes = [Level::Success, Level::Warning, Level::Failure].map(|level| Tone::from(level).code());
        assert_eq!(codes, ["32", "33", "31"]);
        assert_eq!(Tone::Muted.code(), "2");
        let start = FRAME.load(Ordering::Relaxed);
        FRAME.store(6, Ordering::Relaxed);
        assert_eq!(Tone::Running.dot(), SPINNER[2]);
        turn();
        assert_eq!(Tone::Running.dot(), SPINNER[3]);
        FRAME.store(start, Ordering::Relaxed);
    }

    /// Rustic's log goes to stderr and the rest to stdout. In a terminal, progress is the status line, and
    /// this process's lines go to a command's tree. Text leaves out rustic's log without `--verbose`, and progress
    /// outside a terminal.
    #[test]
    fn each_line_goes_where_its_mode_shows_it() {
        let quiet = Mode { quiet: true, ..Mode::default() };
        let (json, live) = (Mode { json: true, ..Mode::default() }, Mode { live: true, ..quiet });
        assert_eq!(route(quiet, "rustic"), None);
        assert_eq!(route(Mode::default(), "rustic"), Some(Out::Stderr));
        assert_eq!(route(quiet, "progress"), None);
        assert_eq!(route(json, "progress"), Some(Out::Stdout));
        assert_eq!(route(quiet, crate::NAME), Some(Out::Stdout));
        assert_eq!(route(live, "progress"), Some(Out::Status));
        assert_eq!(route(live, crate::NAME), Some(Out::Tree));
        assert_eq!(route(live, "other"), Some(Out::Above));
    }

    /// Text reads `collection: message`, `[profile] message` for rustic's, or capitalized on its own, in the
    /// level's color where colors show. JSON has the level that log processors expect, and the extra fields.
    #[test]
    fn a_line_reads_as_its_mode_writes_it() {
        let color = Mode { color: true, ..Mode::default() };
        let text = |mode, target, level, name, message| line(mode, target, level, name, message, Value::Null);
        assert_eq!(text(Mode::default(), "rustic", Level::Success, Some("Docs"), "read"), "[Docs] read");
        assert_eq!(text(color, crate::NAME, Level::Success, Some("Docs"), "backed up"), "Docs: backed up");
        assert_eq!(text(color, crate::NAME, Level::Failure, None, "failed"), "\x1b[31mFailed\x1b[0m");
        let json = Mode { json: true, ..Mode::default() };
        let written = line(json, "rustic", Level::Warning, Some("Docs"), "read", json!({ "doing": "x" }));
        let written: Value = serde_json::from_str(&written).unwrap();
        let fields = ["level", "target", "profile", "message", "doing"].map(|k| written[k].clone());
        assert_eq!(fields, [json!("WARN"), json!("rustic"), json!("Docs"), json!("read"), json!("x")]);
    }

    /// The runs are drawn again when they change, or when lines are printed above them.
    #[test]
    fn the_runs_are_drawn_again_when_they_change() {
        let (a, b) = (vec!["a".to_owned()], vec!["b".to_owned()]);
        assert!(!redraws(&a, &a, &[]));
        assert!(redraws(&a, &b, &[]));
        assert!(redraws(&a, &a, &b));
    }

    /// An error outside runs is a failure. What is not kept is a warning. A copy not made is only said.
    #[test]
    fn logged_events_have_their_levels() {
        let levels = [
            Logged::Error { error: String::new() },
            Logged::NoProfiles { dirs: String::new() },
            Logged::NotCopied { error: String::new() },
        ]
        .map(|logged| logged.level());
        assert_eq!(levels, [Level::Failure, Level::Warning, Level::Success]);
    }
}
