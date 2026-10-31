//! A command's own steps, outside collections: in a terminal, a run below `rusticlone: label`, each step
//! one line that says what it does while it spins, then what it did.

use std::iter;

use parking_lot::Mutex;

use super::{SPIN, branch};
use crate::{
    cli::tree::{Tree, tree},
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

/// Does this command, `label`, with `f`, drawn in a terminal with each [`step`] that `f` does. There, an error
/// is shown in the run instead of returned, as the command's failure.
#[expect(clippy::disallowed_methods, reason = "a spinner turns with time, which no event tells")]
pub(in crate::cli) fn command(label: &str, f: impl FnOnce() -> anyhow::Result<Level>) -> anyhow::Result<Level> {
    if !log::is_live() {
        return f();
    }
    *COMMAND.lock() = Some(Command { label: label.to_owned(), done: Vec::new(), step: None, failed: false });
    log::capture(Some(Hooks { logged, shown, ask }));
    let spinner = std::thread::spawn(|| {
        while COMMAND.lock().is_some() && !clock::interrupted() {
            clock::sleep(SPIN, &clock::interrupted);
            log::turn();
            draw();
        }
    });
    let result = f();
    log::capture(None);
    let Some(mut command) = COMMAND.lock().take() else { return result };
    _ = spinner.join();
    let level = match result {
        Ok(level) => level,
        Err(e) => {
            if !command.failed {
                command.done.push(branch(Level::Failure, &format!("{e:#}")));
            }
            Level::Failure
        }
    };
    log::runs(Vec::new(), &lines(&command.label, &command.done));
    Ok(level)
}

/// Does `f` as a step of this [`command`]: `now` while it runs. Once it ends, what it did: as it showed with
/// [`log::did`], else the first message it logged, else `did`.
pub(in crate::cli) fn step<T>(now: &str, did: &str, f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    if let Some(command) = COMMAND.lock().as_mut() {
        command.step = Some(Step { now: now.to_owned(), did: None, logged: Vec::new() });
    }
    draw();
    let result = f();
    if let Some(command) = COMMAND.lock().as_mut()
        && let Some(Step { now, did: shown, mut logged }) = command.step.take()
    {
        let ended = match (&result, shown) {
            (Err(e), _) => {
                command.failed = true;
                branch(Level::Failure, &format!("{now}: {e:#}"))
            }
            (Ok(_), Some(shown)) => branch(Level::Success, &shown),
            (Ok(_), None) if !logged.is_empty() => logged.remove(0),
            (Ok(_), None) => branch(Level::Success, did),
        };
        command.done.push(Tree { below: ended.below.into_iter().chain(logged).collect(), ..ended });
    }
    draw();
    result
}

/// Puts what this command logs in the step that runs, else in its run.
fn logged(level: Level, text: &str) {
    if let Some(command) = COMMAND.lock().as_mut() {
        let branch = branch(level, text);
        match command.step.as_mut() {
            Some(step) => step.logged.push(branch),
            None => command.done.push(branch),
        }
    }
    draw();
}

fn shown(shown: Shown<'_>) {
    if let Some(step) = COMMAND.lock().as_mut().and_then(|c| c.step.as_mut()) {
        match shown {
            Shown::Now(text) => text.clone_into(&mut step.now),
            Shown::Did(text) => step.did = Some(text.to_owned()),
        }
    }
    draw();
}

/// Asks `question` as the line of the step that runs, where the answer is typed.
fn ask(question: &str) -> std::io::Result<bool> {
    let rows = COMMAND.lock().as_ref().map(|command| {
        let mut branches = command.done.clone();
        branches.push(Tree::leaf(format!("{} {question} [y/N]", Tone::Running.dot())));
        lines(&command.label, &branches)
    });
    let answer = log::prompt(rows.unwrap_or_default())?;
    Ok(answer.trim().eq_ignore_ascii_case("y"))
}

fn draw() {
    let rows = COMMAND.lock().as_ref().map(|command| {
        let mut branches = command.done.clone();
        if let Some(step) = &command.step {
            let line = format!("{} {}", Tone::Running.dot(), step.now);
            branches.push(Tree { line, below: step.logged.clone() });
        }
        lines(&command.label, &branches)
    });
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
}
