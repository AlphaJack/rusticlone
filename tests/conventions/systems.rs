//! Each system's calls are in a file of their own. A `cfg` that picks a system goes only over the declaration
//! that picks such a file, `mod linux;` or `use linux as imp;`. Every one of those files defines the same
//! functions, so no other code tells the systems apart. A system is picked by a `target_*` key, by `unix` or
//! `windows`, or by a cfg that `build.rs` declares for the target, such as `tray`.

#![expect(clippy::disallowed_methods, reason = "reads the sources")]

use std::{fs, path::Path};

use syn::{
    Attribute, Expr, ItemMod, ItemUse, Lit, Macro, Meta, Token, UseTree,
    punctuated::Punctuated,
    spanned::Spanned,
    visit::{self, Visit},
};

/// The cfgs that `build.rs` declares, from its `cargo::rustc-check-cfg=cfg(NAME)` instructions.
#[derive(Default)]
struct Declared(Vec<String>);

impl<'ast> Visit<'ast> for Declared {
    fn visit_macro(&mut self, mac: &'ast Macro) {
        let args = mac.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated).unwrap_or_default();
        for arg in args {
            if let Expr::Lit(lit) = arg
                && let Lit::Str(text) = lit.lit
                && let Some(name) =
                    text.value().strip_prefix("cargo::rustc-check-cfg=cfg(").and_then(|n| n.strip_suffix(')'))
            {
                self.0.push(name.to_owned());
            }
        }
    }
}

/// The lines where a `cfg` picks a system, but over the declaration of a file per system.
struct Systems<'a> {
    declared: &'a [String],
    lines: Vec<usize>,
}

impl Systems<'_> {
    /// Whether a `cfg` or `cfg_attr` attribute's predicate picks a system.
    fn picks(&self, attr: &Attribute) -> bool {
        let predicate = if attr.path().is_ident("cfg") {
            attr.parse_args::<Meta>().ok()
        } else if attr.path().is_ident("cfg_attr") {
            attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                .ok()
                .and_then(|p| p.into_iter().next())
        } else {
            None
        };
        predicate.is_some_and(|p| self.system(&p))
    }

    /// Whether `predicate`, or any predicate within `any`, `all` or `not`, picks a system.
    fn system(&self, predicate: &Meta) -> bool {
        match predicate {
            Meta::Path(path) => {
                path.get_ident().is_some_and(|i| i == "unix" || i == "windows" || self.declared.iter().any(|d| i == d))
            }
            Meta::NameValue(pair) => pair.path.get_ident().is_some_and(|i| i.to_string().starts_with("target_")),
            Meta::List(list) => list
                .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                .is_ok_and(|inner| inner.iter().any(|p| self.system(p))),
        }
    }
}

impl<'ast> Visit<'ast> for Systems<'_> {
    fn visit_attribute(&mut self, attr: &'ast Attribute) {
        if self.picks(attr) {
            self.lines.push(attr.span().start().line);
        }
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        if mac.path.is_ident("cfg") && mac.parse_body::<Meta>().is_ok_and(|p| self.system(&p)) {
            self.lines.push(mac.span().start().line);
        }
    }

    /// `mod linux;` picks a file.
    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        if item.content.is_some() {
            visit::visit_item_mod(self, item);
        }
    }

    /// `use linux as imp;` names the file picked.
    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        if !matches!(item.tree, UseTree::Rename(_)) {
            visit::visit_item_use(self, item);
        }
    }
}

fn declared() -> Vec<String> {
    let mut declared = Declared::default();
    let build = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("build.rs")).unwrap();
    declared.visit_file(&syn::parse_file(&build).unwrap());
    declared.0
}

fn picked(source: &str, declared: &[String]) -> Vec<usize> {
    let mut systems = Systems { declared, lines: Vec::new() };
    systems.visit_file(&syn::parse_file(source).unwrap());
    systems.lines
}

/// Every `cfg` out of place in `dir`, as `file:line`.
fn sources(dir: &Path, declared: &[String], found: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, declared, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let file = path.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap_or(&path).display().to_string();
            let lines = picked(&fs::read_to_string(&path).unwrap(), declared);
            found.extend(lines.into_iter().map(|line| format!("{file}:{line}")));
        }
    }
}

#[test]
fn each_systems_calls_are_in_a_file_of_their_own() {
    let mut found = Vec::new();
    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &declared(), &mut found);
    found.sort();
    found.dedup();
    assert!(found.is_empty(), "a system's cfg outside the declaration of a file per system:\n{}", found.join("\n"));
}

#[test]
fn build_rs_declares_tray() {
    assert!(declared().iter().any(|d| d == "tray"), "{:?}", declared());
}

#[test]
fn only_picking_a_file_may_name_a_system() {
    let source = "#[cfg(windows)]\nmod windows;\n#[cfg(windows)]\nuse windows as imp;\n#[cfg(unix)]\nfn a() {}\n";
    assert_eq!(picked(source, &[]), [5]);
}

#[test]
fn a_system_is_found_wherever_a_cfg_names_it() {
    let source = "#[cfg_attr(\n    any(target_os = \"linux\", tray),\n    expect(dead_code)\n)]\nfn a() {\n    if cfg!(all(unix, not(feature = \"x\"))) {}\n}\nenum E {\n    #[cfg(windows)]\n    V,\n}\n#[cfg(all(test, unix))]\nmod tests {}\n";
    assert_eq!(picked(source, &["tray".to_owned()]), [1, 6, 9, 12]);
}

#[test]
fn features_and_tests_pick_no_system() {
    assert!(picked("#[cfg(feature = \"server\")]\nfn a() {}\n#[cfg(test)]\nmod tests {}\n", &[]).is_empty());
}
