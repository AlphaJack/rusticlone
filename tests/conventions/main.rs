//! The code conventions in CONTRIBUTING that a test can check, read from the sources with `syn`, in one binary.

mod duplicates;
mod literals;
mod paths;
mod systems;

/// The crate's folder, which the sources are under.
#[expect(clippy::disallowed_methods, reason = "finds the sources")]
fn root() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR").map(std::path::PathBuf::from).unwrap_or_default()
}

/// `#[test]`, or `#[cfg(test)]`.
fn is_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| match &a.meta {
        syn::Meta::Path(path) => path.is_ident("test"),
        syn::Meta::List(list) => list.path.is_ident("cfg") && list.tokens.to_string() == "test",
        syn::Meta::NameValue(_) => false,
    })
}
