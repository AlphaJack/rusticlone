//! A command's own steps, outside collections: in a terminal, a run below `rusticlone: label`, each step
//! one line that says what it does while it spins, then what it did.

use std::iter;

use parking_lot::Mutex;

use super::SPIN;
use crate::{
    cli::tree::{Tree, branch, tree},
    clock,
    log::{self, Hooks, Level, Shown, Tone},
};

struct Command {
    label: String,
    /// Ended steps, and what was logged outside them.
    done: Vec<Tree>,
    step: Option<Step>,
    /// A step failed, and shows the error.
    failed: bool,
}

/// The step that runs.
struct Step {
    now: String,
    did: Option<String>,
    /// What it logged.
    logged: Vec<Tree>,
}

static COMMAND: Mutex<Option<Command>> = Mutex::new(None);

impl Command {
    fn new(label: &str) -> Self {
        Self { label: label.to_owned(), done: Vec::new(), step: None, failed: false }
    }

    /// Starts a step that shows `now` while it runs.
    fn start(&mut self, now: &str) {
        self.step = Some(Step { now: now.to_owned(), did: None, logged: Vec::new() });
    }

    /// Ends the step that runs. It shows what it did as it showed, else the first message it logged, else `did`.
    /// A step that failed with `error` shows it.
    fn end(&mut self, error: Option<String>, did: &str) {
        let Some(Step { now, did: shown, mut logged }) = self.step.take() else { return };
        let ended = match (error, shown) {
            (Some(e), _) => {
                self.failed = true;
                branch(Level::Failure, &format!("{now}: {e}"))
            }
            (None, Some(shown)) => branch(Level::Success, &shown),
            (None, None) if !logged.is_empty() => logged.remove(0),
            (None, None) => branch(Level::Success, did),
        };
        self.done.push(Tree { below: ended.below.into_iter().chain(logged).collect(), ..ended });
    }

    /// Puts what this command logs in the step that runs, else in its run.
    fn log(&mut self, level: Level, text: &str) {
        let branch = branch(level, text);
        match self.step.as_mut() {
            Some(step) => step.logged.push(branch),
            None => self.done.push(branch),
        }
    }

    fn show(&mut self, shown: Shown<'_>) {
        if let Some(step) = self.step.as_mut() {
            match shown {
                Shown::Now(text) => text.clone_into(&mut step.now),
                Shown::Did(text) => step.did = Some(text.to_owned()),
            }
        }
    }

    /// Its rows: the ended steps, then the one that runs, with what it logged.
    fn rows(&self) -> Vec<String> {
        let mut branches = self.done.clone();
        if let Some(step) = &self.step {
            let line = format!("{} {}", Tone::Running.dot(), step.now);
            branches.push(Tree { line, below: step.logged.clone() });
        }
        lines(&self.label, &branches)
    }

    /// Its rows while `question` waits for its answer, as the line of the step that runs.
    fn asking(&self, question: &str) -> Vec<String> {
        let mut branches = self.done.clone();
        branches.push(Tree::leaf(format!("{} {question} [y/N]", Tone::Running.dot())));
        lines(&self.label, &branches)
    }

    /// The command's level once it returned `result`. An error that no step showed is shown last.
    fn finish(&mut self, result: anyhow::Result<Level>) -> Level {
        match result {
            Ok(level) => level,
            Err(e) => {
                if !self.failed {
                    self.done.push(branch(Level::Failure, &format!("{e:#}")));
                }
                Level::Failure
            }
        }
    }
}

/// Does this command, `label`, with `f`, drawn in a terminal with each [`step`] that `f` does. There, an error
/// is shown in the run instead of returned, as the command's failure.
pub(in crate::cli) fn command(label: &str, f: impl FnOnce() -> anyhow::Result<Level>) -> anyhow::Result<Level> {
    if !log::is_live() {
        return f();
    }
    live(label, f)
}

/// [`command`] in a terminal.
#[cfg_attr(test, mutants::skip)] // Draws in a terminal, which tests have none of.
#[expect(clippy::disallowed_methods, reason = "a spinner turns with time, which no event tells")]
fn live(label: &str, f: impl FnOnce() -> anyhow::Result<Level>) -> anyhow::Result<Level> {
    *COMMAND.lock() = Some(Command::new(label));
    log::capture(Some(Hooks { logged, shown, ask }));
    let spinner = std::thread::spawn(|| {
        while COMMAND.lock().is_some() && !clock::interrupted() {
            clock::WAKES.sleep(SPIN, &clock::interrupted);
            log::turn();
            draw();
        }
    });
    let result = f();
    log::capture(None);
    let Some(mut command) = COMMAND.lock().take() else { return result };
    _ = spinner.join();
    let level = command.finish(result);
    log::runs(Vec::new(), &lines(&command.label, &command.done));
    Ok(level)
}

/// Does `f` as a step of this [`command`]: `now` while it runs. Once it ends, what it did: as it showed with
/// [`log::did`], else the first message it logged, else `did`.
pub(in crate::cli) fn step<T>(now: &str, did: &str, f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    with(|command| command.start(now));
    let result = f();
    with(|command| command.end(result.as_ref().err().map(|e| format!("{e:#}")), did));
    result
}

/// Changes the command that runs in a terminal, and draws it again.
#[cfg_attr(test, mutants::skip)] // Only a command in a terminal is changed, and tests have none.
fn with(f: impl FnOnce(&mut Command)) {
    if let Some(command) = COMMAND.lock().as_mut() {
        f(command);
    }
    draw();
}

#[cfg_attr(test, mutants::skip)] // Only a command in a terminal logs here, and tests have none.
fn logged(level: Level, text: &str) {
    with(|command| command.log(level, text));
}

#[cfg_attr(test, mutants::skip)] // Only a command in a terminal shows here, and tests have none.
fn shown(shown: Shown<'_>) {
    with(|command| command.show(shown));
}

/// Asks `question` as the line of the step that runs, where the answer is typed.
#[cfg_attr(test, mutants::skip)] // Waits for an answer in a terminal, which tests have none of.
fn ask(question: &str) -> std::io::Result<bool> {
    let rows = COMMAND.lock().as_ref().map(|command| command.asking(question));
    let answer = log::prompt(rows.unwrap_or_default())?;
    Ok(answer.trim().eq_ignore_ascii_case("y"))
}

#[cfg_attr(test, mutants::skip)] // Draws in a terminal, which tests have none of.
fn draw() {
    let rows = COMMAND.lock().as_ref().map(Command::rows);
    if let Some(rows) = rows {
        log::runs(rows, &[]);
    }
}

/// `rusticlone: label`, then `branches`.
fn lines(label: &str, branches: &[Tree]) -> Vec<String> {
    let head = format!("{}: {label}", log::bold(crate::NAME));
    iter::once(head).chain(tree(branches).lines().map(ToOwned::to_owned)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_drawn_below_its_name() {
        let drawn = lines("update", &[branch(Level::Success, "updated"), branch(Level::Failure, "rustic: offline")]);
        assert_eq!(drawn, [format!("{}: update", crate::NAME), "├── ● updated".into(), "└── ● rustic: offline".into()]);
    }

    #[test]
    fn without_a_terminal_a_command_only_runs() {
        assert_eq!(command("update", || Ok(Level::Warning)).unwrap(), Level::Warning);
        assert!(command("update", || Err(std::io::Error::other("offline").into())).is_err());
        assert_eq!(step("checking", "checked", || Ok(1)).unwrap(), 1);
    }

    /// An ended step shows what it showed it did, else the first line it logged, else what it did. The other lines
    /// it logged go below it, and lines logged outside steps go in the run.
    #[test]
    fn an_ended_step_shows_what_it_did() {
        let mut command = Command::new("install");
        command.start("placing");
        command.show(Shown::Did("placed in bin"));
        command.end(None, "placed");
        command.start("writing");
        command.log(Level::Warning, "kept the unit");
        command.log(Level::Success, "linger on");
        command.end(None, "written");
        command.start("starting");
        command.end(None, "started");
        command.log(Level::Success, "outside");
        let rows =
            ["├── ● placed in bin", "├──┐● kept the unit", "│  └── ● linger on", "├── ● started", "└── ● outside"];
        assert_eq!(
            command.rows(),
            iter::once(format!("{}: install", crate::NAME)).chain(rows.map(String::from)).collect::<Vec<_>>()
        );
    }

    /// The step that runs is its last row, with what it does now. A question takes its place while it waits.
    #[test]
    fn the_running_step_is_the_last_row() {
        let mut command = Command::new("update");
        command.start("checking");
        command.log(Level::Success, "found 2.1");
        #[cfg(feature = "self-update")]
        command.show(Shown::Now("downloading"));
        let now = if cfg!(feature = "self-update") { "downloading" } else { "checking" };
        let rows = command.rows();
        assert!(rows[1].ends_with(&format!(" {now}")), "{rows:?}");
        assert_eq!(rows[2..], ["   └── ● found 2.1".to_owned()]);
        assert!(command.asking("Keep them?")[1].ends_with(" Keep them? [y/N]"));
    }

    /// A failed step shows its error, and the command fails without showing it again. An error outside steps is shown
    /// last.
    #[test]
    fn a_failure_shows_once() {
        let head = format!("{}: update", crate::NAME);
        let mut command = Command::new("update");
        command.start("checking");
        command.end(Some("offline".into()), "checked");
        assert_eq!(command.finish(Err(std::io::Error::other("offline").into())), Level::Failure);
        assert_eq!(command.rows(), [head.clone(), "└── ● checking: offline".into()]);
        let mut command = Command::new("update");
        assert_eq!(command.finish(Err(std::io::Error::other("offline").into())), Level::Failure);
        assert_eq!(command.rows(), [head, "└── ● offline".into()]);
        assert_eq!(Command::new("update").finish(Ok(Level::Warning)), Level::Warning);
    }
}
