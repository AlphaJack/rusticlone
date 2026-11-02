//! Paths are joined a component at a time: `dir.join("a").join("b")`, not `dir.join("a").join("b")`, which Windows shows
//! as `C:\dir\a/b`. An absolute Unix path, such as `/usr/bin`, stays whole, as only Unix has it.

#![expect(clippy::unwrap_used, reason = "a test fails loudly")]
#![expect(clippy::disallowed_methods, reason = "reads the sources")]

use std::{fs, path::Path};

use syn::{
    Expr, ExprCall, ExprMethodCall, Lit, Macro, Token,
    punctuated::Punctuated,
    visit::{self, Visit},
};

/// The lines where a path is written with `/` between its components.
#[derive(Default)]
struct Slashed(Vec<usize>);

/// Whether `text` is a relative path of more than one component, or a format of one.
fn slashed(text: &str) -> bool {
    !text.starts_with('/') && text.trim_end_matches('/').contains('/')
}

impl Slashed {
    /// A literal, or a `format!` of one, with `/` between components.
    fn check(&mut self, arg: Option<&Expr>) {
        let text = if let Some(Expr::Lit(syn::ExprLit { lit: Lit::Str(text), .. })) = arg {
            Some((text.value(), text.span()))
        } else if let Some(Expr::Macro(mac)) = arg
            && mac.mac.path.is_ident("format")
        {
            format_literal(&mac.mac)
        } else {
            None
        };
        if let Some((_, span)) = text.filter(|(t, _)| slashed(t)) {
            self.0.push(span.start().line);
        }
    }
}

fn format_literal(mac: &Macro) -> Option<(String, proc_macro2::Span)> {
    let lit: syn::LitStr = mac
        .parse_body_with(|input: syn::parse::ParseStream<'_>| {
            let lit = input.parse()?;
            _ = input.parse::<proc_macro2::TokenStream>()?;
            Ok(lit)
        })
        .ok()?;
    Some((lit.value(), lit.span()))
}

impl<'ast> Visit<'ast> for Slashed {
    /// Macros' arguments, such as `assert_eq!`'s, are expressions too, which syn leaves as tokens.
    fn visit_macro(&mut self, mac: &'ast Macro) {
        if let Ok(args) = mac.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated) {
            args.iter().for_each(|arg| self.visit_expr(arg));
        }
    }

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        if call.method == "join" {
            self.check(call.args.first());
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(path) = &*call.func {
            let names: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();
            if names.ends_with(&["Path".into(), "new".into()]) || names.ends_with(&["PathBuf".into(), "from".into()]) {
                self.check(call.args.first());
            }
        }
        visit::visit_expr_call(self, call);
    }
}

fn root() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR").map(std::path::PathBuf::from).unwrap_or_default()
}

fn sources(dir: &Path, found: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let mut slashed = Slashed::default();
            slashed.visit_file(&syn::parse_file(&fs::read_to_string(&path).unwrap()).unwrap());
            let file = path.strip_prefix(root()).unwrap_or(&path).display().to_string();
            found.extend(slashed.0.into_iter().map(|line| format!("{file}:{line}")));
        }
    }
}

#[test]
fn paths_are_joined_a_component_at_a_time() {
    let mut found = Vec::new();
    for dir in ["src", "tests"] {
        sources(&root().join(dir), &mut found);
    }
    found.sort();
    assert!(
        found.is_empty(),
        "a path with / between its components, to join a component at a time:\n{}",
        found.join("\n")
    );
}

#[test]
fn only_relative_paths_of_several_components_are_slashed() {
    assert!(slashed("a/b") && slashed("tests/fixtures/x.json") && slashed("devices/{name}/queue"));
    assert!(!slashed("/usr/bin") && !slashed("a") && !slashed("a/") && !slashed("/proc"));
}
