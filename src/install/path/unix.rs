use std::{io::Write, path::Path};

use anyhow::Result;

/// The startup files changed, by name.
#[expect(clippy::disallowed_methods, reason = "reads PATH and edits the shells' startup files")]
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
        (
            home.join(".config").join("fish").join("conf.d").join(format!("{}.fish", crate::NAME)),
            &fish,
            shell.ends_with("fish"),
        ),
    ];
    let mut changed = Vec::new();
    for (file, line, create) in files {
        // As bytes: a file that is not UTF-8 still shows whether it has the line.
        let text = std::fs::read(&file).unwrap_or_default();
        if (!create && !file.exists()) || text.windows(line.trim().len()).any(|w| w == line.trim().as_bytes()) {
            continue;
        }
        std::fs::create_dir_all(file.parent().unwrap_or(&file))?;
        let separator = if text.is_empty() || text.ends_with(b"\n") { "" } else { "\n" };
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)?
            .write_all(format!("{separator}{line}").as_bytes())?;
        changed.push(file.display().to_string());
    }
    Ok(changed.join(", "))
}
