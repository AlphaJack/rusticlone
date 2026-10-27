//! Our messages go to stdout, rustic's own log to stderr. Text: `collection: message`. JSON: one object
//! per line, with `target` set to our name, `progress` or `rustic`. In a terminal, progress rewrites
//! its line, and rustic's own log is left out: the results already say what went wrong. Text outside a
//! terminal has no progress.

#![expect(clippy::disallowed_methods, reason = "timestamps log lines, and reads NO_COLOR")]

use std::{
    io::{IsTerminal, Write, stderr, stdout},
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
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
            Level::Success => "🟢",
            Level::Warning => "🟡",
            Level::Failure => "🔴",
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

impl Block {
    /// Erases it, prints `lines` above it, and draws it again.
    fn redraw(&mut self, lines: &[String]) {
        let mut out = stdout().lock();
        let rows: Vec<&String> = self.runs.iter().chain(&self.status).collect();
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
pub(crate) fn runs(runs: Vec<String>, ended: &[String]) {
    let mut block = BLOCK.lock();
    if block.runs != runs || !ended.is_empty() {
        block.runs = runs;
        block.redraw(ended);
    }
}

/// Erases the status line, so that what is printed next follows the printed lines.
pub(crate) fn clear() {
    let mut block = BLOCK.lock();
    if block.status.take().is_some() {
        block.redraw(&[]);
    }
}

pub(crate) fn init(json: bool, verbose: bool) {
    let live = !json && stdout().is_terminal() && crate::sys::ansi();
    let color = live && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    _ = MODE.set(Mode { json, live, quiet: !json && !verbose, color });
}

/// `text` in the color of `level`, where colors are shown.
pub(crate) fn paint(level: Level, text: &str) -> String {
    style(Tone::from(level).code(), text)
}

/// `text` in bold, where colors are shown.
pub(crate) fn bold(text: &str) -> String {
    style("1", text)
}

fn style(code: &str, text: &str) -> String {
    if mode().color { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_owned() }
}

fn mode() -> Mode {
    MODE.get().copied().unwrap_or_default()
}

pub(crate) fn is_json() -> bool {
    mode().json
}

/// Text on a terminal, redrawn in place.
pub(crate) fn is_live() -> bool {
    mode().live
}

/// `name`: the collection, or for rustic's own lines, the profile. `extra` fields appear in JSON only.
fn write(target: &str, level: Level, name: Option<&str>, message: &str, extra: Value) {
    let mode = mode();
    // Progress is a live line in a terminal, and JSON elsewhere: as text lines, it would flood logs.
    if (target == "rustic" && mode.quiet) || (target == "progress" && !mode.json && !mode.live) {
        return;
    }
    let line = if mode.json {
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
        v.to_string()
    } else {
        let message = if level == Level::Success { message.to_owned() } else { paint(level, message) };
        match name {
            Some(profile) if target == "rustic" => format!("[{profile}] {message}"),
            Some(collection) => format!("{collection}: {message}"),
            None => message,
        }
    };
    // A closed pipe must not stop a backup.
    if target == "rustic" {
        _ = writeln!(stderr().lock(), "{line}");
    } else if !mode.live {
        _ = writeln!(stdout().lock(), "{line}");
    } else if target == "progress" {
        let mut block = BLOCK.lock();
        block.status = Some(line);
        block.redraw(&[]);
    } else {
        BLOCK.lock().redraw(&[line]);
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
        examples: String,
    },
    NotCopied {
        error: String,
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
            | Self::NoProfiles { .. } => Level::Warning,
            #[cfg(feature = "server")]
            Self::NotScheduled { .. } => Level::Warning,
            Self::NotCopied { .. } => Level::Success,
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
}
