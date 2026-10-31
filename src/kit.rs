//! The emergency kit: a zip of every profile to keep away from this computer. Its `info.txt` says what it is and
//! how to restore from it in the reader's language, for people only. Any zip or folder of profiles is a kit:
//! `--kit` puts its `.toml` files where rustic finds them.

use std::io::{Read, Seek};

use anyhow::Result;
use toml_edit::DocumentMut;

/// The kit's name. Every download has the same one.
#[cfg(feature = "server")]
pub(crate) const FILE: &str = "restore-kit.zip";
/// What the kit says about itself, for whoever opens it.
#[cfg(feature = "server")]
const INFO: &str = "info.txt";
/// Far more files and far larger ones than a kit holds. A zip made to fill the memory as it is read (a zip bomb)
/// is refused before that.
const MOST_FILES: usize = 256;
const LARGEST_FILE: u64 = 1024 * 1024;

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "kit-error")]
pub(crate) enum Error {
    /// A file name that is not a profile's: `../x.toml`.
    Name {
        name: String,
    },
    Empty,
    /// Neither a zip nor a folder.
    NotAKit(#[source] zip::result::ZipError),
    /// Two profiles of that name, in different folders of the zip.
    Twice {
        name: String,
    },
    /// More files than [`MOST_FILES`].
    TooManyFiles,
    /// Larger than [`LARGEST_FILE`] once decompressed.
    TooLarge {
        name: String,
    },
    /// Saved there already with other contents.
    #[fluent(path = path.display())]
    Differs {
        path: std::path::PathBuf,
    },
}

/// What a profile's text says: its repository, the folders it backs up and the profiles it copies to.
#[cfg(feature = "server")]
struct Profile {
    repository: String,
    sources: Vec<String>,
    targets: Vec<String>,
}

#[cfg(feature = "server")]
impl Profile {
    fn of(text: &str) -> Self {
        let doc = text.parse::<DocumentMut>().unwrap_or_default();
        let strings = |item: Option<&toml_edit::Item>| -> Vec<String> {
            let values = item.and_then(toml_edit::Item::as_array).into_iter().flatten();
            values.filter_map(|v| v.as_str()).map(ToOwned::to_owned).collect()
        };
        let snapshots = doc.get("backup").and_then(|b| b.get("snapshots")).and_then(|s| s.as_array_of_tables());
        Self {
            repository: doc
                .get("repository")
                .and_then(|r| r.get("repository"))
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .into(),
            sources: snapshots.into_iter().flatten().flat_map(|s| strings(s.get("sources"))).collect(),
            targets: strings(doc.get("copy").and_then(|c| c.get("targets"))),
        }
    }
}

/// The collections of `profiles` by file name: the profiles that back something up.
#[cfg(feature = "server")]
pub(crate) fn primaries(profiles: &[(String, String)]) -> Vec<String> {
    let backs_up = |(_, text): &&(String, String)| !Profile::of(text).sources.is_empty();
    profiles.iter().filter(backs_up).map(|(file, _)| file.trim_end_matches(".toml").to_owned()).collect()
}

/// `items` as a sentence lists them: `a`, `a and b`, `a, b and c`, in the reader's language.
#[cfg(feature = "server")]
fn listed(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => crate::i18n::t!("kit-list", rest = rest.join(", "), last = last),
    }
}

/// How each collection of `profiles` restores: its name, from which repository into which folders, and which
/// repositories it falls back to. Profiles that back up nothing come only as targets.
#[cfg(feature = "server")]
fn collections(profiles: &[(String, String)]) -> String {
    use crate::i18n::t;
    let names: Vec<String> = profiles.iter().map(|(f, _)| f.trim_end_matches(".toml").to_owned()).collect();
    let parsed: Vec<Profile> = profiles.iter().map(|(_, text)| Profile::of(text)).collect();
    let repository = |name: &str| names.iter().position(|n| n == name).map(|i| parsed[i].repository.clone());
    let mut text = String::new();
    for (name, primary) in names.iter().zip(&parsed).filter(|(_, p)| !p.sources.is_empty()) {
        let title = crate::engine::title(name, &primary.targets, &names);
        let mut from: Vec<String> = vec![primary.repository.clone()];
        from.extend(primary.targets.iter().filter_map(|t| repository(t)));
        text.push_str(&format!("\n{title}:\n"));
        text.push_str(&t!("kit-restores", from = from[0].as_str(), to = listed(&primary.sources)));
        for (i, pair) in from.windows(2).enumerate() {
            let (missing, from) = (pair[0].as_str(), pair[1].as_str());
            let line = match i {
                0 => t!("kit-falls-back", missing = missing, from = from),
                _ => t!("kit-falls-back-again", missing = missing, from = from),
            };
            text.push_str(&format!("\n{line}"));
        }
        text.push('\n');
    }
    text
}

/// Where and by whom a kit was made. Root's commands need `sudo` again on Unix.
#[cfg(feature = "server")]
pub(crate) struct Made<'a> {
    pub host: &'a str,
    pub time: &'a str,
    pub user: &'a str,
    pub root: bool,
}

/// `info.txt` for `profiles` by file name and text.
#[cfg(feature = "server")]
fn info(profiles: &[(String, String)], made: &Made<'_>) -> String {
    use crate::args::command_line;
    let sudo = if made.root && cfg!(unix) { "sudo " } else { "" };
    let restore = format!("{sudo}{} --kit {FILE}", command_line("restore", &[("all", None)]));
    let resume = format!("{sudo}{}", command_line("install", &[("auto_update", None)]));
    let mut info = String::new();
    let url = concat!(env!("CARGO_PKG_REPOSITORY"), "#restore");
    let (host, time, user) = (made.host, made.time, made.user);
    info.push_str(&crate::i18n::t!(
        "kit-header",
        host = host,
        time = time,
        user = user,
        url = url,
        restore = restore,
        resume = resume
    ));
    info.push_str("\n\n");
    info.push_str(&crate::i18n::t!("kit-collections"));
    info.push('\n');
    info.push_str(&collections(profiles));
    info
}

/// The kit of `profiles` by file name and text as zip bytes.
#[cfg(feature = "server")]
pub(crate) fn render(profiles: &[(String, String)], made: &Made<'_>) -> Result<Vec<u8>> {
    use std::io::Write;
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file(INFO, options)?;
    zip.write_all(info(profiles, made).as_bytes())?;
    for (file, text) in profiles {
        zip.start_file(file, options)?;
        zip.write_all(text.as_bytes())?;
    }
    Ok(zip.finish()?.into_inner())
}

/// A file as text. Refused if it holds more than [`LARGEST_FILE`] whatever size it claims.
fn text(name: &str, claimed: u64, file: impl Read) -> Result<String> {
    let too_large = || Error::TooLarge { name: name.to_owned() };
    if claimed > LARGEST_FILE {
        return Err(too_large().into());
    }
    let mut text = String::new();
    file.take(LARGEST_FILE + 1).read_to_string(&mut text)?;
    if text.len() as u64 > LARGEST_FILE {
        return Err(too_large().into());
    }
    Ok(text)
}

/// Whether the zip's entry at `path` is a profile: a `.toml` file, in any folder but the hidden ones that
/// archivers add, as macOS's `__MACOSX/._Docs.toml`.
fn is_profile(path: &std::path::Path) -> bool {
    let hidden = path.iter().any(|part| part.to_string_lossy().starts_with('.') || part == "__MACOSX");
    !hidden && path.extension().is_some_and(|e| e == "toml")
}

/// The profiles of a zip by file name: every `.toml` file, at the top or in a folder. The rest, as `info.txt`,
/// is for people.
pub(crate) fn parse(kit: impl Read + Seek) -> Result<Vec<(String, String)>> {
    let mut zip = zip::ZipArchive::new(kit).map_err(Error::NotAKit)?;
    if zip.len() > MOST_FILES {
        return Err(Error::TooManyFiles.into());
    }
    let mut profiles: Vec<(String, String)> = Vec::new();
    for i in 0..zip.len() {
        let file = zip.by_index(i)?;
        let raw = file.name()?.into_owned();
        // A path that leaves the zip's folder, as `../x.toml`.
        let Some(path) = file.enclosed_name() else { return Err(Error::Name { name: raw }.into()) };
        if file.is_dir() || !is_profile(&path) {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if profiles.iter().any(|(n, _)| *n == name) {
            return Err(Error::Twice { name }.into());
        }
        let text = text(&raw, file.size(), file)?;
        profiles.push((name, text));
    }
    Ok(profiles)
}

/// The profiles of a folder by file name: its `.toml` files, not those of its subfolders.
#[expect(clippy::disallowed_methods, reason = "reads the folder that `--kit` names")]
fn read_folder(dir: &std::path::Path) -> Result<Vec<(String, String)>> {
    let mut profiles = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if !path.is_file() || !is_profile(std::path::Path::new(path.file_name().unwrap_or_default())) {
            continue;
        }
        if profiles.len() == MOST_FILES {
            return Err(Error::TooManyFiles.into());
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let claimed = std::fs::metadata(&path)?.len();
        profiles.push((name.clone(), text(&name, claimed, std::fs::File::open(&path)?)?));
    }
    profiles.sort();
    Ok(profiles)
}

/// [`unpack`] of the kit at `kit`: a zip, or a folder of profiles.
#[expect(clippy::disallowed_methods, reason = "reads the kit that `--kit` names")]
pub(crate) fn unpack_path(kit: &std::path::Path, dir: &std::path::Path) -> Result<usize> {
    let profiles = if kit.is_dir() { read_folder(kit)? } else { parse(std::fs::File::open(kit)?)? };
    unpack(profiles, dir)
}

/// Saves the kit's profiles that are not in `dir` yet and returns how many. Never overwrites a profile that
/// differs: rustic would read the one in `dir`.
#[expect(clippy::disallowed_methods, reason = "saves the kit's profiles")]
pub(crate) fn unpack(profiles: Vec<(String, String)>, dir: &std::path::Path) -> Result<usize> {
    if profiles.is_empty() {
        return Err(Error::Empty.into());
    }
    for (name, text) in &profiles {
        let stem = name.strip_suffix(".toml").unwrap_or_default();
        if stem.is_empty() || !stem.chars().all(|c| c.is_alphanumeric() || "-_.".contains(c)) {
            return Err(Error::Name { name: name.clone() }.into());
        }
        text.parse::<DocumentMut>()?;
        let path = dir.join(name);
        if std::fs::read_to_string(&path).is_ok_and(|old| old != *text) {
            return Err(Error::Differs { path }.into());
        }
    }
    std::fs::create_dir_all(dir)?;
    let mut saved = 0;
    for (name, text) in profiles {
        let path = dir.join(name);
        if !path.exists() {
            std::fs::write(path, text)?;
            saved += 1;
        }
    }
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCS: &str = "[[backup.snapshots]]\nsources = [\"/home/me/Documents\"]\n\n[repository]\nrepository = \"/mnt/usb/Docs\"\npassword = \"pw\"\n";
    /// Three folders copied to two other repositories, which a restore falls back to in turn.
    #[cfg(feature = "server")]
    const PICTURES: &str = "[[backup.snapshots]]\nsources = [\"/home/me/Pictures\", \"/home/me/Videos\"]\n\n\
        [[backup.snapshots]]\nsources = [\"/home/me/Music\"]\n\n[repository]\nrepository = \"/mnt/usb/Pictures\"\n\
        password = \"pw\"\n\n[copy]\ntargets = [\"Pictures-nas\", \"Pictures-gd\"]\n";
    #[cfg(feature = "server")]
    const COPY: &str = "[repository]\nrepository = \"rclone:gd:Pictures\"\npassword = \"pw\"\n";
    #[cfg(feature = "server")]
    const NAS: &str = "[repository]\nrepository = \"/Volumes/nas/Pictures\"\npassword = \"pw\"\n";

    /// A zip with these files.
    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, text) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// What users keep. A change here is a change to every kit downloaded from now on.
    #[cfg(feature = "server")]
    #[test]
    fn a_kit_says_what_it_holds_and_reads_back_its_profiles() {
        let profiles: Vec<(String, String)> = [
            ("Docs.toml", DOCS),
            ("Pictures-usb.toml", PICTURES),
            ("Pictures-gd.toml", COPY),
            ("Pictures-nas.toml", NAS),
        ]
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect();
        let made = Made { host: "laptop", time: "2026-10-30 12:00", user: "me", root: false };
        insta::assert_snapshot!("kit_info", info(&profiles, &made));
        let root = info(&profiles, &Made { user: "root", root: true, ..made });
        assert_eq!(
            root.contains("sudo rusticlone restore") && root.contains("sudo rusticlone install"),
            cfg!(unix),
            "{root}"
        );
        let kit = render(&profiles, &made).unwrap();
        assert_eq!(parse(std::io::Cursor::new(kit)).unwrap(), profiles);
    }

    /// Any zip of profiles is a kit, as one zipped by hand from a folder: `info.txt` is for people.
    #[test]
    fn any_zip_of_profiles_is_a_kit() {
        let parse = |files: &[(&str, &str)]| parse(std::io::Cursor::new(zip_of(files)));
        let names = |files| parse(files).unwrap().into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(names(&[("Docs.toml", DOCS)]), ["Docs.toml"]);
        assert_eq!(
            names(&[("rustic/Docs.toml", DOCS), ("info.txt", "notes"), ("__MACOSX/rustic/._Docs.toml", "\0")]),
            ["Docs.toml"]
        );
        let twice = parse(&[("a/Docs.toml", DOCS), ("b/Docs.toml", DOCS)]).unwrap_err();
        assert!(matches!(twice.downcast_ref(), Some(Error::Twice { .. })), "{twice}");
        let other = parse(&[("notes.txt", "")]).unwrap();
        assert!(other.is_empty());
    }

    /// A folder of profiles is a kit too: its `.toml` files, not its subfolders'.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "lays out a folder kit")]
    fn a_folder_of_profiles_is_a_kit() {
        let dir = crate::sys::test_dir("kit-folder");
        let kit = dir.join("kit");
        std::fs::create_dir_all(kit.join("old")).unwrap();
        std::fs::write(kit.join("Docs.toml"), DOCS).unwrap();
        std::fs::write(kit.join("old/Docs-usb.toml"), DOCS).unwrap();
        std::fs::write(kit.join("info.txt"), "notes").unwrap();
        let profiles = dir.join("profiles");
        assert_eq!(unpack_path(&kit, &profiles).unwrap(), 1);
        assert!(profiles.join("Docs.toml").is_file() && !profiles.join("Docs-usb.toml").exists());
    }

    /// A small zip that decompresses to more than any profile is refused. So is one with too many files.
    #[test]
    fn a_zip_bomb_is_refused() {
        let huge = " ".repeat(usize::try_from(LARGEST_FILE).unwrap() + 1);
        let bomb = zip_of(&[("Docs.toml", &huge)]);
        assert!(bomb.len() < 100_000, "deflate shrinks it: {}", bomb.len());
        let refused = parse(std::io::Cursor::new(bomb)).unwrap_err();
        assert!(matches!(refused.downcast_ref(), Some(Error::TooLarge { .. })), "{refused}");
        let names: Vec<String> = (0..=MOST_FILES).map(|i| format!("{i}.toml")).collect();
        let files: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), DOCS)).collect();
        let refused = parse(std::io::Cursor::new(zip_of(&files))).unwrap_err();
        assert!(matches!(refused.downcast_ref(), Some(Error::TooManyFiles)), "{refused}");
    }

    #[test]
    #[expect(clippy::disallowed_methods, reason = "lays out a kit and a profile folder")]
    fn unpacking_never_overwrites_a_different_profile() {
        let dir = crate::sys::test_dir("kit");
        let kit = || parse(std::io::Cursor::new(zip_of(&[("Docs.toml", DOCS)]))).unwrap();
        let profiles = dir.join("profiles");
        assert_eq!(unpack(kit(), &profiles).unwrap(), 1);
        assert_eq!(unpack(kit(), &profiles).unwrap(), 0, "the same profile is there already");
        std::fs::write(profiles.join("Docs.toml"), "[repository]\n").unwrap();
        assert!(unpack(kit(), &profiles).is_err());
        let escaping = parse(std::io::Cursor::new(zip_of(&[("../x.toml", DOCS)]))).unwrap_err();
        assert!(matches!(escaping.downcast_ref(), Some(Error::Name { .. })), "{escaping}");
    }
}
