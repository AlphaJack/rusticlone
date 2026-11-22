//! A crate that more than one package uses is declared once, in `[workspace.dependencies]`, and each package
//! takes it with `workspace = true`. Renovate updates a crate's lock file entry once for each manifest that
//! declares its version: the second update looks for the version that the first one replaced, and fails.

#![expect(clippy::disallowed_methods, reason = "reads the manifests")]

use std::{collections::BTreeMap, fs};

use toml_edit::{DocumentMut, Item, TableLike};

use super::root;

/// The crates that a dependency table declares with a version of its own.
fn versioned(table: &dyn TableLike) -> impl Iterator<Item = &str> {
    table.iter().filter(|(_, spec)| spec.get("workspace").is_none() && spec.get("path").is_none()).map(|(n, _)| n)
}

/// The dependency tables of a manifest, also those of each target.
fn tables(manifest: &DocumentMut) -> Vec<&dyn TableLike> {
    let kinds = ["dependencies", "dev-dependencies", "build-dependencies"];
    let targets =
        manifest.get("target").and_then(Item::as_table_like).into_iter().flat_map(|t| t.iter().map(|(_, t)| t));
    std::iter::once(manifest.as_item())
        .chain(targets)
        .flat_map(|t| kinds.iter().filter_map(|k| t.get(k)?.as_table_like()))
        .collect()
}

#[test]
fn a_shared_crate_is_declared_once() {
    let read =
        |dir: std::path::PathBuf| fs::read_to_string(dir.join("Cargo.toml")).unwrap().parse::<DocumentMut>().unwrap();
    let workspace = read(root());
    let members = workspace["workspace"]["members"].as_array().unwrap().iter().map(|m| m.as_str().unwrap().to_owned());
    let mut packages = vec![("rusticlone".to_owned(), workspace.clone())];
    packages.extend(members.map(|m| (m.clone(), read(root().join(m)))));
    let mut declared: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (package, manifest) in &packages {
        for name in tables(manifest).into_iter().flat_map(versioned) {
            let by = declared.entry(name).or_default();
            if !by.contains(&package.as_str()) {
                by.push(package);
            }
        }
    }
    declared.retain(|_, by| by.len() > 1);
    assert!(declared.is_empty(), "declare these in [workspace.dependencies]: {declared:?}");
}
