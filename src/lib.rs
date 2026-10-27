//! Turns rustic profiles into collections that back up, copy to their targets, forget only what
//! every target has, prune, check and restore.

/// The environment variable `KEY` with this crate's prefix, as a constant.
macro_rules! env_key {
    ($key:literal) => {{
        const KEY: &str = {
            const BYTES: [u8; $crate::NAME.len() + 1 + $key.len()] = $crate::env_key($key);
            match str::from_utf8(&BYTES) {
                Ok(key) => key,
                Err(_) => panic!("not UTF-8"),
            }
        };
        KEY
    }};
}

mod args;
pub mod cli;
mod clock;
mod engine;
mod first_run;
mod fmt;
#[cfg(feature = "server")]
mod health;
mod i18n;
#[cfg(any(feature = "desktop", feature = "self-update"))]
mod install;
mod live;
mod log;
mod notification;
mod profile;
mod request;
mod result;
mod rustic;
#[cfg(feature = "server")]
mod server;
mod settings;
mod shape;
mod state;
mod sys;

/// What people type: the executable, its commands, files and services.
const NAME: &str = env!("CARGO_PKG_NAME");
/// What people see on the app, its windows and menus: `NAME` capitalized, as Docker's app runs `docker`.
pub const APP_NAME: &str = env!("APP_NAME");
/// Starts every environment variable read.
pub const ENV_PREFIX: &str = env_key!("");

/// `NAME` in upper case, `_` and `key`.
const fn env_key<const N: usize>(key: &str) -> [u8; N] {
    let (name, key, mut out, mut i) = (NAME.as_bytes(), key.as_bytes(), [b'_'; N], 0);
    while i < name.len() {
        out[i] = name[i].to_ascii_uppercase();
        i += 1;
    }
    while i < key.len() + name.len() {
        out[i + 1] = key[i - name.len()];
        i += 1;
    }
    out
}

/// For the integration tests, which read the state.
pub use request::StepKey;
pub use result::Status;
pub use state::{Event, RunRecord, Store};

#[cfg(test)]
mod tests {
    /// Writes each file at its path in the repository, as its content renders in English, under a line that says
    /// `source` generates it. Fails when that changes one, as they are committed. The `generated_*` tests call it,
    /// and `cargo xtask generate` runs them.
    #[cfg(feature = "server")]
    #[expect(clippy::disallowed_methods, reason = "writes the generated files")]
    pub(crate) fn generated(source: &str, files: &[(String, &dyn Fn() -> String)]) {
        let written: Vec<String> = (files.iter())
            .filter_map(|(path, content)| {
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
                let content = crate::i18n::Language::english().scope(content);
                let file = format!("# Generated from {source} by cargo xtask generate\n{content}");
                if std::fs::read_to_string(&path).is_ok_and(|before| before == file) {
                    return None;
                }
                std::fs::create_dir_all(path.parent().unwrap_or(&path)).unwrap_or_default();
                std::fs::write(&path, file).unwrap_or_default();
                Some(path.display().to_string())
            })
            .collect();
        assert!(written.is_empty(), "written anew, to commit: {}", written.join(", "));
    }

    /// The Linux targets that dist builds, from Cargo.toml.
    #[cfg(feature = "server")]
    #[expect(clippy::disallowed_methods, reason = "reads Cargo.toml")]
    pub(crate) fn linux_targets() -> Vec<String> {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap_or_default();
        let manifest: toml_edit::DocumentMut = manifest.parse().unwrap_or_default();
        let targets = manifest["workspace"]["metadata"]["dist"]["targets"].as_array().into_iter().flatten();
        targets.filter_map(|t| t.as_str()).filter(|t| t.contains("-linux-")).map(ToOwned::to_owned).collect()
    }
}
