//! Rustic for tests: recorded answers instead of a process.

use std::{collections::VecDeque, sync::Arc};

use anyhow::Result;
use parking_lot::Mutex;

use super::{Ctx, Error, Invocation, Output, Progress, Runner, Stdout, output::Log};

/// Each call takes the first answer for its profile whose arguments are a prefix of its own. Calls
/// are recorded as `-P profile args`.
#[derive(Default)]
pub(crate) struct Replay {
    answers: Mutex<VecDeque<Answer>>,
    calls: Mutex<Vec<String>>,
}

struct Answer {
    profile: String,
    args: Vec<String>,
    stdout: String,
}

impl Replay {
    pub(crate) fn new() -> Arc<Self> {
        Arc::default()
    }

    /// `rustic -P profile args...` succeeds and prints `stdout`.
    pub(crate) fn ok(self: &Arc<Self>, profile: &str, args: &[&str], stdout: &str) -> Arc<Self> {
        let args = args.iter().map(|a| (*a).to_owned()).collect();
        self.answers.lock().push_back(Answer { profile: profile.into(), args, stdout: stdout.into() });
        self.clone()
    }

    /// The calls, in order.
    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls.lock().clone()
    }

    /// Answers that no call used.
    pub(crate) fn unused(&self) -> Vec<String> {
        let answers = self.answers.lock();
        answers.iter().map(|a| format!("-P {} {}", a.profile, a.args.join(" "))).collect()
    }
}

impl Runner for Replay {
    fn run(&self, _: &Ctx, call: &Invocation<'_>, _: Option<&mut dyn FnMut(&Progress)>) -> Result<(Output, u64)> {
        let line = format!("-P {} {}", call.profile, call.args.join(" "));
        self.calls.lock().push(line.clone());
        let mut answers = self.answers.lock();
        let fits = |a: &Answer| a.profile == call.profile && call.args.iter().zip(&a.args).all(|(c, a)| c == a);
        let Some(answer) = answers.iter().position(fits).and_then(|i| answers.remove(i)) else {
            return Err(Error::NoAnswer { command: line }.into());
        };
        let bytes = if call.stdout == Stdout::Count { answer.stdout.len() as u64 } else { 0 };
        let output = Output {
            success: true,
            stdout: if call.stdout == Stdout::Count { String::new() } else { answer.stdout },
            command: call.args.first().copied().unwrap_or_default().to_owned(),
            code: Some(0),
            stderr: Log::default(),
        };
        Ok((output, bytes))
    }

    #[cfg(feature = "server")]
    #[expect(clippy::disallowed_methods, reason = "a placeholder process for downloads")]
    fn command(&self, _: &str, dir: &std::path::Path) -> std::process::Command {
        let mut cmd = std::process::Command::new("false");
        cmd.current_dir(dir);
        cmd
    }
}
