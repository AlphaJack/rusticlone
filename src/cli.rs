//! The command line.

use std::{cell::RefCell, iter, process::ExitCode, time::Duration};

use anyhow::Result;
use jiff::SignedDuration;

mod board;
mod collections;
mod text;
mod tree;

use crate::{
    args::{self, Cli, Command, parse},
    clock,
    engine::{self, Asked, Batch, Collection, Engine},
    i18n::t,
    log::{self, Level, Logged},
    notification, profile,
    request::{RestoreOptions, RunKind, Trigger},
    result::{RunResults, StepResult},
    rustic::Rustic,
    settings,
    state::Store,
};

/// The exit code a shell uses for a command stopped by SIGINT.
const INTERRUPTED: u8 = 130;

pub fn main() -> ExitCode {
    clap_complete::CompleteEnv::with_factory(args::command).complete();
    let args = std::env::args_os();
    #[cfg(all(target_vendor = "apple", feature = "desktop"))]
    let args = args.chain(
        crate::install::autostart::opened_as_app().then(|| ["serve".into(), "--open".into()]).into_iter().flatten(),
    );
    let cli = match parse(args) {
        Ok(cli) => cli,
        // Usage errors exit with 1, because 2 means warnings.
        Err(e) => return e.print().map_or(ExitCode::FAILURE, |()| ExitCode::from(u8::from(e.use_stderr()))),
    };
    log::init(cli.json, cli.verbose);
    // Stopping when asked is how a server ends: launchd and systemd restart it only after a failure.
    #[cfg(feature = "server")]
    let serving = matches!(cli.command, Some(Command::Serve { .. }));
    #[cfg(not(feature = "server"))]
    let serving = false;
    let result = settings::load_env_file().and_then(|()| real_main(cli));
    if clock::interrupted() && !serving {
        return ExitCode::from(INTERRUPTED);
    }
    match result {
        Ok(level) => ExitCode::from(match level {
            Level::Success => 0,
            Level::Warning => 2,
            Level::Failure => 1,
        }),
        Err(e) => {
            log::event(None, &Logged::Error { error: format!("{e:#}") });
            ExitCode::FAILURE
        }
    }
}

/// Rustic runs in its own process group, so Ctrl-C and SIGTERM reach only this process, which
/// then stops every rustic call and run. A second signal exits at once.
fn handle_signals() -> Result<()> {
    let mut stopping = false;
    ctrlc::set_handler(move || {
        if std::mem::replace(&mut stopping, true) {
            std::process::exit(INTERRUPTED.into());
        }
        clock::interrupt();
    })?;
    Ok(())
}

/// Commands about the server that need no state: a healthcheck, opening the running server, installing it.
#[cfg(feature = "server")]
fn without_state(command: &Command) -> Result<Option<Level>> {
    use crate::{health, server::open_running};
    let listen = &settings::LISTEN.get(&[]).unwrap_or_default();
    if matches!(command, Command::Healthcheck) {
        health::check(listen)?;
        log::event(None, &crate::server::Logged::Healthy { listen: listen.clone() });
        return Ok(Some(Level::Success));
    }
    #[cfg(feature = "desktop")]
    if matches!(command, Command::Install { .. } | Command::Uninstall { .. }) {
        use crate::install::autostart;
        let install = matches!(command, Command::Install { .. });
        let label = if install { t!("cli-install") } else { t!("cli-uninstall") };
        return board::command(&label, || {
            // Here, in a terminal: the server started at login or with the computer has none. Asked, unless
            // `--auto-update` agreed to downloads already.
            #[cfg(feature = "self-update")]
            if let Command::Install { auto_update } = command {
                let download = || crate::install::rustic::download(!auto_update);
                board::step(&t!("cli-step-check-rustic"), &t!("cli-step-checked-rustic"), download)?;
            }
            #[cfg(feature = "self-update")]
            if matches!(command, Command::Install { auto_update: true }) {
                board::step(&t!("cli-step-auto-update"), &t!("cli-step-auto-updated"), || {
                    // A package manager's executable is updated by it instead.
                    if crate::install::update::replaceable() {
                        let file = settings::write_env_file(&settings::AUTO_UPDATE, "true")?;
                        log::event(None, &autostart::Logged::AutoUpdate { file: file.display().to_string() });
                    } else {
                        log::event(None, &autostart::Logged::AutoUpdateManaged);
                    }
                    Ok(())
                })?;
            }
            let service = if autostart::with_computer() { t!("cli-boot") } else { t!("cli-login") };
            if install {
                let exe = board::step(&t!("cli-step-place"), &t!("cli-step-placed"), autostart::place)?;
                board::step(&t!("cli-step-install", service = service), "", || {
                    log::event(None, &autostart::install(listen, &exe)?);
                    Ok(())
                })?;
            } else {
                board::step(&t!("cli-step-uninstall", service = service), "", || {
                    log::event(None, &autostart::uninstall()?);
                    Ok(())
                })?;
            }
            // A first-time user creates the first collection in the web UI. Root's server has no desktop to open
            // it in.
            if install
                && !crate::sys::is_root()
                && profile::profile_files()?.is_empty()
                && autostart::answers_soon(listen)
            {
                board::step(&t!("cli-step-open"), &t!("cli-step-opened"), || open_running(listen))?;
            }
            if matches!(command, Command::Uninstall { delete: true }) {
                board::step(&t!("cli-step-delete"), &t!("cli-step-deleted"), || {
                    autostart::delete()?.iter().for_each(|deleted| log::event(None, deleted));
                    Ok(())
                })?;
            }
            Ok(Level::Success)
        })
        .map(Some);
    }
    if let Command::Serve { open: true, .. } = command {
        #[cfg(all(any(target_os = "linux", target_vendor = "apple"), feature = "desktop"))]
        if health::check(listen).is_err() {
            crate::install::autostart::start_installed(listen)?;
        }
        if open_running(listen)? {
            return Ok(Some(Level::Success));
        }
    }
    Ok(None)
}

/// Updates this executable, then rustic.
#[cfg(feature = "self-update")]
fn update(dry_run: bool) -> Result<Level> {
    board::command(&t!("cli-update"), || {
        let step = if dry_run { t!("cli-step-check-update") } else { t!("cli-step-update") };
        if board::step(&step, "", || crate::install::update::update(dry_run))? {
            let download = || crate::install::rustic::download(true);
            board::step(&t!("cli-step-check-rustic"), &t!("cli-step-checked-rustic"), download)?;
        }
        Ok(Level::Success)
    })
}

fn real_main(cli: Cli) -> Result<Level> {
    let Some(command) = cli.command else {
        args::command().print_help()?;
        return Ok(Level::Success);
    };
    let dry_run = command.dry_run();
    #[cfg(feature = "self-update")]
    if matches!(command, Command::Update { .. }) {
        return update(dry_run);
    }
    #[cfg(feature = "server")]
    if let Some(level) = without_state(&command)? {
        return Ok(level);
    }
    settings::check_env()?;
    #[cfg(feature = "self-update")]
    if !dry_run {
        crate::install::rustic::download(true)?;
    }
    let s = &settings::STALL_TIMEOUT;
    let stall: Duration =
        s.get(&[]).unwrap_or_default().parse::<SignedDuration>().map_err(|e| s.invalid(e))?.try_into()?;
    let store = if dry_run { Store::open_dry_run()? } else { Store::open()? };
    let named = command.named();
    // Rustic's version is read again with the profiles, because it may have been upgraded since.
    let build = move |runs| {
        let rustic = Rustic::new(stall, dry_run)?;
        Engine::new(rustic, store.clone(), runs, named.clone())
    };
    let engine = || -> Result<Engine> {
        handle_signals()?;
        let engine = build(board::runs())?;
        // Fails on a name that matches nothing, which every command would otherwise skip.
        engine.selections()?;
        board::titles(&engine);
        Ok(engine)
    };
    let none = RestoreOptions::default();
    match command {
        // Answered before the state is opened.
        #[cfg(feature = "self-update")]
        Command::Update { .. } => Ok(Level::Success),
        #[cfg(feature = "desktop")]
        Command::Install { .. } | Command::Uninstall { .. } => Ok(Level::Success),
        #[cfg(feature = "server")]
        Command::Healthcheck => Ok(Level::Success),
        #[cfg(feature = "server")]
        Command::Serve { open, .. } => {
            let listen = settings::LISTEN.get(&[]).unwrap_or_default();
            crate::server::serve(build, &listen, open).map(|()| Level::Success)
        }
        Command::Due { .. } => Ok(due(&engine()?)),
        Command::Collections { refresh, .. } => {
            let engine = engine()?;
            engine.list_unknown();
            Ok(collections::show(&engine, refresh))
        }
        Command::Notify { .. } => Ok(notify(&engine()?)),
        Command::Backup { .. } => run(&engine()?, RunKind::Backup, &none, None),
        Command::Copy { .. } => run(&engine()?, RunKind::Copy, &none, None),
        Command::Prune { .. } => run(&engine()?, RunKind::Prune, &none, None),
        Command::Check { .. } => run(&engine()?, RunKind::Check, &none, None),
        Command::Recreate { from, .. } => run(&engine()?, RunKind::Recreate, &RestoreOptions { from, ..none }, None),
        // Without choices, a dry run reads the snapshots as a restore would. With them, it only says what
        // would be restored where, from the listings.
        Command::Restore { snapshot, source, at_time: None, to: None, missing: false, snapshot_path: None, .. }
            if dry_run =>
        {
            run(&engine()?, RunKind::TestRestore, &RestoreOptions { snapshot, ..none }, source.as_deref())
        }
        Command::Restore { at_time: at, snapshot, to, missing, source, snapshot_path: path, .. } => {
            let options = RestoreOptions { at, snapshot, path, to, missing, ..none };
            let engine = engine()?;
            let level = run(&engine, RunKind::Restore, &options, source.as_deref())?;
            if !dry_run {
                keep_profiles(&engine)?;
            }
            Ok(level)
        }
    }
}

#[derive(crate::i18n::Fluent)]
#[fluent(prefix = "cli-ask")]
enum Ask {
    Keep { files: String, dir: String },
}

/// After a restore with profiles from the working directory, as in a disaster recovery, offers to copy
/// those of the restored collections where rusticlone and its server read them.
#[expect(clippy::disallowed_methods, reason = "copies the profiles")]
fn keep_profiles(engine: &Engine) -> Result<()> {
    let (Ok(cwd), Some(dir)) = (std::env::current_dir(), profile::own_dir()) else { return Ok(()) };
    let collections = engine.collections();
    let files: Vec<String> = (collections.iter().flat_map(Collection::repos))
        .filter(|p| p.dir == cwd && cwd != dir)
        .map(|p| format!("{}.toml", p.name))
        .collect();
    if files.is_empty()
        || !crate::sys::confirm(&Ask::Keep { files: files.join(", "), dir: dir.display().to_string() }.to_string())?
    {
        return Ok(());
    }
    std::fs::create_dir_all(&dir)?;
    for file in files {
        std::io::copy(&mut std::fs::File::open(cwd.join(&file))?, &mut std::fs::File::create_new(dir.join(&file))?)?;
    }
    Ok(())
}

/// Runs what is due in each named collection. Requests that run elsewhere are skipped: the next
/// invocation catches up. In a terminal, says when the next run is due if none is now: from cron, which
/// mails what a command prints, it stays silent.
fn due(engine: &Engine) -> Level {
    engine.list_unknown();
    let batch = Batch::new(Trigger::Schedule);
    for_each_collection(engine, &t!("cli-due"), |collection| {
        let mut ran = engine.schedule(collection, &batch);
        if ran.results.is_empty() && std::io::IsTerminal::is_terminal(&std::io::stdout()) {
            let next = engine.next_due(collection).map(|(request, at)| (request.label(collection.name()), at));
            ran.push(StepResult::NothingDue { next });
        }
        ran
    })
}

/// Sends each named collection's status, and says what it sent.
fn notify(engine: &Engine) -> Level {
    use notification::Logged as N;
    let collections = engine.collections();
    let Some(mut level) = found(engine, &collections) else { return Level::Warning };
    let names: Vec<&str> = collections.iter().map(Collection::name).collect();
    board::steps(&names, &t!("cli-notify"), &t!("cli-notify-step"), |i| {
        let mut warnings = Vec::new();
        let sent = match engine.notify_now(&collections[i], &mut |warning| warnings.push(warning)) {
            Ok(text) if engine.store.is_dry_run() => N::WouldNotify { text },
            Ok(text) => N::Notified { text },
            Err(e) => {
                level = Level::Failure;
                N::Undelivered { error: e.to_string() }
            }
        };
        iter::once(sent).chain(warnings.into_iter().map(|warning| N::Warned { warning })).map(Sent).collect()
    });
    level
}

/// A notification the command was asked to send. Failing to send it is the command's failure, where a
/// status change that fails to notify is only a warning.
#[derive(serde::Serialize)]
#[serde(transparent)]
struct Sent(notification::Logged);

impl std::fmt::Display for Sent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl log::Message for Sent {
    fn level(&self) -> Level {
        if matches!(self.0, notification::Logged::Undelivered { .. }) { Level::Failure } else { self.0.level() }
    }
}

/// Runs every run of `kind` that the command names, one collection after the other, until stopped.
/// `source`: a restore of only the source with this path, in the collections that have it.
fn run(engine: &Engine, kind: RunKind, options: &RestoreOptions, source: Option<&str>) -> Result<Level> {
    let mut requests = Vec::new();
    for selection in engine.selections()? {
        let Some(path) = source else {
            requests.extend(engine.requests(&selection, kind, options)?);
            continue;
        };
        for (collection, repo) in engine.named(&selection)? {
            let Some(source) = collection.source_index(path) else { continue };
            let options = RestoreOptions { source: Some(source), ..options.clone() };
            let own = collection.requests(kind, repo, &options)?.into_iter();
            requests.extend(own.map(|r| (collection.name().to_owned(), r)));
        }
    }
    if let (Some(path), true) = (source, requests.is_empty()) {
        return Err(engine::Error::NotASource { path: path.to_owned() }.into());
    }
    // Queued at once, as from the web UI, so that each shows before it runs.
    let collections = engine.collections();
    let queued: Vec<(String, Result<Asked>)> = (requests.iter())
        .filter_map(|(name, request)| {
            let collection = collections.iter().find(|c| c.name() == name)?;
            Some((name.clone(), engine.ask(collection, request, Trigger::Cli)))
        })
        .collect();
    board::queued(queued.iter().filter_map(|(_, q)| Some(q.as_ref().ok()?.record())));
    let queued = RefCell::new(queued);
    let batch = Batch::new(Trigger::Cli);
    let level = for_each_collection(engine, &kind.label(), |collection| {
        let (own, rest) = queued.take().into_iter().partition(|(name, _)| name == collection.name());
        queued.replace(rest);
        let lines = own.into_iter().flat_map(|(_, queued): (_, Result<Asked>)| match queued {
            Ok(asked) if clock::interrupted() => {
                stopped(engine, &asked);
                Vec::new()
            }
            Ok(Asked::Queued(queued)) => {
                let record = queued.record.clone();
                let results = engine.execute(collection, queued, &batch).results;
                board::ended(&record);
                results
            }
            Ok(Asked::Joined(run)) => {
                let results = engine.follow(collection, &run).results;
                board::ended(&run);
                results
            }
            Err(e) => vec![StepResult::Error { error: format!("{e:#}") }],
        });
        RunResults { collection: collection.title.to_owned(), results: lines.collect() }
    });
    // Stopped before their collection's turn.
    queued.take().into_iter().filter_map(|(_, q)| q.ok()).for_each(|asked| stopped(engine, &asked));
    Ok(level)
}

/// Takes a request stopped before its turn out of the queue. One that joined another leaves it there.
fn stopped(engine: &Engine, asked: &Asked) {
    if let Asked::Queued(queued) = asked {
        _ = engine.store.dequeue(&queued.record.collection, queued.record.id);
    }
    board::ended(asked.record());
}

/// Reports broken profiles, and returns their level. Without them or `collections`, says where profiles go instead.
fn found(engine: &Engine, collections: &[Collection<'_>]) -> Option<Level> {
    let level = engine.report_broken();
    if collections.is_empty() && level == Level::Success {
        log::event(None, &Logged::NoProfiles { dirs: profile::dirs_searched() });
        return None;
    }
    Some(level)
}

/// Reports broken profiles and each collection, sends status changes, and returns the worst level. `heading`:
/// what the command does, which results that no run showed follow.
fn for_each_collection(engine: &Engine, heading: &str, f: impl Fn(&Collection<'_>) -> RunResults) -> Level {
    let collections = engine.collections();
    let Some(mut level) = found(engine, &collections) else { return Level::Warning };
    for collection in collections.iter().take_while(|_| !clock::interrupted()) {
        let mut report = f(collection);
        board::leave_out(&mut report);
        board::print(&report, heading);
        level = level.max(report.level());
        engine.notify(collection);
    }
    level
}
