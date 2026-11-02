//! What macOS and Linux share to tell what keeps them awake.

/// What `command` prints.
#[cfg(test)]
#[expect(clippy::disallowed_methods, reason = "asks the system what keeps it awake")]
pub(super) fn output(command: &[&str]) -> String {
    let out = std::process::Command::new(command[0]).args(&command[1..]).output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}
