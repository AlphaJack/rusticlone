//! What Linux's and macOS's service managers share: they run as commands, with this process's `PATH`.

use anyhow::Result;

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "autostart-error")]
pub(crate) enum Error {
    CannotRun { program: String, source: std::io::Error },
    Failed { command: String, error: String },
}

/// Runs a service manager's command, failing with its output.
pub(super) fn run(program: &str, args: &[&str]) -> Result<()> {
    let out = (std::process::Command::new(program).args(args).output())
        .map_err(|source| Error::CannotRun { program: program.to_owned(), source })?;
    if !out.status.success() {
        let (command, output) = (format!("{program} {}", args.join(" ")), String::from_utf8_lossy(&out.stderr));
        return Err(Error::Failed { command, error: output.trim().to_owned() }.into());
    }
    Ok(())
}

pub(super) fn path_var() -> String {
    std::env::var("PATH").unwrap_or_default()
}
