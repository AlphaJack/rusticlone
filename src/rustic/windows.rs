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

/// The names of `path` below its root, as a snapshot's tree has them: the drive's first, so `C:\Users\me` is `C`,
/// `Users` and `me`.
pub(crate) fn parts(path: &std::path::Path) -> Vec<String> {
    use std::path::Component;
    let mut out = Vec::new();
    for c in path.components() {
        match c {
            Component::Normal(part) => out.push(part.to_string_lossy().into_owned()),
            Component::Prefix(prefix) => out.extend(
                (prefix.as_os_str().to_string_lossy().split(['\\', '/', ':', '?']).filter(|p| !p.is_empty()))
                    .map(ToOwned::to_owned),
            ),
            Component::RootDir | Component::CurDir | Component::ParentDir => {}
        }
    }
    out
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
