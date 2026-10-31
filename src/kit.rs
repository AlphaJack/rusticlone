//! The emergency kit: a zip of every profile to keep away from this computer. Its `info.txt` says what it is and
//! how to restore from it in the reader's language. The first line is never translated: it names the kit's
//! layout so that a later release still reads the kits of this one. `--kit` puts its profiles where rustic
//! finds them.

use std::io::{Read, Seek};

use anyhow::Result;
use toml_edit::DocumentMut;

/// The kit's name. Every download has the same one.
#[cfg(feature = "server")]
pub(crate) const FILE: &str = "restore-kit.zip";
/// What the kit says about itself.
const INFO: &str = "info.txt";
/// The start of `info.txt`'s first line before the layout's number: `restore-kit v1`.
const VERSION_MARK: &str = "restore-kit";
/// The layout this release writes: `info.txt` and each profile as `<name>.toml` at the top of the zip.
#[cfg(feature = "server")]
const VERSION: u32 = 1;
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
    /// Not a zip or one without `info.txt`.
    NotAKit(#[source] zip::result::ZipError),
    /// An `info.txt` that names no layout.
    NoLayout,
    /// Written by a later release in a layout this one cannot read.
    Newer {
        version: u32,
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
    let mut info = format!("{VERSION_MARK} v{VERSION}\n\n");
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

/// A file of the zip as text. Refused if it decompresses to more than [`LARGEST_FILE`] whatever size it claims.
fn text(file: zip::read::ZipFile<'_, impl Read>) -> Result<String> {
    let name = file.name()?.into_owned();
    if file.size() > LARGEST_FILE {
        return Err(Error::TooLarge { name }.into());
    }
    let mut text = String::new();
    file.take(LARGEST_FILE + 1).read_to_string(&mut text)?;
    if text.len() as u64 > LARGEST_FILE {
        return Err(Error::TooLarge { name }.into());
    }
    Ok(text)
}

/// The profiles of a kit by file name in the layout its `info.txt` names.
pub(crate) fn parse(kit: impl Read + Seek) -> Result<Vec<(String, String)>> {
    let mut zip = zip::ZipArchive::new(kit).map_err(Error::NotAKit)?;
    if zip.len() > MOST_FILES {
        return Err(Error::TooManyFiles.into());
    }
    let info = text(zip.by_name(INFO).map_err(Error::NotAKit)?)?;
    let version =
        info.lines().next().and_then(|l| l.trim().strip_prefix(VERSION_MARK)?.trim().strip_prefix('v')?.parse().ok());
    match version.ok_or(Error::NoLayout)? {
        1 => parse_v1(&mut zip),
        version => Err(Error::Newer { version }.into()),
    }
}

/// Each `.toml` file of the zip.
fn parse_v1(zip: &mut zip::ZipArchive<impl Read + Seek>) -> Result<Vec<(String, String)>> {
    let mut profiles = Vec::new();
    for i in 0..zip.len() {
        let file = zip.by_index(i)?;
        let name = file.name()?.into_owned();
        if std::path::Path::new(&name).extension().is_none_or(|e| e != "toml") {
            continue;
        }
        profiles.push((name, text(file)?));
    }
    Ok(profiles)
}

/// Saves the kit's profiles that are not in `dir` yet and returns how many. Never overwrites a profile that
/// differs: rustic would read the one in `dir`.
#[expect(clippy::disallowed_methods, reason = "saves the kit's profiles")]
pub(crate) fn unpack(kit: &std::path::Path, dir: &std::path::Path) -> Result<usize> {
    let profiles = parse(std::fs::File::open(kit)?)?;
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

    /// The first line of `info.txt` picks the layout whatever language the rest is in.
    #[test]
    fn the_layout_is_read_from_info() {
        let kit = |info| std::io::Cursor::new(zip_of(&[("info.txt", info), ("Docs.toml", DOCS)]));
        assert_eq!(parse(kit("restore-kit v1\n\nKit di emergenza")).unwrap().len(), 1);
        let newer = parse(kit("restore-kit v2\n")).unwrap_err();
        assert!(matches!(newer.downcast_ref(), Some(Error::Newer { version: 2 })), "{newer}");
        let other = parse(kit("a zip of something else")).unwrap_err();
        assert!(matches!(other.downcast_ref(), Some(Error::NoLayout)), "{other}");
    }

    /// A small zip that decompresses to more than any profile is refused. So is one with too many files.
    #[test]
    fn a_zip_bomb_is_refused() {
        let huge = " ".repeat(usize::try_from(LARGEST_FILE).unwrap() + 1);
        let bomb = zip_of(&[("info.txt", "restore-kit v1\n"), ("Docs.toml", &huge)]);
        assert!(bomb.len() < 100_000, "deflate shrinks it: {}", bomb.len());
        let refused = parse(std::io::Cursor::new(bomb)).unwrap_err();
        assert!(matches!(refused.downcast_ref(), Some(Error::TooLarge { .. })), "{refused}");
        let names: Vec<String> = (0..=MOST_FILES).map(|i| format!("{i}.toml")).collect();
        let files: Vec<(&str, &str)> =
            std::iter::once(("info.txt", "restore-kit v1\n")).chain(names.iter().map(|n| (n.as_str(), DOCS))).collect();
        let refused = parse(std::io::Cursor::new(zip_of(&files))).unwrap_err();
        assert!(matches!(refused.downcast_ref(), Some(Error::TooManyFiles)), "{refused}");
    }

    #[test]
    #[expect(clippy::disallowed_methods, reason = "lays out a kit and a profile folder")]
    fn unpacking_never_overwrites_a_different_profile() {
        let dir = crate::sys::test_dir("kit");
        let kit = dir.join("kit.zip");
        std::fs::write(&kit, zip_of(&[("info.txt", "restore-kit v1\n"), ("Docs.toml", DOCS)])).unwrap();
        let profiles = dir.join("profiles");
        assert_eq!(unpack(&kit, &profiles).unwrap(), 1);
        assert_eq!(unpack(&kit, &profiles).unwrap(), 0, "the same profile is there already");
        std::fs::write(profiles.join("Docs.toml"), "[repository]\n").unwrap();
        assert!(unpack(&kit, &profiles).is_err());
        std::fs::write(&kit, zip_of(&[("info.txt", "restore-kit v1\n"), ("../x.toml", DOCS)])).unwrap();
        assert!(unpack(&kit, &profiles).is_err());
    }
}
