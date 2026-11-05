//! Executables downloaded from GitHub releases: rustic's when it is missing, and rusticlone's own on update.

use std::{
    io::{Read, Write},
    path::Path,
};

use anyhow::Result;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as os;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as os;

pub(crate) use os::unpack;

/// A larger download is refused.
const MAX_SIZE: u64 = 1 << 26;

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "download-error")]
pub(crate) enum Error {
    Download { url: String, source: mercurio::Error },
    Status { url: String, status: u16 },
    NoExecutable { archive: String, name: String },
}

/// With Mercurio's HTTP client, which notifications use too.
pub(crate) fn fetch(url: &str) -> Result<Vec<u8>> {
    use mercurio::transport::{BlockingTransport, Request, Transport};
    let request = Request::get(url).max_body(MAX_SIZE);
    let response = BlockingTransport::new().http(&request);
    let response = response.map_err(|source| Error::Download { url: url.to_owned(), source })?;
    if !response.is_success() {
        return Err(Error::Status { url: url.to_owned(), status: response.status }.into());
    }
    Ok(response.body)
}

/// The file called `name` in the `.tar.gz` `archive`, at any depth.
pub(crate) fn untar(archive: &str, data: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(data));
    let mut entry = (tar.entries()?.filter_map(Result::ok))
        .find(|e| e.path().is_ok_and(|p| p.file_name().is_some_and(|f| f == name)))
        .ok_or_else(|| Error::NoExecutable { archive: archive.to_owned(), name: name.to_owned() })?;
    let mut file = Vec::new();
    entry.read_to_end(&mut file)?;
    Ok(file)
}

/// Writes an executable, renamed into place so that no half-written one runs.
#[expect(clippy::disallowed_methods, reason = "writes downloaded executables")]
pub(crate) fn install(binary: &[u8], path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let partial = path.with_extension("partial");
    let mut options = std::fs::OpenOptions::new();
    os::executable(&mut options);
    options.write(true).create(true).truncate(true).open(&partial)?.write_all(binary)?;
    std::fs::rename(&partial, path)?;
    Ok(())
}

/// A server on this computer that answers every request with `status` and `body`, and sends the path asked
/// for. Its URL, without a trailing slash.
#[cfg(test)]
pub(super) fn serve(status: u16, body: &'static [u8]) -> (String, std::sync::mpsc::Receiver<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for mut socket in listener.incoming().flatten() {
            let mut request = [0; 8192];
            let read = socket.read(&mut request).unwrap_or_default();
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request.split(' ').nth(1).unwrap_or_default().to_owned();
            let head = format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            _ = socket.write_all(head.as_bytes()).and_then(|()| socket.write_all(body));
            _ = tx.send(path);
        }
    });
    (url, rx)
}

/// The next path that the server of [`serve`] was asked for. A test whose request never comes fails, rather than
/// waiting forever.
#[cfg(test)]
pub(super) fn asked(paths: &std::sync::mpsc::Receiver<String>) -> String {
    paths.recv_timeout(std::time::Duration::from_secs(10)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_is_its_body_on_success_only() {
        let (url, asked) = serve(200, b"archive");
        assert_eq!(fetch(&format!("{url}/v1/r.tar.gz")).unwrap(), b"archive");
        assert_eq!(super::asked(&asked), "/v1/r.tar.gz");
        let (url, _) = serve(404, b"Not Found");
        let missing = fetch(&format!("{url}/r.tar.gz")).unwrap_err();
        assert!(matches!(missing.downcast_ref(), Some(Error::Status { status: 404, .. })), "{missing}");
        // Nothing listens on port 9 (discard).
        let refused = fetch("http://127.0.0.1:9/r.tar.gz").unwrap_err();
        assert!(matches!(refused.downcast_ref(), Some(Error::Download { .. })), "{refused}");
    }

    /// A download larger than [`MAX_SIZE`] is refused.
    #[test]
    fn a_larger_download_is_refused() {
        let (url, _) = serve(200, vec![0; usize::try_from(MAX_SIZE).unwrap() + 1].leak());
        let refused = fetch(&format!("{url}/r.tar.gz")).unwrap_err();
        assert!(matches!(refused.downcast_ref(), Some(Error::Download { .. })), "{refused}");
    }

    #[test]
    #[expect(clippy::disallowed_methods, reason = "writes downloaded executables")]
    fn the_executable_is_found_at_any_depth_and_installed_whole() {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        let mut header = tar::Header::new_gnu();
        header.set_size(6);
        tar.append_data(&mut header, "rustic-v1/bin/rustic", &b"binary"[..]).unwrap();
        let archive = tar.into_inner().unwrap().finish().unwrap();
        let binary = untar("r.tar.gz", &archive, "rustic").unwrap();
        assert_eq!(binary, b"binary");
        let missing = untar("r.tar.gz", &archive, "rusticlone").unwrap_err();
        assert_eq!(missing.to_string(), "r.tar.gz has no rusticlone");
        let path = std::env::temp_dir().join(format!("{}-install-{}", crate::NAME, std::process::id())).join("rustic");
        install(&binary, &path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"binary");
        assert!(!path.with_extension("partial").exists());
        assert!(os::is_executable(&path));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
