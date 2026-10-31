//! The command line's grammar: what can be typed, and how a page spells a command line.

use std::path::PathBuf;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use clap_complete::{ArgValueCompleter, CompletionCandidate};

use crate::{
    engine::{self, Named},
    i18n::{self, t},
    rustic,
};

/// The flag of `install` that turns auto-update on.
#[cfg(all(feature = "desktop", feature = "self-update"))]
const AUTO_UPDATE: &str = "auto-update";

#[derive(Parser)]
#[command(version, about)]
pub(crate) struct Cli {
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(short, long, global = true)]
    pub verbose: bool,
    /// First save the profiles of this emergency kit, a zip or a folder of profiles, where rustic finds them
    /// unless they are there already.
    #[arg(long, global = true, value_name = "PATH")]
    pub kit: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// What a command acts on: every collection, or those named, repeatably; and whether it only shows
/// what it would do.
#[derive(clap::Args)]
pub(crate) struct Select {
    #[arg(short, long)]
    all: bool,
    #[arg(short, long, value_name = "COLLECTION", conflicts_with = "all", add = ArgValueCompleter::new(profiles))]
    collection: Vec<String>,
    #[arg(long)]
    dry_run: bool,
}

/// [`Select`], or repositories, for the commands that can act on one.
#[derive(clap::Args)]
pub(crate) struct SelectRepos {
    #[command(flatten)]
    select: Select,
    #[arg(short, long, value_name = "PROFILE", conflicts_with = "all", add = ArgValueCompleter::new(profiles))]
    repo: Vec<String>,
}

/// A command line as clap spells it: the subcommand and the flags with these ids, with their values. A
/// page shows it as the CLI reads it, whatever the subcommand and flags are called.
#[cfg(feature = "server")]
pub(crate) fn command_line(subcommand: &str, flags: &[(&str, Option<&str>)]) -> String {
    let cli = Cli::command();
    let sub = cli.find_subcommand(subcommand);
    let mut line = vec![crate::NAME.to_owned(), sub.map_or(subcommand, clap::Command::get_name).to_owned()];
    for (id, value) in flags {
        let long = sub.and_then(|s| s.get_arguments().find(|a| a.get_id() == id)).and_then(clap::Arg::get_long);
        line.push(format!("--{}", long.unwrap_or(id)));
        line.extend(value.map(ToOwned::to_owned));
    }
    line.join(" ")
}

impl Select {
    /// `--all` names nothing, which is every collection.
    fn named(&self, repos: &[String]) -> Named {
        Named { collections: self.collection.clone(), repos: repos.to_vec() }
    }

    fn names_nothing(&self, repos: &[String]) -> bool {
        !self.all && self.collection.is_empty() && repos.is_empty()
    }
}

#[derive(Subcommand)]
pub(crate) enum Command {
    Due {
        #[command(flatten)]
        select: SelectRepos,
    },
    Backup {
        #[command(flatten)]
        select: Select,
    },
    Copy {
        #[command(flatten)]
        select: SelectRepos,
    },
    Prune {
        #[command(flatten)]
        select: SelectRepos,
    },
    Check {
        #[command(flatten)]
        select: SelectRepos,
    },
    Restore {
        #[command(flatten)]
        select: SelectRepos,
        /// The newest snapshots not after TIME (2026-10-01, "2026-10-01 12:00").
        #[arg(long, value_name = "TIME", value_parser = engine::parse_at)]
        at_time: Option<jiff::Timestamp>,
        /// This snapshot, of the repository that --repo names (default: the primary).
        #[arg(long, value_name = "ID", conflicts_with = "at_time")]
        snapshot: Option<rustic::SnapshotId>,
        /// Restore below DIR: a --snapshot-path by its name, sources by their full path.
        #[arg(long, value_name = "DIR")]
        to: Option<PathBuf>,
        /// Only absent or empty paths, never into unmounted mount points.
        #[arg(long)]
        missing: bool,
        /// Only this source, by one of its paths (default: every source).
        #[arg(long, value_name = "PATH", conflicts_with = "snapshot")]
        source: Option<String>,
        /// Only this path of the snapshot.
        #[arg(long, value_name = "PATH", requires = "snapshot")]
        snapshot_path: Option<String>,
    },
    Recreate {
        #[command(flatten)]
        select: SelectRepos,
        /// Copy from this profile instead: a copy target, for the primary; the primary or another target,
        /// for a target.
        #[arg(long, value_name = "PROFILE", add = ArgValueCompleter::new(profiles))]
        from: Option<String>,
    },
    Notify {
        #[command(flatten)]
        select: SelectRepos,
    },
    Collections {
        #[command(flatten)]
        select: SelectRepos,
        /// List the repositories first: their snapshots, sizes and copy lag.
        #[arg(long)]
        refresh: bool,
    },
    #[cfg(feature = "server")]
    Serve {
        /// Open the web UI in the browser, logged in.
        #[arg(long)]
        open: bool,
    },
    #[cfg(feature = "desktop")]
    Install {
        /// Also turn on auto-update in rusticlone.env: at each start, the server updates rusticlone and the
        /// rustic it is tested with, each checked against its checksum and GitHub's attestation.
        #[cfg(feature = "self-update")]
        #[arg(long = AUTO_UPDATE)]
        auto_update: bool,
    },
    #[cfg(feature = "desktop")]
    Uninstall {
        /// Also delete the state directory: the runs and the login token. The
        /// profiles and the repositories stay.
        #[arg(long)]
        delete: bool,
    },
    #[cfg(feature = "self-update")]
    Update {
        /// Only show whether a newer release exists.
        #[arg(long)]
        dry_run: bool,
    },
    #[cfg(feature = "server")]
    Healthcheck,
}

impl Command {
    /// What it acts on, and whether it must be named: acting on everything is never a default, so only
    /// `collections`, which changes nothing, takes every collection without a selection. A server runs every
    /// collection; an update acts on none.
    #[cfg_attr(
        not(any(feature = "server", feature = "self-update")),
        expect(clippy::unnecessary_wraps, reason = "only the server's commands and update act on none")
    )]
    fn select(&self) -> Option<(&Select, &[String], bool)> {
        match self {
            Self::Collections { select, .. } => Some((&select.select, &select.repo, false)),
            Self::Backup { select } => Some((select, &[], true)),
            Self::Due { select }
            | Self::Copy { select }
            | Self::Prune { select }
            | Self::Check { select }
            | Self::Restore { select, .. }
            | Self::Recreate { select, .. }
            | Self::Notify { select } => Some((&select.select, &select.repo, true)),
            #[cfg(feature = "self-update")]
            Self::Update { .. } => None,
            #[cfg(feature = "desktop")]
            Self::Install { .. } | Self::Uninstall { .. } => None,
            #[cfg(feature = "server")]
            Self::Serve { .. } | Self::Healthcheck => None,
        }
    }

    pub(crate) fn named(&self) -> Named {
        self.select().map(|(select, repos, _)| select.named(repos)).unwrap_or_default()
    }

    pub(crate) fn dry_run(&self) -> bool {
        #[cfg(feature = "self-update")]
        if let Self::Update { dry_run } = self {
            return *dry_run;
        }
        self.select().is_some_and(|(select, ..)| select.dry_run)
    }
}

/// The command line's grammar, with its help from the catalog.
pub(crate) fn command() -> clap::Command {
    help(Cli::command(), "help")
}

/// `command`'s help and its arguments', from messages `<key>` and `<key>-<argument>`, else `help-<argument>` for
/// an argument that several subcommands share; then its subcommands', whose keys are `<key>-<subcommand>`.
/// `$root`: whether who runs it is root or an administrator, for whom the server starts with the computer.
fn help(command: clap::Command, key: &str) -> clap::Command {
    let find = |id: &str| i18n::find(id, [("root", crate::sys::is_root().to_string().into())]);
    let command = match find(key) {
        Some(about) => command.about(about),
        None => command,
    };
    let command = command.mut_args(|arg| {
        let id = arg.get_id().as_str().replace('_', "-");
        match find(&format!("{key}-{id}")).or_else(|| find(&format!("help-{id}"))) {
            Some(text) => arg.help(text),
            None => arg,
        }
    });
    let names: Vec<String> = command.get_subcommands().map(|c| c.get_name().to_owned()).collect();
    names.iter().fold(command, |command, name| command.mut_subcommand(name, |c| help(c, &format!("{key}-{name}"))))
}

/// Completes a collection or repository with the profiles' names, read without rustic, which would be slow.
fn profiles(typed: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let typed = typed.to_string_lossy();
    let names = crate::profile::profile_files().unwrap_or_default().into_iter().map(|(name, _)| name);
    names.filter(|n| n.starts_with(&*typed)).map(CompletionCandidate::new).collect()
}

/// The command line, refusing an action that names nothing.
pub(crate) fn parse<I: Into<std::ffi::OsString> + Clone>(
    args: impl IntoIterator<Item = I>,
) -> Result<Cli, clap::Error> {
    let mut command = command();
    let matches = command.try_get_matches_from_mut(args)?;
    let cli = Cli::from_arg_matches(&matches)?;
    if !cli
        .command
        .as_ref()
        .and_then(Command::select)
        .is_some_and(|(select, repos, needed)| needed && select.names_nothing(repos))
    {
        return Ok(cli);
    }
    let mut sub = command.find_subcommand(matches.subcommand_name().unwrap_or_default()).cloned();
    Err(sub
        .as_mut()
        .unwrap_or(&mut command)
        .error(clap::error::ErrorKind::MissingRequiredArgument, t!("cli-unselected")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NAME;

    #[test]
    fn cli_parses_selections() {
        Cli::command().debug_assert();
        let parse = |args: &[&str]| parse(std::iter::once(&NAME).chain(args));
        let named = |args: &[&str]| parse(args).unwrap().command.unwrap().named();
        let n = named(&["check", "-c", "A", "--collection=B", "--repo", "B-usb", "--json"]);
        assert_eq!((n.collections, n.repos), (vec!["A".into(), "B".into()], vec!["B-usb".into()]));
        assert!(named(&["check", "--all"]).collections.is_empty());
        // --from names where a re-create copies from, and only there.
        assert!(
            parse(&["restore", "--from", "B-usb"]).is_err()
                && parse(&["recreate", "-r", "B", "--from", "B-usb"]).is_ok()
        );
        assert!(parse(&["backup", "--repo", "B-usb"]).is_err());
        // An action needs a selection; showing collections does not.
        assert!(
            parse(&["backup"]).is_err() && parse(&["due"]).is_err() && parse(&["backup", "--all", "-c", "A"]).is_err()
        );
        assert!(parse(&["collections"]).is_ok() && parse(&["due", "-r", "A"]).is_ok());
        assert!(Cli::try_parse_from([NAME, "backup", "--all", "restore"]).is_err());
        // Only what can show what it would do has a dry run.
        assert!(parse(&["check", "--all", "--dry-run"]).is_ok_and(|c| c.command.unwrap().dry_run()));
        assert!(parse(&["healthcheck", "--dry-run"]).is_err() && parse(&["--dry-run", "update"]).is_err());
        assert!(Cli::try_parse_from([NAME, "archive"]).is_err());
        assert!(Cli::try_parse_from([NAME, "--unknown-flag"]).is_err());
    }

    /// Every subcommand and argument has help, and every `help-` message is one's.
    #[test]
    #[cfg(feature = "self-update")]
    fn every_argument_has_help() {
        fn keys(command: &clap::Command, key: &str, out: &mut Vec<String>) {
            assert!(command.get_about().is_some() || key == "help", "no message {key}");
            for arg in command.get_arguments() {
                assert!(arg.get_help().is_some(), "no message for {key} --{}", arg.get_id());
                let id = arg.get_id().as_str().replace('_', "-");
                out.extend([format!("{key}-{id}"), format!("help-{id}")]);
            }
            for sub in command.get_subcommands() {
                out.push(format!("{key}-{}", sub.get_name()));
                keys(sub, &format!("{key}-{}", sub.get_name()), out);
            }
        }
        let mut used = Vec::new();
        keys(&command(), "help", &mut used);
        let unused: Vec<String> =
            i18n::ids().into_iter().filter(|id| id.starts_with("help-") && !used.contains(id)).collect();
        assert!(unused.is_empty(), "messages of no argument: {unused:?}");
    }

    /// The command lines that pages show are the ones the CLI reads.
    #[test]
    #[cfg(feature = "server")]
    fn shown_command_lines_parse() {
        let read = |line: String| Cli::try_parse_from(line.split(' ')).map(|c| c.command);
        let restore = read(command_line("restore", &[("collection", Some("Docs"))]));
        assert!(matches!(restore, Ok(Some(Command::Restore { select, .. })) if select.select.collection == ["Docs"]));
        assert!(matches!(read(command_line("serve", &[("open", None)])), Ok(Some(Command::Serve { open: true, .. }))));
    }

    /// What `install` writes into the service is a plain server, which reads whether it updates itself from
    /// the env file, as one started any other way.
    #[cfg(all(feature = "desktop", feature = "self-update"))]
    #[test]
    fn the_installed_server_is_a_plain_one() {
        let args = std::iter::once(NAME.to_owned()).chain(crate::install::autostart::serve_args());
        assert!(matches!(Cli::try_parse_from(args).map(|c| c.command), Ok(Some(Command::Serve { open: false }))));
        let install = Cli::try_parse_from(command_line("install", &[("auto_update", None)]).split(' '));
        assert!(matches!(install.map(|c| c.command), Ok(Some(Command::Install { auto_update: true }))));
    }
}
