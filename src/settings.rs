#![expect(clippy::disallowed_methods, reason = "settings are read from the environment")]

use std::{env, path::PathBuf};

use etcetera::BaseStrategy;

use anyhow::Result;
use serde::Serialize;

use crate::profile::RusticProfile;

mod cron;

use cron::Cron;

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "settings-error")]
pub(crate) enum Error {
    #[fluent(transparent = key)]
    Invalid {
        key: &'static str,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    NoSchedule {
        expr: String,
    },
    ProfileOnly {
        key: &'static str,
    },
    EnvFile {
        path: String,
        source: dotenvy::Error,
    },
    #[cfg(all(feature = "desktop", feature = "self-update"))]
    NoConfigDir,
}

/// Where a setting is read: the environment, or a profile's `[global.env]`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    Environment,
    /// Also read from the file that `<KEY>_FILE` names, so that it need not be in the environment.
    Secret,
    Collection,
    /// The repository's profile first, then the backup profile's.
    Repository,
}

impl Scope {
    /// Read at start, from the environment.
    pub(crate) fn at_start(self) -> bool {
        matches!(self, Self::Environment | Self::Secret)
    }
}

pub(crate) struct Setting {
    pub key: &'static str,
    pub default: Option<&'static str>,
    pub scope: Scope,
}

impl Setting {
    /// What it does, from message `setting-<key>`: `setting-backup-cron`.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the tests of the env file and the catalog"))]
    pub(crate) fn help(&self) -> Option<String> {
        let key = self.key.strip_prefix(crate::ENV_PREFIX).unwrap_or(self.key).to_lowercase().replace('_', "-");
        crate::i18n::find(&format!("setting-{key}"), [])
    }

    /// For a setting read at start the environment, else its file; for the others the first profile in
    /// `chain` that sets it. Else the default. A blank value is not set.
    pub(crate) fn get(&self, chain: &[&RusticProfile]) -> Option<String> {
        let set = |v: &String| !v.trim().is_empty();
        let value = if self.scope.at_start() {
            env::var(self.key).ok().filter(set).or_else(|| self.file().ok().flatten().filter(set))
        } else {
            chain.iter().find_map(|p| p.env.get(self.key).filter(|v| set(v)).cloned())
        };
        value.or_else(|| self.default.map(Into::into))
    }

    /// The trimmed contents of a secret's file.
    fn file(&self) -> Result<Option<String>, Error> {
        let path = (self.scope == Scope::Secret).then(|| env::var_os(format!("{}_FILE", self.key)));
        let Some(path) = path.flatten() else { return Ok(None) };
        Ok(Some(std::fs::read_to_string(path).map_err(|e| self.invalid(e))?.trim().to_owned()))
    }

    pub(crate) fn invalid(&self, source: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Error {
        Error::Invalid { key: self.key, source: source.into() }
    }

    /// `true` or `false`, in any case.
    #[cfg(all(feature = "server", feature = "self-update"))]
    pub(crate) fn enabled(&self) -> Result<bool, Error> {
        self.get(&[]).unwrap_or_default().to_lowercase().parse().map_err(|e| self.invalid(e))
    }

    /// `None` if unset.
    pub(crate) fn schedule(&self, chain: &[&RusticProfile]) -> Result<Option<Cron>> {
        Ok(self.get(chain).map(|expr| cron(&expr)).transpose().map_err(|e| self.invalid(e))?)
    }
}

/// Fails on a `<KEY>_FILE` that cannot be read, which [`Setting::get`] would skip, and on a profile's setting
/// in the environment, which it ignores.
pub(crate) fn check_env() -> Result<(), Error> {
    ALL.iter().try_for_each(|s| match env::var_os(s.key) {
        Some(_) if !s.scope.at_start() => Err(Error::ProfileOnly { key: s.key }),
        _ => s.file().map(drop),
    })
}

/// A 5-field cron expression in local time, or `@daily` and the like.
pub(crate) fn cron(expr: &str) -> Result<Cron, Error> {
    Cron::parse(expr).ok_or_else(|| Error::NoSchedule { expr: expr.to_owned() })
}

/// Declares each setting, and [`ALL`] in the same order, sorted by key.
macro_rules! settings {
    ($($(#[$attr:meta])* $name:ident = $key:literal, $default:expr, $scope:expr;)*) => {
        $($(#[$attr])* pub(crate) const $name: Setting = Setting { key: env_key!($key), default: $default, scope: $scope };)*
        pub(crate) const ALL: &[&Setting] = &[$($(#[$attr])* &$name,)*];
    };
}

settings! {
    #[cfg(feature = "server")]
    ADMIN_TOKEN = "ADMIN_TOKEN", None, Scope::Secret;
    #[cfg(feature = "self-update")]
    AUTO_UPDATE = "AUTO_UPDATE", Some("false"), Scope::Environment;
    BACKUP = "BACKUP_CRON", None, Scope::Collection;
    CHECK = "CHECK_CRON", None, Scope::Repository;
    CHECK_SUBSETS = "CHECK_SUBSETS", Some("10"), Scope::Repository;
    #[cfg(feature = "server")]
    KEEP_AWAKE = "KEEP_AWAKE", Some("true"), Scope::Environment;
    #[cfg(feature = "server")]
    LISTEN = "LISTEN", Some("127.0.0.1:32110"), Scope::Environment;
    #[cfg(feature = "server")]
    METRICS_TOKEN = "METRICS_TOKEN", None, Scope::Secret;
    NOTIFY = "NOTIFY_URLS", None, Scope::Collection;
    PRUNE = "PRUNE_CRON", None, Scope::Repository;
    RUSTIC = "RUSTIC_BIN", None, Scope::Environment;
    STALL_TIMEOUT = "STALL_TIMEOUT", Some("1h"), Scope::Environment;
    STATE_DIR = "STATE_DIR", None, Scope::Environment;
}

/// A variable with our prefix in a profile's `[global.env]` that has no effect there.
#[derive(Debug, Clone, Serialize, crate::i18n::Fluent)]
#[fluent(prefix = "settings-ignored")]
pub(crate) struct Ignored {
    pub key: String,
    pub why: Unused,
}

#[derive(Debug, Clone, Serialize, crate::i18n::Fluent)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Unused {
    Unknown,
    EnvironmentOnly,
    /// Set on a copy target, which uses the settings of these backup profiles.
    #[fluent(profiles = v0.join(", "))]
    FollowsProfiles(Vec<String>),
}

/// `profiles`: the backup profiles that copy to `p`.
pub(crate) fn ignored(p: &RusticProfile, profiles: &[&RusticProfile]) -> Vec<Ignored> {
    p.env
        .iter()
        .filter(|(key, _)| key.starts_with(crate::ENV_PREFIX))
        .filter_map(|(key, value)| {
            let why = match ALL.iter().find(|s| s.key == key).map(|s| s.scope) {
                None => Unused::Unknown,
                Some(scope) if scope.at_start() => Unused::EnvironmentOnly,
                // No effect unless every profile sets the same value, as when they share a profile.
                Some(Scope::Collection)
                    if !p.is_backup_profile() && profiles.iter().any(|j| j.env.get(key) != Some(value)) =>
                {
                    Unused::FollowsProfiles(profiles.iter().map(|j| j.name.clone()).collect())
                }
                Some(_) => return None,
            };
            Some(Ignored { key: key.clone(), why })
        })
        .collect()
}

/// `rusticlone.env` in the configuration directory.
fn env_file() -> Option<PathBuf> {
    crate::sys::own_config_dir(crate::NAME).map(|d| d.join(format!("{}.env", crate::NAME)))
}

/// Sets the variables of the env file that the environment does not set already. Called before any thread
/// starts, since setting a variable is not thread-safe.
pub(crate) fn load_env_file() -> Result<()> {
    let Some(path) = env_file() else { return Ok(()) };
    match dotenvy::from_path(&path) {
        Err(dotenvy::Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        r => Ok(r.map_err(|source| Error::EnvFile { path: path.display().to_string(), source })?),
    }
}

/// The env file's mode: it may hold secrets.
#[cfg(all(unix, feature = "desktop", feature = "self-update"))]
const OWNER_ONLY: u32 = 0o600;

#[cfg(all(feature = "desktop", feature = "self-update"))]
/// Writes `value` for `setting` into the env file, created readable by its owner only, and returns its path.
pub(crate) fn write_env_file(setting: &Setting, value: &str) -> Result<PathBuf> {
    let path = env_file().ok_or(Error::NoConfigDir)?;
    let text = match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        text => text?,
    };
    std::fs::create_dir_all(path.parent().unwrap_or(&path))?;
    let mut file = std::fs::OpenOptions::new();
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut file, OWNER_ONLY);
    std::io::Write::write_all(
        &mut file.write(true).create(true).truncate(true).open(&path)?,
        with_line(&text, setting.key, value).as_bytes(),
    )?;
    Ok(path)
}

/// Whether the env file switches `setting` on, which the next start reads unless the environment sets it.
#[cfg(all(tray, feature = "self-update"))]
pub(crate) fn on_in_env_file(setting: &Setting) -> bool {
    let set = env_file().and_then(|path| dotenvy::from_path_iter(path).ok()).and_then(|vars| {
        vars.filter_map(Result::ok).filter(|(key, _)| key == setting.key).last().map(|(_, value)| value)
    });
    set.or_else(|| setting.default.map(Into::into)).is_some_and(|v| v.eq_ignore_ascii_case("true"))
}

#[cfg(all(feature = "desktop", feature = "self-update"))]
/// `text` with `key=value` in place of the line that sets `key`, else of the commented one that shows its
/// default, as the env file in `packaging` does, else appended. Every other line stays as it was.
fn with_line(text: &str, key: &str, value: &str) -> String {
    let sets = |line: &str, commented: bool| {
        let line = line.trim_start();
        let line = if commented { line.strip_prefix('#').map(str::trim_start) } else { Some(line) };
        let line = line.map(|l| l.strip_prefix("export ").unwrap_or(l));
        line.and_then(|l| l.strip_prefix(key)).is_some_and(|rest| rest.trim_start().starts_with('='))
    };
    let mut lines: Vec<&str> = text.lines().collect();
    let new = format!("{key}={value}");
    // The last line that sets it wins, as when the file is read.
    let line = lines.iter().rposition(|l| sets(l, false)).or_else(|| lines.iter().position(|l| sets(l, true)));
    match line {
        Some(i) => lines[i] = &new,
        None => lines.push(&new),
    }
    lines.join("\n") + "\n"
}

/// Where the state is kept: by default, for root or an administrator, whose server is the system's, the
/// system's state directory, else the user's, machine-local, so not the Windows roaming profile, and not the
/// purgeable macOS caches.
pub(crate) fn state_dir() -> Result<PathBuf> {
    if let Some(dir) = STATE_DIR.get(&[]) {
        return Ok(dir.into());
    }
    if crate::sys::is_root() {
        return Ok(if cfg!(windows) {
            PathBuf::from(env::var_os("PROGRAMDATA").unwrap_or_else(|| r"C:\ProgramData".into()))
                .join(crate::NAME)
                .join("state")
        } else if cfg!(target_vendor = "apple") {
            PathBuf::from("/Library/Application Support").join(crate::NAME)
        } else {
            PathBuf::from("/var/lib").join(crate::NAME)
        });
    }
    let base = etcetera::base_strategy::choose_native_strategy()?;
    let dir = base.state_dir().unwrap_or_else(|| if cfg!(windows) { base.cache_dir() } else { base.data_dir() });
    Ok(dir.join(crate::NAME))
}

#[cfg(test)]
mod tests {
    use super::ALL;

    #[cfg(all(feature = "desktop", feature = "self-update"))]
    /// The env file in `packaging`: every setting read from the environment, commented out, with its default.
    fn env_file() -> String {
        let of = ALL.iter().filter(|s| s.scope.at_start()).map(|s| {
            let mut lines = vec![String::from("#")];
            for word in s.help().unwrap().split_whitespace() {
                if lines.last().unwrap().len() + word.len() >= 100 {
                    lines.push("#".into());
                }
                *lines.last_mut().unwrap() += &format!(" {word}");
            }
            format!("\n{}\n#{}={}\n", lines.join("\n"), s.key, s.default.unwrap_or_default())
        });
        of.collect()
    }

    /// What the release ships, which is built with the default features.
    #[test]
    #[cfg(all(feature = "desktop", feature = "self-update"))]
    fn generated_env_file() {
        crate::tests::generated(
            file!(),
            &[(format!("packaging/{}.env", crate::NAME), &env_file), ("packaging/Dockerfile".into(), &dockerfile)],
        );
    }
    /// The container image's recipe: it fetches the release that dist built for the platform's Linux target, of the
    /// version that `--build-arg VERSION` names, and the rustic that the tests run.
    #[cfg(all(feature = "desktop", feature = "self-update"))]
    fn dockerfile() -> String {
        let listen = super::LISTEN.default.unwrap_or_default();
        let port = listen.rsplit_once(':').map_or(listen, |(_, port)| port);
        // Docker's name for each target's architecture.
        let cases: String = (crate::tests::dist_targets("linux").iter())
            .map(|target| {
                let arch = match target.split('-').next() {
                    Some("x86_64") => "amd64",
                    Some("aarch64") => "arm64",
                    other => other.unwrap_or_default(),
                };
                format!("      {arch}) triple={target} ;; \\\n")
            })
            .collect();
        format!(
            r#"ARG NAME={name}
ARG VERSION
ARG REPOSITORY={repository}
ARG RUSTIC=v{rustic}

FROM --platform=$BUILDPLATFORM alpine:latest AS fetch
ARG NAME
ARG VERSION
ARG REPOSITORY
ARG RUSTIC
ARG TARGETARCH
WORKDIR /work
RUN set -eu; \
    test -n "${{VERSION}}" || {{ echo "pass --build-arg VERSION=x.y.z" >&2; exit 1; }}; \
    case "${{TARGETARCH}}" in \
{cases}      *) echo "unsupported platform: ${{TARGETARCH}}" >&2; exit 1 ;; \
    esac; \
    fetch() {{ wget -q "$1/$2" "$1/$2.sha256"; echo "$(cut -d ' ' -f 1 "$2.sha256")  $2" | sha256sum -c -; }}; \
    fetch "https://github.com/${{REPOSITORY}}/releases/download/v${{VERSION#v}}" "${{NAME}}-${{triple}}.tar.gz"; \
    fetch "https://github.com/rustic-rs/rustic/releases/download/${{RUSTIC}}" "rustic-${{RUSTIC}}-${{triple}}.tar.gz"; \
    mkdir out; tar -xzf "${{NAME}}-${{triple}}.tar.gz" -C out; tar -xzf "rustic-${{RUSTIC}}-${{triple}}.tar.gz" -C out; \
    install -m 0755 "$(find out -type f -name "${{NAME}}" | head -n 1)" "/${{NAME}}"; \
    install -m 0755 "$(find out -type f -name rustic | head -n 1)" /rustic

FROM alpine:latest
ARG NAME
ARG VERSION
LABEL org.opencontainers.image.title="${{NAME}}" \
      org.opencontainers.image.description="{description}" \
      org.opencontainers.image.licenses="{license}" \
      org.opencontainers.image.version="${{VERSION}}"
RUN apk add --no-cache ca-certificates tzdata rclone openssh-client
COPY --from=fetch /${{NAME}} /rustic /usr/local/bin/
ENV {listen}=0.0.0.0:{port} \
    {state}=/var/lib/${{NAME}}
VOLUME ["/etc/rustic", "/var/lib/${{NAME}}"]
EXPOSE {port}
HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 CMD ["{name}", "healthcheck"]
ENTRYPOINT ["{name}"]
CMD ["serve"]
"#,
            name = crate::NAME,
            repository = env!("CARGO_PKG_REPOSITORY").trim_start_matches("https://github.com/"),
            rustic = *crate::rustic::TESTED,
            license = env!("CARGO_PKG_LICENSE"),
            description = env!("CARGO_PKG_DESCRIPTION"),
            listen = super::LISTEN.key,
            state = super::STATE_DIR.key,
        )
    }

    /// The examples reach the server where it listens by default: nginx's proxy, and the README's port.
    #[test]
    #[cfg(feature = "server")]
    #[expect(clippy::disallowed_methods, reason = "reads the examples")]
    fn examples_use_the_default_address() {
        let listen = super::LISTEN.default.unwrap_or_default();
        let read = |file: &str| std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file));
        let nginx = read("packaging/nginx.conf").unwrap();
        assert!(nginx.contains(&format!("proxy_pass http://{listen}")), "nginx.conf proxies elsewhere than {listen}");
        let port = listen.rsplit_once(':').map_or(listen, |(_, port)| port);
        assert!(read("README.md").unwrap().contains(&format!("port {port}")), "the README names another port");
    }

    /// Every setting has a description.
    #[test]
    fn every_setting_has_help() {
        for s in ALL {
            assert!(s.help().is_some(), "no message for {}", s.key);
        }
    }

    /// Every variable that the README and the screenshots' scripts name is read: a setting, or a test's.
    #[test]
    #[cfg(feature = "server")]
    #[expect(clippy::disallowed_methods, reason = "reads the documentation, the scripts and the tests")]
    fn documented_variables_exist() {
        fn files(path: &std::path::Path) -> Vec<std::path::PathBuf> {
            match std::fs::read_dir(path) {
                Ok(entries) => entries.flatten().flat_map(|e| files(&e.path())).collect(),
                Err(_) => vec![path.to_owned()],
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let read = |path: &std::path::Path| std::fs::read_to_string(path).unwrap_or_default();
        let tests: String = files(&root.join("tests/integration")).iter().map(|p| read(p)).collect();
        for doc in [root.join("README.md")].into_iter().chain(files(&root.join("screenshots/scripts"))) {
            let text = read(&doc);
            let names = text
                .split(|c: char| !(c.is_ascii_uppercase() || c == '_'))
                .filter(|w| w.starts_with(crate::ENV_PREFIX));
            for name in names {
                // Tests name their variables without the prefix.
                let test =
                    name.strip_prefix(crate::ENV_PREFIX).is_some_and(|key| tests.contains(&format!("\"{key}\"")));
                assert!(
                    ALL.iter().any(|s| s.key == name) || test,
                    "{} names {name}, which nothing reads",
                    doc.display()
                );
            }
        }
    }

    /// Writing a setting replaces its line, or the commented one of the packaged env file, and keeps every
    /// other line, so that installing twice neither repeats it nor loses what the user wrote.
    #[test]
    #[cfg(all(feature = "desktop", feature = "self-update"))]
    fn writing_a_setting_keeps_the_rest_of_the_env_file() {
        let write = |text| super::with_line(text, "RUSTICLONE_AUTO_UPDATE", "true");
        assert_eq!(write(""), "RUSTICLONE_AUTO_UPDATE=true\n");
        let user = "# mine\nRUSTICLONE_LISTEN=0.0.0.0:80\n";
        assert_eq!(write(user), format!("{user}RUSTICLONE_AUTO_UPDATE=true\n"));
        let set = "RUSTICLONE_AUTO_UPDATE = false\n# keep\nRUSTICLONE_AUTO_UPDATE=false\nRUSTICLONE_AUTO_UPDATES=1";
        assert_eq!(
            write(set),
            "RUSTICLONE_AUTO_UPDATE = false\n# keep\nRUSTICLONE_AUTO_UPDATE=true\nRUSTICLONE_AUTO_UPDATES=1\n"
        );
        assert_eq!(write("export RUSTICLONE_AUTO_UPDATE=\"false\""), "RUSTICLONE_AUTO_UPDATE=true\n");
        let packaged = env_file();
        let written = write(&packaged);
        assert_eq!(written.lines().count(), packaged.lines().count());
        assert_eq!(written, write(&written));
        assert!(written.contains("\nRUSTICLONE_AUTO_UPDATE=true\n") && !written.contains("#RUSTICLONE_AUTO_UPDATE="));
    }

    #[test]
    fn settings_are_sorted() {
        assert!(ALL.is_sorted_by_key(|s| s.key));
    }

    #[test]
    fn invalid_schedule_shows_the_expected_format() {
        let e = super::cron("every tuesday").err().map(|e| format!("{e:#}")).unwrap_or_default();
        assert!(e.contains("every tuesday is not") && e.contains("minute hour day month weekday"), "{e}");
        assert!(super::cron("never").is_err() && super::cron("0 3 * * 0").is_ok());
    }
}
