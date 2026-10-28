//! `cargo xtask translations`: lists, for each translation in `i18n`, the English messages it lacks, which
//! show in English. What a translation gets wrong fails the build instead.

use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use fluent_syntax::ast::Entry;

/// The language of the catalog that holds every message, which the others fall back to.
const FALLBACK: &str = "en";

pub(crate) fn report() -> Result<()> {
    let i18n = Path::new(env!("CARGO_MANIFEST_DIR")).join("../i18n");
    let languages: Vec<PathBuf> =
        entries(&i18n)?.into_iter().filter(|p| p.is_dir() && p.file_name() != Some(OsStr::new(FALLBACK))).collect();
    for catalog in entries(&i18n.join(FALLBACK))?.iter().filter(|p| p.extension() == Some(OsStr::new("ftl"))) {
        let english = messages(catalog)?;
        for language in &languages {
            let path = language.join(catalog.file_name().unwrap_or_default());
            let translated = messages(&path)?;
            let missing: Vec<&str> = english.difference(&translated).map(String::as_str).collect();
            let shown = path.strip_prefix(&i18n).unwrap_or(&path).display();
            if missing.is_empty() {
                println!("{shown}: complete");
            } else {
                println!("{shown}: {} messages shown in English: {}", missing.len(), missing.join(" "));
            }
        }
    }
    Ok(())
}

/// The paths in `dir`, sorted.
fn entries(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir(dir).with_context(|| dir.display().to_string())?;
    let mut paths: Vec<PathBuf> = entries.map(|entry| Ok(entry?.path())).collect::<Result<_>>()?;
    paths.sort();
    Ok(paths)
}

/// The ids of the messages in the catalog at `path`.
fn messages(path: &Path) -> Result<BTreeSet<String>> {
    let text = fs::read_to_string(path).with_context(|| path.display().to_string())?;
    let resource =
        fluent_syntax::parser::parse(text.as_str()).map_err(|(_, errors)| anyhow!("{}: {errors:?}", path.display()))?;
    Ok((resource.body.iter())
        .filter_map(|entry| match entry {
            Entry::Message(message) => Some(message.id.name.to_owned()),
            Entry::Term(_)
            | Entry::Comment(_)
            | Entry::GroupComment(_)
            | Entry::ResourceComment(_)
            | Entry::Junk { .. } => None,
        })
        .collect())
}
