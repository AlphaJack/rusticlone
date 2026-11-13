//! What reaches the system says so where it does: a function at a time, not a whole module.

#![expect(clippy::disallowed_methods, reason = "reads the sources")]

use std::{fs, path::Path};

/// Every `.rs` file below `dir`.
fn sources(dir: &Path, found: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

/// A test file, which may touch files anywhere: `tests.rs`, or one below a `tests` folder.
fn is_test_file(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n == "tests.rs") || path.components().any(|c| c.as_os_str() == "tests")
}

/// Whether `source` expects `clippy::disallowed_methods` for a whole module, other than an inline `mod tests`.
fn module_wide(source: &str) -> bool {
    let mut in_tests = false;
    source.lines().any(|line| {
        let trimmed = line.trim_start();
        if trimmed.starts_with("mod ") && trimmed.ends_with('{') {
            in_tests = trimmed == "mod tests {";
        }
        let inner = trimmed.starts_with("#![expect(") || trimmed.starts_with("#![allow(");
        inner && trimmed.contains("clippy::disallowed_methods") && !(in_tests && line.starts_with(' '))
    })
}

#[test]
fn the_system_is_reached_one_function_at_a_time() {
    let src = super::root().join("src");
    let mut files = Vec::new();
    sources(&src, &mut files);
    let module_wide: Vec<String> = files
        .iter()
        .filter(|f| !is_test_file(f.strip_prefix(&src).unwrap_or(f)))
        .filter(|f| module_wide(&fs::read_to_string(f).unwrap()))
        .map(|f| f.strip_prefix(super::root()).unwrap_or(f).display().to_string())
        .collect();
    assert!(
        module_wide.is_empty(),
        "a module-wide expect of clippy::disallowed_methods, which belongs on each function that reaches the \
         system:\n{}",
        module_wide.join("\n")
    );
}
