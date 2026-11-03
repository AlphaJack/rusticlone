//! How rustic names paths in a snapshot's tree on Unix.

/// Rustic's paths in a snapshot's tree, which it prints in `{:?}` form: with `/` between their parts here.
#[cfg(feature = "server")]
pub(super) fn with_slashes(stdout: &str) -> std::borrow::Cow<'_, str> {
    stdout.into()
}

/// `path` as rustic names it in a snapshot's tree. rustic on Windows records `C:\Users\me` as `C/Users/me` in the
/// tree, which it finds here only as `/C/Users/me`.
pub(crate) fn tree_path(path: &str) -> std::borrow::Cow<'_, str> {
    match crate::sys::windows_drive(path) {
        Some((letter, rest)) => format!("/{letter}{}", rest.replace('\\', "/")).into(),
        None => path.into(),
    }
}

/// The names of `path` below its root, as a snapshot's tree has them. A Unix path has no drive.
pub(crate) fn parts(path: &std::path::Path) -> Vec<String> {
    let name = |c| {
        if let std::path::Component::Normal(part) = c {
            Some(std::ffi::OsStr::to_string_lossy(part).into_owned())
        } else {
            None
        }
    };
    path.components().filter_map(name).collect()
}

#[cfg(test)]
mod tests {
    use super::tree_path;

    /// A Windows folder restores here by its name in the tree. This system's own stays as it is.
    #[test]
    fn windows_folders_are_named_as_the_tree_names_them() {
        assert_eq!(tree_path(r"C:\Users\me\Résumé"), "/C/Users/me/Résumé");
        assert_eq!(tree_path("d:/data"), "/d/data");
        assert_eq!(tree_path("/home/me"), "/home/me");
        assert_eq!(tree_path("C:relative"), "C:relative");
    }
}
