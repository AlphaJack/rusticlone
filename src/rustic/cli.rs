//! The `rustic` CLI, run as a process.

#![expect(clippy::disallowed_methods, reason = "runs rustic")]

use std::{
    io::{self, BufRead, BufReader, Read},
    path::Path,
    process::{Command, ExitStatus, Stdio},
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use jiff::{SignedDuration, Timestamp};

use super::Ctx;
use super::{
    Call, Error, Invocation, Output, PROGRESS, Progress, Runner, Stdout, Version,
    output::{Log, Message},
    process::{kill, work},
    shell_word,
};
use crate::fmt;

/// How often a running rustic is checked for activity and stop requests.
const TICK: Duration = Duration::from_millis(500);
/// Bytes read at a time from a counted stdout.
const READ_CHUNK: usize = 1 << 16;

pub(crate) struct Cli {
    pub(super) stall_timeout: Duration,
}

/// Messages from the threads that watch rustic.
enum Line {
    Out(String),
    Err(String),
    Bytes(usize),
    Exited(io::Result<ExitStatus>),
}

impl Cli {
    /// Rustic. On Windows, without a console window, since the server started at login has none and
    /// Windows would open one for each run, and below normal priority, as systemd's `Nice` and launchd's
    /// `Background` run it elsewhere, so that a backup does not slow down the user.
    pub(crate) fn rustic() -> Command {
        #[cfg_attr(not(windows), expect(unused_mut, reason = "flagged on Windows only"))]
        let mut cmd = Command::new(super::install::bin());
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::{BELOW_NORMAL_PRIORITY_CLASS, CREATE_NO_WINDOW};
            std::os::windows::process::CommandExt::creation_flags(
                &mut cmd,
                CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS,
            );
        }
        cmd
    }

    /// Rustic without progress output, in its own process group. Stopping the group also stops what
    /// rustic started (rclone, stdin commands), which would otherwise keep running and keep its locks.
    fn command(profile: &str, dir: &Path) -> Command {
        let mut cmd = Self::rustic();
        // These override every profile: a stray RUSTIC_REPOSITORY would send all of them to one repository,
        // RUSTIC_DRY_RUN would report backups never made, and the others change the output read back.
        // The rest, such as RUSTIC_PASSWORD or RUSTIC_CACHE_DIR, are the user's to set.
        for key in ["REPOSITORY", "REPO_HOT", "USE_PROFILE", "DRY_RUN", "GROUP_BY", "JSON_PROGRESS"] {
            cmd.env_remove(format!("RUSTIC_{key}"));
        }
        // Not `--no-progress`, which would also disable `--json-progress`. Rustic colors by `TERM` even
        // into a pipe, and its output ends up in logs and on web pages.
        cmd.env("RUSTIC_NO_PROGRESS", "true")
            .env("NO_COLOR", "1")
            .arg("-P")
            .arg(profile)
            .current_dir(dir)
            .stdin(Stdio::null());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        cmd
    }

    pub(super) fn version() -> Result<Version> {
        let out = Self::rustic().arg("--version").output().map_err(Error::CannotRun)?;
        let text = String::from_utf8_lossy(&out.stdout);
        Ok(super::install::parse_version(&text).ok_or_else(|| Error::NoVersion { output: text.trim().to_owned() })?)
    }

    #[expect(clippy::too_many_arguments, reason = "one call's context, command and outputs")]
    fn exec(
        &self,
        cx: &Ctx,
        call: usize,
        profile: &str,
        args: &[&str],
        cmd: &mut Command,
        mut progress: Option<&mut dyn FnMut(&Progress)>,
        stdout: Stdout,
    ) -> Result<(Output, u64)> {
        let log = |line: &str| {
            crate::log::rustic(profile, line);
            cx.log(Some(call), &format!("[{profile}] {line}"));
        };
        let command = args.first().copied().unwrap_or_default();
        if let Some(stop) = self.stop(cx, command, None) {
            return Err(stop.into());
        }
        #[cfg(unix)]
        inherit_locks(cmd, cx);
        let child = cmd.spawn().map_err(Error::CannotRun)?;
        #[cfg(windows)]
        super::process::tie(std::os::windows::io::AsRawHandle::as_raw_handle(&child));
        log(&format!("rustic {}", args.join(" ")));

        let pid = child.id();
        let rx = read(child, stdout)?;
        let (mut text, mut bytes, mut alive, mut status) = (String::new(), 0, Instant::now(), None);
        let mut worked = work(pid);
        let mut stderr = Log::default();
        loop {
            let received = rx.recv_timeout(TICK);
            // No output while it hashes or waits for a remote. Its I/O shows that it is still working.
            let work = work(pid);
            if received.is_ok() || work != worked {
                (alive, worked) = (Instant::now(), work);
            }
            match received {
                Ok(Line::Err(line)) => {
                    log(&line);
                    stderr.line(&line);
                }
                // Progress lines, requested here or by the profile, are not part of the parsed output.
                Ok(Line::Out(line)) => match serde_json::from_str::<Message>(&line) {
                    Ok(message) if message.message_type == "status" => {
                        if let Some(callback) = progress.as_mut() {
                            callback(&message.progress);
                        }
                    }
                    _ => text.push_str(&format!("{line}\n")),
                },
                Ok(Line::Bytes(n)) => bytes += n as u64,
                Ok(Line::Exited(exited)) => status = Some(exited?),
                // Exited and silent. A process that rustic started may keep the pipes open.
                Err(RecvTimeoutError::Timeout) if status.is_some() => break,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if status.is_none()
                && let Some(why) = self.stop(cx, command, Some(alive))
            {
                kill(pid);
                return Err(why.into());
            }
        }
        let status = status.ok_or(Error::NoStatus)?;
        let output = Output {
            success: status.success(),
            stdout: text,
            command: command.to_owned(),
            code: status.code(),
            stderr,
        };
        Ok((output, bytes))
    }

    /// Why rustic should be stopped, if it should. `alive`: its last sign of activity, if it runs.
    fn stop(&self, cx: &Ctx, command: &str, alive: Option<Instant>) -> Option<Error> {
        if let Some(stop) = cx.stop() {
            Some(Error::Stopped(stop))
        } else if alive.is_some_and(|alive| alive.elapsed() > self.stall_timeout) {
            let silent = fmt::human_duration(self.stall_timeout.as_secs());
            Some(Error::Stalled { command: command.to_owned(), silent })
        } else {
            None
        }
    }
}

/// Rustic inherits the run's locks, so that they stay held while it runs, even if this process
/// is killed. On Windows, rustic ends with this process instead (see [`super::process::tie`]).
#[cfg(unix)]
fn inherit_locks(cmd: &mut Command, cx: &Ctx) {
    let locks = cx.held();
    // SAFETY: only fcntl, async-signal-safe, between fork and exec.
    #[expect(unsafe_code, reason = "pass lock descriptors through fork and exec with fcntl")]
    unsafe {
        std::os::unix::process::CommandExt::pre_exec(cmd, move || {
            for fd in &locks {
                libc::fcntl(*fd, libc::F_SETFD, 0);
            }
            Ok(())
        })
    };
}

/// Detached threads read rustic's output and wait for its exit. After a kill, processes that rustic
/// started may keep the pipes open.
fn read(mut child: std::process::Child, stdout: Stdout) -> Result<mpsc::Receiver<Line>> {
    let (tx, rx) = mpsc::channel();
    let (out, err) = (child.stdout.take().ok_or(Error::NotPiped)?, child.stderr.take().ok_or(Error::NotPiped)?);
    let lines = |pipe: Box<dyn Read + Send>, tx: mpsc::Sender<Line>, wrap: fn(String) -> Line| {
        thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(io::Result::ok) {
                if tx.send(wrap(line)).is_err() {
                    return;
                }
            }
        })
    };
    match stdout {
        Stdout::Text => _ = lines(Box::new(out), tx.clone(), Line::Out),
        Stdout::Count => {
            let (tx, mut out) = (tx.clone(), out);
            thread::spawn(move || {
                let mut buf = vec![0; READ_CHUNK];
                while let Ok(n @ 1..) = out.read(&mut buf) {
                    if tx.send(Line::Bytes(n)).is_err() {
                        return;
                    }
                }
            });
        }
    }
    lines(Box::new(err), tx.clone(), Line::Err);
    thread::spawn(move || tx.send(Line::Exited(child.wait())));
    Ok(rx)
}

impl Runner for Cli {
    fn run(
        &self,
        cx: &Ctx,
        call: &Invocation<'_>,
        progress: Option<&mut dyn FnMut(&Progress)>,
    ) -> Result<(Output, u64)> {
        let &Invocation { profile, dir, args, stdout } = call;
        let (started, clock) = (Timestamp::now(), Instant::now());
        let mut cmd = Self::command(profile, dir);
        if progress.is_some() {
            cmd.env_remove("RUSTIC_NO_PROGRESS");
            let interval = SignedDuration::try_from(PROGRESS)?;
            cmd.args(["--json-progress", "--progress-interval", &format!("{interval:#}")]);
        }
        cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
        let call = Call {
            step: None,
            args: cmd.get_args().map(shell_word).collect::<Vec<_>>().join(" "),
            bin: Some(cmd.get_program().to_string_lossy().into_owned()),
            dir: cmd.get_current_dir().map(|dir| dir.to_string_lossy().into_owned()),
            started,
            secs: 0.0,
            exit: None,
        };
        let call = cx.call(call);
        let result = self.exec(cx, call, profile, args, &mut cmd, progress, stdout);
        let exit = match &result {
            Ok((out, _)) => out.code.map_or_else(|| out.ended(), |c| c.to_string()),
            Err(e) => format!("{e:#}"),
        };
        cx.ended(call, clock.elapsed().as_secs_f64(), exit);
        result
    }

    #[cfg(feature = "server")]
    fn command(&self, profile: &str, dir: &Path) -> Command {
        Cli::command(profile, dir)
    }
}
