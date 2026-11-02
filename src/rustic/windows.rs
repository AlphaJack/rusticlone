//! How rustic names paths in a snapshot's tree on Windows.

/// Rustic's paths in a snapshot's tree, which it prints in `{:?}` form: it prints `\\`, escaped, which is in no
/// file name here, where the tree has `/`.
#[cfg(feature = "server")]
pub(super) fn with_slashes(stdout: &str) -> std::borrow::Cow<'_, str> {
    stdout.replace(r"\\", "/").into()
}

/// `path` as rustic names it in a snapshot's tree: as it is here, where rustic records `C:\Users\me` itself.
pub(crate) fn tree_path(path: &str) -> std::borrow::Cow<'_, str> {
    path.into()
}

#[cfg(test)]
mod tests {
    use super::tree_path;

    /// This system's folders stay as they are.
    #[test]
    fn folders_are_named_as_the_tree_names_them() {
        assert_eq!(tree_path(r"C:\Users\me"), r"C:\Users\me");
        assert_eq!(tree_path("/home/me"), "/home/me");
        assert_eq!(tree_path("C:relative"), "C:relative");
    }

    #[cfg(feature = "server")]
    #[test]
    fn sources_are_found_in_the_tree() {
        assert_eq!(crate::rustic::in_tree(r"C:\Users\me\docs"), "/C/Users/me/docs");
    }
}
