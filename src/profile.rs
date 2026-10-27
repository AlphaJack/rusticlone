//! Rustic profiles, read from `rustic show-config` so that rustic merges `use-profiles` itself.

#![expect(clippy::disallowed_methods, reason = "finds and resolves profiles on disk")]

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use parking_lot::RwLock;
use serde_json::{Map, Value};

use crate::sys;

/// A profile as `rustic show-config` prints it, merged with the profiles it uses.
pub(crate) type Config = Map<String, Value>;

#[derive(Debug, Clone)]
pub(crate) struct RusticProfile {
    pub name: String,
    /// Rustic runs here, so that it finds profiles that `use-profiles` names relative to it.
    pub dir: PathBuf,
    pub repository: Option<String>,
    /// Identifies the repository for locks: the canonical path of a local one, else its URL and
    /// options.
    key: String,
    local: Option<PathBuf>,
    pub sources: Vec<Sources>,
    pub targets: Vec<String>,
    pub has_keep: bool,
    /// `[forget] delete-unchanged`: rustic removes snapshots equal to the previous one.
    pub delete_unchanged: bool,
    /// `[forget] prune`: every forget also prunes, outside the prune schedule.
    pub prunes_on_forget: bool,
    /// `[global] group-by`, else rustic's default.
    pub group_by: String,
    /// `skip-if-unchanged`: a backup of unchanged sources makes no snapshot.
    pub skip_unchanged: bool,
    pub env: BTreeMap<String, String>,
    /// `[repository]` hooks run whenever rustic opens the repository, also as a copy's target; `[global]` ones
    /// only when it opens it with `-P`. Either may mount it.
    repository_hooks: bool,
    global_hooks: bool,
}

/// One `[[backup.snapshots]]` entry.
#[derive(Debug, Clone)]
pub(crate) struct Sources {
    #[expect(clippy::struct_field_names, reason = "named after rustic's key")]
    pub sources: Vec<String>,
    /// The paths rustic records in the snapshot, as derived from the profile.
    derived: Vec<String>,
    /// Its `host`, else `[backup] host`, else this computer's name.
    pub host: String,
    /// Its own or `[backup]` hooks run before it is backed up: they may mount it, and a restore runs none.
    pub hooked: bool,
    /// Its paths are derived as rustic would record them. Not when a source is missing, as before a hook
    /// creates it: rustic then records where it leads, such as a link's target.
    resolved: bool,
    /// What rustic recorded instead, once learned. Shared, so that a run's later steps see what its backup
    /// learned.
    learned: Arc<RwLock<Option<Vec<String>>>>,
}

/// A remote source (`opendal:`), which rustic backs up as the snapshot's root, recorded as `""`.
fn remote(sources: &[String]) -> bool {
    matches!(sources, [s] if s.starts_with("opendal:"))
}

impl Sources {
    #[cfg(all(test, feature = "server"))]
    pub(crate) fn new(sources: Vec<String>, paths: Vec<String>, host: String) -> Self {
        Self { sources, derived: paths, host, hooked: false, resolved: true, learned: Arc::default() }
    }

    /// The paths rustic records in the snapshot, which identify its group: learned, else derived.
    pub(crate) fn paths(&self) -> Vec<String> {
        self.learned.read().clone().unwrap_or_else(|| self.derived.clone())
    }

    /// Whether its paths must be learned from what rustic records: a hook creates the source.
    pub(crate) fn unknown(&self) -> bool {
        self.hooked && !self.resolved
    }

    pub(crate) fn learned(&self) -> bool {
        self.learned.read().is_some()
    }

    pub(crate) fn learn(&self, paths: Vec<String>) {
        *self.learned.write() = Some(paths);
    }

    /// Whether its snapshots have a place on this computer to restore to: stdin and remote sources have none.
    pub(crate) fn in_place(&self) -> bool {
        self.sources != ["-"] && !remote(&self.sources)
    }
}

impl RusticProfile {
    pub(crate) fn is_backup_profile(&self) -> bool {
        self.repository.is_some() && !self.sources.is_empty()
    }

    /// The snapshot paths of each `[[backup.snapshots]]` entry.
    pub(crate) fn groups(&self) -> impl Iterator<Item = Vec<String>> {
        self.sources.iter().map(Sources::paths)
    }

    /// [`Self::groups`], or only that of entry `source`.
    pub(crate) fn groups_of(&self, source: Option<usize>) -> Vec<Vec<String>> {
        self.groups().enumerate().filter(|(i, _)| source.is_none_or(|s| s == *i)).map(|(_, g)| g).collect()
    }

    /// Whether every backup makes a snapshot, so that the newest snapshot dates the last backup.
    pub(crate) fn keeps_unchanged(&self) -> bool {
        !self.delete_unchanged && !self.skip_unchanged
    }

    pub(crate) fn repo_key(&self) -> &str {
        &self.key
    }

    /// Whether listings may lag recent writes: true for remote repositories and for local paths on a
    /// network or FUSE mount.
    pub(crate) fn may_lag(&self) -> bool {
        self.local.as_deref().is_none_or(sys::on_network_mount)
    }

    /// The path of a local repository that is not on a network mount.
    pub(crate) fn on_disk(&self) -> Option<&Path> {
        self.local.as_deref().filter(|l| !sys::on_network_mount(l))
    }

    #[cfg(feature = "server")]
    pub(crate) fn is_remote(&self) -> bool {
        self.local.is_none()
    }

    /// Whether re-creating it from a copy lands where it was, not on another disk in place of an unplugged one:
    /// rustic creates every missing folder of its path, and runs only its `[repository]` hooks, which may
    /// mount it. A local one without them, only where its parent folder is.
    #[expect(clippy::disallowed_methods, reason = "whether its disk is there")]
    pub(crate) fn recreatable(&self) -> bool {
        self.repository_hooks || self.local.as_ref().is_none_or(|p| p.parent().is_some_and(Path::is_dir))
    }

    /// Whether a local repository has its `config` file. Always true for a remote one.
    pub(crate) fn exists(&self) -> bool {
        self.repository_hooks || self.global_hooks || self.local.as_ref().is_none_or(|p| p.join("config").is_file())
    }

    /// Its files in rustic's search path, then in the directory rusticlone was started in, which a user may
    /// expect it to read. Rustic reads the first one only.
    pub(crate) fn files(&self) -> Vec<PathBuf> {
        let mut dirs = search_path(Some(self.dir.clone())).unwrap_or_default();
        for dir in profile_dirs().unwrap_or_default() {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        dirs.into_iter().map(|d| d.join(format!("{}.toml", self.name))).filter(|f| f.is_file()).collect()
    }
}

/// The path of a local repository (a path, `local:`, or `opendal:fs` with `root`), relative to `dir`.
fn local_repo(repository: &str, options: &BTreeMap<String, String>, dir: &Path) -> Option<PathBuf> {
    let path = match repository {
        "opendal:fs" => options.get("root")?.as_str(),
        r if ["rclone:", "rest:", "opendal:"].iter().any(|backend| r.starts_with(backend)) => return None,
        r => r.strip_prefix("local:").unwrap_or(r),
    };
    Some(resolve(&dir.join(path)))
}

/// Rustic's profile directories in its search order, each once: user, system, then `cwd`.
fn search_path(cwd: Option<PathBuf>) -> Result<Vec<PathBuf>> {
    let mut seen = BTreeSet::new();
    Ok(sys::config_dirs("rustic")?.into_iter().chain(cwd).filter(|d| seen.insert(d.clone())).collect())
}

/// The directory profiles are saved in: the system's for root or an elevated Windows process, else the
/// user's.
pub(crate) fn own_dir() -> Option<PathBuf> {
    sys::own_config_dir("rustic")
}

/// Directories scanned for profiles, with the working directory, from which a disaster recovery may
/// run with the profiles it kept. The system's directory, `/etc/rustic` or `ProgramData`, is left out
/// for other users, so that they do not all run the system service's profiles.
pub(crate) fn profile_dirs() -> Result<Vec<PathBuf>> {
    let system = sys::config_dirs("rustic")?.into_iter().nth(1);
    let mut dirs = search_path(env::current_dir().ok())?;
    dirs.retain(|d| sys::is_root() || Some(d) != system.as_ref());
    Ok(dirs)
}

/// Profile names and their directories. Like rustic, the first directory wins.
pub(crate) fn profile_files() -> Result<Vec<(String, PathBuf)>> {
    let mut found = BTreeMap::new();
    for dir in profile_dirs()? {
        for path in fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()) {
            if path.extension().is_some_and(|e| e == "toml")
                && path.is_file()
                && let Some(name) = path.file_stem().and_then(|s| s.to_str())
            {
                found.entry(name.to_owned()).or_insert(dir.clone());
            }
        }
    }
    Ok(found.into_iter().collect())
}

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "profile-error")]
pub(crate) enum Error {
    #[fluent(dirs = display(dirs))]
    NotFound {
        name: String,
        dirs: Vec<PathBuf>,
    },
    DryRun {
        name: String,
    },
    #[fluent(native = native_drive(location))]
    SlashedDrive {
        name: String,
        location: String,
    },
    BackslashAfterScheme {
        name: String,
        location: String,
        scheme: String,
    },
}

/// A Windows drive path with forward slashes, `C:/a`, which rustic reads as the backend `C`.
fn slashed_drive(path: &str) -> bool {
    matches!(path.as_bytes(), [drive, b':', b'/', ..] if drive.is_ascii_alphabetic())
}

/// The scheme of a repository that also holds a backslash, which rustic on Windows reads whole as a local
/// path, `local:` included. Elsewhere a backslash is a character of a name.
fn backslash_after_scheme(repository: &str, windows: bool) -> Option<&str> {
    let scheme =
        ["local", "rclone", "rest", "opendal"].into_iter().find(|s| repository.starts_with(&format!("{s}:")))?;
    (windows && repository.contains('\\')).then_some(scheme)
}

/// `path`, with the forward slashes of a Windows drive path as backslashes, which rustic reads as a path.
pub(crate) fn native_drive(path: &str) -> String {
    if slashed_drive(path) { path.replace('/', "\\") } else { path.to_owned() }
}

/// Finds a profile named on the command line, as rustic would.
pub(crate) fn locate(name: &str) -> Result<PathBuf> {
    let dirs = search_path(env::current_dir().ok())?;
    let not_found = Error::NotFound { name: name.to_owned(), dirs: dirs.clone() };
    Ok(dirs.into_iter().find(|d| d.join(format!("{name}.toml")).is_file()).ok_or(not_found)?)
}

pub(crate) fn dirs_display() -> String {
    display(&profile_dirs().unwrap_or_default())
}

fn display(dirs: &[PathBuf]) -> String {
    dirs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
}

/// Rejects `dry-run = true`, which reports success and saves nothing, and a repository that rustic would
/// read as a backend named after a drive.
pub(crate) fn load(name: &str, dir: &Path, config: &Config) -> Result<RusticProfile, Error> {
    if set(config, &["global", "dry-run"]) {
        return Err(Error::DryRun { name: name.to_owned() });
    }
    if let Some(repository) = value(config, &["repository", "repository"]).and_then(Value::as_str)
        && slashed_drive(repository)
    {
        return Err(Error::SlashedDrive { name: name.to_owned(), location: repository.to_owned() });
    }
    if let Some(repository) = value(config, &["repository", "repository"]).and_then(Value::as_str)
        && let Some(scheme) = backslash_after_scheme(repository, cfg!(windows))
    {
        let (name, location, scheme) = (name.to_owned(), repository.to_owned(), scheme.to_owned());
        return Err(Error::BackslashAfterScheme { name, location, scheme });
    }
    Ok(from_config(name, dir, config))
}

#[cfg(test)]
pub(crate) fn parse(name: &str, config: &str) -> RusticProfile {
    from_config(name, Path::new("/profiles"), &toml_edit::de::from_str(config).unwrap())
}

fn value<'a>(table: &'a Config, path: &[&str]) -> Option<&'a Value> {
    let (last, parents) = path.split_last()?;
    parents.iter().try_fold(table, |t, key| t.get(*key)?.as_object())?.get(*last)
}

/// Whether the hooks table at `path` runs commands before rustic does its work.
fn runs_before(table: &Config, path: &[&str]) -> bool {
    value(table, &[path, &["run-before"]].concat()).and_then(Value::as_array).is_some_and(|a| !a.is_empty())
}

fn set(table: &Config, path: &[&str]) -> bool {
    value(table, path).and_then(Value::as_bool) == Some(true)
}

fn from_config(name: &str, dir: &Path, config: &Config) -> RusticProfile {
    let get = |path: &[&str]| value(config, path);
    let strings = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(Into::into).collect()
    };
    let table = |path: &[&str]| get(path).and_then(Value::as_object).into_iter().flatten();

    let snapshots: Vec<&Config> = get(&["backup", "snapshots"])
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .collect();
    let sources: Vec<Sources> = snapshots
        .iter()
        .map(|s| {
            let sources = strings(s.get("sources"));
            let mut resolved = true;
            let paths = match s.get("as-path").and_then(Value::as_str) {
                Some(as_path) => vec![as_path.to_owned()],
                None if sources == ["-"] => vec![
                    s.get("stdin-filename")
                        .or_else(|| get(&["backup", "stdin-filename"]))
                        .and_then(Value::as_str)
                        .unwrap_or("stdin")
                        .to_owned(),
                ],
                None if remote(&sources) => vec![String::new()],
                None => {
                    resolved = sources.iter().all(|s| dir.join(s).exists());
                    snapshot_paths(&sources, dir)
                }
            };
            let host = s.get("host").or_else(|| get(&["backup", "host"])).and_then(Value::as_str);
            let host = host.unwrap_or_else(|| sys::hostname()).to_owned();
            let hooked = runs_before(s, &["hooks"]) || runs_before(config, &["backup", "hooks"]);
            Sources { sources, derived: paths, host, hooked, resolved, learned: Arc::default() }
        })
        .filter(|s| !s.derived.is_empty())
        .collect();
    let has_keep = table(&["forget"]).any(|(k, v)| {
        k.starts_with("keep-") && !matches!(v, Value::Bool(false)) && !v.as_array().is_some_and(Vec::is_empty)
    });
    let repository = get(&["repository", "repository"]).and_then(Value::as_str);
    // Options also identify the repository: a bucket, the `root` of `opendal:fs`.
    let options: BTreeMap<String, String> = ["options", "options-cold"]
        .iter()
        .flat_map(|t| table(&["repository", t]))
        .map(|(k, v)| (k.clone(), v.as_str().map_or_else(|| v.to_string(), str::to_owned)))
        .collect();
    let local = repository.and_then(|r| local_repo(r, &options, dir));
    RusticProfile {
        name: name.to_owned(),
        dir: dir.to_owned(),
        key: match (repository, &local) {
            (_, Some(path)) => path.display().to_string(),
            (Some(r), None) => {
                let options = options.iter().map(|(k, v)| format!("{k}={v}"));
                std::iter::once(r.trim_end_matches('/').to_owned()).chain(options).collect::<Vec<_>>().join(" ")
            }
            (None, None) => name.to_owned(),
        },
        local,
        repository: repository.map(Into::into),
        sources,
        targets: strings(get(&["copy", "targets"])),
        has_keep,
        delete_unchanged: set(config, &["forget", "delete-unchanged"]),
        prunes_on_forget: set(config, &["forget", "prune"]),
        group_by: get(&["global", "group-by"]).and_then(Value::as_str).unwrap_or("host,label,paths").to_owned(),
        skip_unchanged: set(config, &["backup", "skip-if-unchanged"])
            || snapshots.iter().any(|s| set(s, &["skip-if-unchanged"])),
        env: table(&["global", "env"]).filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect(),
        repository_hooks: runs_before(config, &["repository", "hooks"]),
        global_hooks: runs_before(config, &["global", "hooks"]),
    }
}

/// The paths rustic records for `sources`: canonicalized and sorted. Missing sources (before a
/// restore) can only be normalized.
fn snapshot_paths(sources: &[String], dir: &Path) -> Vec<String> {
    let normalized: Vec<PathBuf> = sources.iter().map(|s| normalize(Path::new(s))).collect();
    // Rustic keeps paths relative unless one is absolute or outside its working directory.
    let absolute = normalized.iter().any(|p| p.is_absolute() || p.starts_with(".."));
    let mut paths: Vec<String> = normalized
        .into_iter()
        .map(|path| {
            let path = if absolute { resolve(&dir.join(&path)) } else { path };
            path.to_string_lossy().into_owned()
        })
        .collect();
    paths.sort_unstable();
    paths.dedup();
    paths
}

/// Removes `.`, `..`, and duplicate or trailing separators, without reading the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => _ = out.pop(),
                Some(Component::RootDir) => {}
                _ => out.push(".."),
            },
            c @ (Component::Prefix(_) | Component::RootDir | Component::Normal(_)) => out.push(c),
        }
    }
    out
}

/// Canonicalizes the existing part of `path` and appends the rest unchanged.
fn resolve(path: &Path) -> PathBuf {
    let path = normalize(path);
    let existing = path.ancestors().find_map(|a| {
        let (real, rest) = (dunce::canonicalize(a).ok()?, path.strip_prefix(a).ok()?);
        Some(if rest.as_os_str().is_empty() { real } else { real.join(rest) })
    });
    existing.unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_paths() {
        assert_eq!(normalize(Path::new("/a/./b//c/../d/")), PathBuf::from("/a/b/d"));
        assert_eq!(normalize(Path::new("../../a/../b")), PathBuf::from("../../b"));
        assert_eq!(normalize(Path::new("/../a")), PathBuf::from("/a"));
    }

    /// Rustic reads `C:/b` as the backend `C`, and `C:\b` or `local:C:/b` as paths.
    #[test]
    fn a_drive_path_with_slashes_is_refused() {
        let loaded = |repository: &str| {
            let config = toml_edit::de::from_str(&format!("[repository]\nrepository = '{repository}'\n")).unwrap();
            load("Docs", Path::new("/p"), &config)
        };
        assert!(matches!(loaded("C:/b"), Err(Error::SlashedDrive { .. })));
        assert!(loaded(r"C:\b").is_ok() && loaded("local:C:/b").is_ok() && loaded("/b").is_ok());
        assert_eq!(native_drive("D:/a/b"), r"D:\a\b");
        assert_eq!(backslash_after_scheme(r"local:C:\b", true), Some("local"));
        assert_eq!(backslash_after_scheme(r"rclone:remote:a\b", true), Some("rclone"));
        assert_eq!(backslash_after_scheme(r"rclone:remote:a\b", false), None);
        assert_eq!(backslash_after_scheme(r"C:\b", true), None);
        assert_eq!(native_drive("rest:http://h/r"), "rest:http://h/r");
    }

    #[test]
    fn snapshot_paths_match_rustic() {
        let dir = sys::test_dir("paths");
        fs::create_dir_all(dir.join("a/b/sub")).unwrap();
        fs::create_dir_all(dir.join("a/other")).unwrap();
        let (b, dir) = (dir.join("a/b"), dunce::canonicalize(&dir).unwrap());
        let paths = |sources: &[&str]| snapshot_paths(&sources.iter().map(ToString::to_string).collect::<Vec<_>>(), &b);
        // With the system's separators, as rustic records them.
        let abs = |p: &str| p.split('/').fold(dir.clone(), |d, c| d.join(c)).to_string_lossy().into_owned();
        assert_eq!(paths(&["sub"]), ["sub"]);
        assert_eq!(paths(&["./sub"]), ["sub"]);
        assert_eq!(paths(&["../other"]), [abs("a/other")]);
        assert_eq!(paths(&["sub", "../other"]), [abs("a/b/sub"), abs("a/other")]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn classifies_show_config_output() {
        let config: Config = toml_edit::de::from_str(&format!(
            r#"
            [global.env]
            {} = "@hourly"
            [repository]
            repository = "/backup/rustic/Pictures"
            [[backup.snapshots]]
            sources = ["/nonexistent/pics/"]
            [[backup.snapshots]]
            sources = ["/nonexistent/b", "/nonexistent/a"]
            [[backup.snapshots]]
            sources = ["/srv/new-name"]
            as-path = "/srv/old-name"
            [copy]
            targets = ["Pictures-offsite"]
            [forget]
            prune = true
            keep-daily = 7
            "#,
            crate::settings::BACKUP.key
        ))
        .unwrap();
        let p = from_config("Pictures", Path::new("/"), &config);
        assert!(p.is_backup_profile() && p.has_keep);
        assert_eq!(p.targets, ["Pictures-offsite"]);
        assert_eq!(p.env[crate::settings::BACKUP.key], "@hourly");
        // Sources with the system's separators, as rustic records them; `as-path` as written.
        let native = |p: &str| p.replace('/', std::path::MAIN_SEPARATOR_STR);
        assert_eq!(
            p.groups().collect::<Vec<_>>(),
            [
                &[native("/nonexistent/pics")][..],
                &[native("/nonexistent/a"), native("/nonexistent/b")],
                &["/srv/old-name".to_owned()],
            ]
        );

        assert_eq!(p.sources[1].sources, ["/nonexistent/b", "/nonexistent/a"]);

        let target: Config =
            toml_edit::de::from_str("[repository]\nrepository = \"rclone:x:y\"\n[forget]\nprune = true").unwrap();
        let t = from_config("Pictures-offsite", Path::new("/"), &target);
        assert!(!t.is_backup_profile() && !t.has_keep);
        assert!(t.prunes_on_forget);
        assert_eq!(t.repo_key(), "rclone:x:y");
        // Different spellings of one repository have the same key.
        let at = |repository: &str| {
            let config: Config =
                toml_edit::de::from_str(&format!("[repository]\nrepository = '{repository}'")).unwrap();
            from_config("R", Path::new("/nonexistent"), &config).repo_key().to_owned()
        };
        assert_eq!(at("local:/nonexistent/r/"), at("/nonexistent/./r"));
        assert_eq!(at("r"), at("/nonexistent/r"), "relative to where rustic runs");
        let with = |repository: &str, options: &str| {
            let config = format!("[repository]\nrepository = '{repository}'\n[repository.options]\n{options}");
            from_config("R", Path::new("/"), &toml_edit::de::from_str(&config).unwrap()).repo_key().to_owned()
        };
        assert_eq!(with("opendal:fs", "root = '/nonexistent/r'"), at("/nonexistent/r"));
        assert_ne!(with("opendal:s3", "bucket = 'a'"), with("opendal:s3", "bucket = 'b'"));
    }
}
