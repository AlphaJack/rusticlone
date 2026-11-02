//! Putting a folder on the user's `PATH` for new terminals, as rustup and the installer script do: for
//! rusticlone, and for the rustic it downloads beside it.

#![expect(clippy::disallowed_methods, reason = "reads PATH and edits the shells' startup files")]

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::log;

#[derive(Debug, Serialize, crate::i18n::Fluent)]
#[fluent(prefix = "install")]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum Logged {
    OnPath { dir: String, changed: String },
}

log::level!(Success: Logged);

/// Whether this process's `PATH` has `dir`.
pub(crate) fn has(dir: &Path) -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d == dir))
}

/// Puts `dir` on `PATH` for new terminals, unless this process's has it, and says what changed.
pub(crate) fn put(dir: &Path) -> Result<()> {
    let changed = if has(dir) { String::new() } else { imp::add(dir)? };
    if !changed.is_empty() {
        log::event(None, &Logged::OnPath { dir: dir.display().to_string(), changed });
    }
    Ok(())
}

/// As rustup and the installer script do: an `env` script beside `dir` that puts it on `PATH`, sourced by
/// the shells' startup files.
#[cfg(unix)]
mod imp {
    use std::{io::Write, path::Path};

    use anyhow::Result;

    /// The startup files changed, by name.
    pub(super) fn add(dir: &Path) -> Result<String> {
        // Root's folder is on `PATH` already.
        if crate::sys::is_root() {
            return Ok(String::new());
        }
        let home = etcetera::home_dir()?;
        let env = dir.parent().unwrap_or(dir).join("env");
        let quoted = dir.display().to_string().replace('"', "\\\"");
        if !env.exists() {
            let script = format!(
                r#"#!/bin/sh
# Puts {quoted} on PATH, as rustup does.
case ":${{PATH}}:" in
    *:"{quoted}":*)
        ;;
    *)
        export PATH="{quoted}:$PATH"
        ;;
esac
"#
            );
            std::fs::write(&env, script)?;
        }
        let shell = std::env::var("SHELL").unwrap_or_default();
        let source = format!(". \"{}\"\n", env.display().to_string().replace('"', "\\\""));
        let fish = format!("fish_add_path --global --prepend \"{quoted}\"\n");
        // Created if missing: .profile for every shell, the others for the user's own.
        let files = [
            (home.join(".profile"), &source, true),
            (home.join(".bashrc"), &source, false),
            (home.join(".bash_profile"), &source, false),
            (home.join(".zshenv"), &source, shell.ends_with("zsh")),
            (home.join(".config/fish/conf.d").join(format!("{}.fish", crate::NAME)), &fish, shell.ends_with("fish")),
        ];
        let mut changed = Vec::new();
        for (file, line, create) in files {
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            if (!create && !file.exists()) || text.contains(line.trim()) {
                continue;
            }
            std::fs::create_dir_all(file.parent().unwrap_or(&file))?;
            let separator = if text.is_empty() || text.ends_with('\n') { "" } else { "\n" };
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&file)?
                .write_all(format!("{separator}{line}").as_bytes())?;
            changed.push(file.display().to_string());
        }
        Ok(changed.join(", "))
    }
}

/// As the installer script does: prepended to the `Path` that new processes get, the user's or, for an
/// administrator, the system's, and announced to running programs so that new terminals see it.
#[cfg(windows)]
mod imp {
    use std::path::Path;

    use anyhow::Result;
    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_EXPAND_SZ, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ,
            RRF_RT_REG_SZ, RegGetValueW, RegSetKeyValueW,
        },
        UI::WindowsAndMessaging::{HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE},
    };

    const SYSTEM: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
    /// How long a window may take to read the change of `Path`, in milliseconds.
    const BROADCAST_TIMEOUT: u32 = 5000;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    /// The key that holds `Path`, by its name too.
    fn key() -> (HKEY, &'static str, String) {
        match crate::sys::is_root() {
            true => (HKEY_LOCAL_MACHINE, SYSTEM, format!(r"HKEY_LOCAL_MACHINE\{SYSTEM}")),
            false => (HKEY_CURRENT_USER, "Environment", r"HKEY_CURRENT_USER\Environment".into()),
        }
    }

    /// `Path` as stored, with its variables unexpanded. Empty if unset.
    #[expect(unsafe_code, reason = "reads a registry value into a buffer of the size it reports")]
    fn read(root: HKEY, key: &str) -> String {
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
        let (key, value) = (wide(key), wide("Path"));
        // SAFETY: NUL-terminated strings, and a buffer of `size` bytes; without one, it reports the size it needs.
        let get = |buffer: *mut u16, size: &mut u32| unsafe {
            RegGetValueW(root, key.as_ptr(), value.as_ptr(), flags, std::ptr::null_mut(), buffer.cast(), size)
        };
        let mut size = 0u32;
        if get(std::ptr::null_mut(), &mut size) != ERROR_SUCCESS {
            return String::new();
        }
        let mut buffer = vec![0u16; size as usize / 2];
        if get(buffer.as_mut_ptr(), &mut size) != ERROR_SUCCESS {
            return String::new();
        }
        String::from_utf16_lossy(&buffer).trim_end_matches('\0').to_owned()
    }

    /// The key changed, by name.
    #[expect(unsafe_code, reason = "writes a registry value and broadcasts that the environment changed")]
    pub(super) fn add(dir: &Path) -> Result<String> {
        let (root, key, name) = key();
        let path = read(root, key);
        let dir = dir.display().to_string();
        if path.split(';').any(|d| d.eq_ignore_ascii_case(&dir)) {
            return Ok(name);
        }
        let data = wide(&[dir.as_str(), &path].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(";"));
        let size = u32::try_from(size_of_val(data.as_slice()))?;
        // SAFETY: NUL-terminated strings, and the data's size in bytes.
        let status = unsafe {
            RegSetKeyValueW(root, wide(key).as_ptr(), wide("Path").as_ptr(), REG_EXPAND_SZ, data.as_ptr().cast(), size)
        };
        if status != ERROR_SUCCESS {
            return Err(std::io::Error::from_raw_os_error(status.cast_signed()).into());
        }
        // SAFETY: a NUL-terminated string that outlives the call, which times out rather than waits on a hung window.
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                wide("Environment").as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                BROADCAST_TIMEOUT,
                std::ptr::null_mut(),
            )
        };
        Ok(name)
    }
}
