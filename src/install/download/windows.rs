//! Windows's archives are `.zip`, and any file runs.

use std::{fs::OpenOptions, io::Read, path::Path};

use anyhow::Result;

use super::Error;

/// The file called `name` in the `.zip` `archive`, at any depth.
pub(crate) fn unpack(archive: &str, data: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data))?;
    let index = (0..zip.len())
        .find(|&i| {
            zip.name_for_index(i)
                .and_then(Result::ok)
                .is_some_and(|p| Path::new(&*p).file_name().is_some_and(|f| f == name))
        })
        .ok_or_else(|| Error::NoExecutable { archive: archive.to_owned(), name: name.to_owned() })?;
    let mut file = Vec::new();
    zip.by_index(index)?.read_to_end(&mut file)?;
    Ok(file)
}

/// Nothing to set: Windows runs a file by its name.
pub(super) fn executable(_: &mut OpenOptions) {}

#[cfg(test)]
pub(super) fn is_executable(_: &Path) -> bool {
    true
}
