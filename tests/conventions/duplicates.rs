//! Each thing is done in one place: no two functions or closures have the same body, and no chain of calls is
//! written out again and again. A body is compared with its own parameters renamed, so `|a| a.len()` and
//! `|b| b.len()` are the same. Some bodies say nothing twice, and tests may repeat themselves. Those that only:
//! - build a value, as `|e| Error::Io { e }` and `Missing::Page(path)` do
//! - read a field, as `self.calls.lock().clone()` does, or pass fields on, as `retry_at(self.at, self.failures)`
//! - compare with a constant, as `|r| r.level() == Level::Failure` does

#![expect(clippy::disallowed_methods, reason = "reads the sources")]

use std::{collections::BTreeMap, fs, path::Path, str::FromStr};

use proc_macro2::{Span, TokenStream, TokenTree};
use syn::{
    Expr, ExprClosure, ExprMethodCall, ImplItemFn, ItemFn, Pat, Signature,
    spanned::Spanned,
    visit::{self, Visit},
};

use super::{is_test, root};

/// Bodies of fewer tokens are too short to say the same thing twice: `{ Ok(()) }`, `self.name.clone()`.
const SHORTEST_BODY: usize = 10;
/// Calls in a row that make a chain worth naming.
const LONGEST_CHAIN: usize = 3;
/// How often a chain may be written out before it should be a function.
const CHAIN_REPEATS: usize = 3;

/// Where each body and each chain is, by its tokens.
#[derive(Default)]
struct Found {
    bodies: BTreeMap<String, Vec<String>>,
    chains: BTreeMap<String, Vec<String>>,
}

struct Visitor<'a> {
    file: &'a str,
    source: &'a str,
    found: &'a mut Found,
}

impl Visitor<'_> {
    /// The text of `span` in the file.
    fn text(&self, span: Span) -> String {
        let (start, end) = (span.start(), span.end());
        let lines: Vec<&str> = self.source.lines().skip(start.line - 1).take(end.line - start.line + 1).collect();
        let mut text = String::new();
        for (i, line) in lines.iter().enumerate() {
            let from = if i == 0 { start.column } else { 0 };
            let to = if i == lines.len() - 1 { end.column } else { line.chars().count() };
            text.extend(line.chars().skip(from).take(to.saturating_sub(from)));
            text.push('\n');
        }
        text
    }

    fn here(&self, span: Span) -> String {
        format!("{}:{}", self.file, span.start().line)
    }

    /// Records a body with its parameters renamed `$0`, `$1`, and so on.
    fn body(&mut self, params: &[String], span: Span) {
        let Ok(tokens) = TokenStream::from_str(&self.text(span)) else { return };
        let mut flat = Vec::new();
        flatten(tokens, params, &mut flat);
        if flat.len() >= SHORTEST_BODY {
            let at = self.here(span);
            self.found.bodies.entry(flat.join(" ")).or_default().push(at);
        }
    }
}

/// Whether it only builds a value, reads or passes on fields of `self`, or compares with a constant.
fn builds(expr: &Expr) -> bool {
    if let Expr::Block(b) = expr {
        return builds_only(&b.block.stmts);
    }
    if let Expr::Call(call) = expr {
        return constructor(&call.func) || call.args.iter().all(reads_field);
    }
    if let Expr::Binary(b) = expr {
        return matches!(b.op, syn::BinOp::Eq(_) | syn::BinOp::Ne(_)) && (constant(&b.left) || constant(&b.right));
    }
    matches!(expr, Expr::Struct(_) | Expr::Tuple(_)) || reads_field(expr)
}

/// A tuple struct or an enum variant, named with a capital: `Missing::Page`.
fn constructor(func: &Expr) -> bool {
    matches!(func, Expr::Path(p) if p.path.segments.last().is_some_and(|s| s.ident.to_string().starts_with(char::is_uppercase)))
}

/// A literal, or a path to a constant or a unit variant: `Level::Failure`, `MAX`.
fn constant(expr: &Expr) -> bool {
    matches!(expr, Expr::Lit(_)) || constructor(expr)
}

fn reads_field(expr: &Expr) -> bool {
    if let Expr::MethodCall(call) = expr {
        return call.args.is_empty() && reads_field(&call.receiver);
    }
    let Expr::Field(field) = expr else { return false };
    matches!(&*field.base, Expr::Path(p) if p.path.is_ident("self")) || reads_field(&field.base)
}

fn builds_only(stmts: &[syn::Stmt]) -> bool {
    matches!(stmts, [syn::Stmt::Expr(e, None)] if builds(e))
}

/// The tokens, with each of `params` replaced by its position, but where a field or method has its name.
fn flatten(tokens: TokenStream, params: &[String], out: &mut Vec<String>) {
    for token in tokens {
        match token {
            TokenTree::Group(group) => {
                let (open, close) = match group.delimiter() {
                    proc_macro2::Delimiter::Parenthesis => ("(", ")"),
                    proc_macro2::Delimiter::Brace => ("{", "}"),
                    proc_macro2::Delimiter::Bracket => ("[", "]"),
                    proc_macro2::Delimiter::None => ("", ""),
                };
                out.push(open.to_owned());
                flatten(group.stream(), params, out);
                out.push(close.to_owned());
            }
            TokenTree::Ident(ident) => {
                let name = ident.to_string();
                let member = out.last().is_some_and(|t| t == ".");
                let param = params.iter().position(|p| *p == name).filter(|_| !member);
                out.push(param.map_or(name, |i| format!("${i}")));
            }
            other @ (TokenTree::Punct(_) | TokenTree::Literal(_)) => out.push(other.to_string()),
        }
    }
}

/// The names its parameters bind.
fn names<'a>(patterns: impl IntoIterator<Item = &'a Pat>) -> Vec<String> {
    let mut names = Vec::new();
    for pattern in patterns {
        if let Pat::Ident(p) = pattern {
            names.push(p.ident.to_string());
        } else if let Pat::Tuple(p) = pattern {
            names.extend(self::names(&p.elems));
        } else if let Pat::Type(syn::PatType { pat, .. }) | Pat::Reference(syn::PatReference { pat, .. }) = pattern {
            names.extend(self::names([&**pat]));
        }
    }
    names
}

fn inputs(signature: &Signature) -> Vec<String> {
    names(signature.inputs.iter().filter_map(|input| match input {
        syn::FnArg::Typed(t) => Some(&*t.pat),
        syn::FnArg::Receiver(_) => None,
    }))
}

impl<'ast> Visit<'ast> for Visitor<'_> {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        if !is_test(&m.attrs) {
            visit::visit_item_mod(self, m);
        }
    }

    fn visit_item_fn(&mut self, f: &'ast ItemFn) {
        if is_test(&f.attrs) {
            return;
        }
        if !builds_only(f.block.stmts.as_slice()) {
            self.body(&inputs(&f.sig), f.block.span());
        }
        visit::visit_item_fn(self, f);
    }

    fn visit_impl_item_fn(&mut self, f: &'ast ImplItemFn) {
        if !builds_only(f.block.stmts.as_slice()) {
            self.body(&inputs(&f.sig), f.block.span());
        }
        visit::visit_impl_item_fn(self, f);
    }

    fn visit_expr_closure(&mut self, c: &'ast ExprClosure) {
        if !builds(&c.body) {
            self.body(&names(&c.inputs), c.body.span());
        }
        visit::visit_expr_closure(self, c);
    }

    /// A chain is recorded once, from its last call, without what it starts from.
    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        let mut calls = vec![call];
        while let Expr::MethodCall(inner) = &*calls.last().unwrap().receiver {
            calls.push(inner);
        }
        if calls.len() >= LONGEST_CHAIN {
            let chain: Vec<String> =
                calls.iter().rev().map(|c| format!(".{}({})", c.method, self.text(c.args.span()).trim())).collect();
            let at = self.here(call.span());
            self.found.chains.entry(chain.concat()).or_default().push(at);
        }
        visit::visit_expr(self, &calls.last().unwrap().receiver);
        for c in calls {
            for arg in &c.args {
                visit::visit_expr(self, arg);
            }
        }
    }
}

fn sources(dir: &Path, found: &mut Found) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") && path.file_name().is_some_and(|f| f != "tests.rs") {
            let source = fs::read_to_string(&path).unwrap();
            let file = path.strip_prefix(root()).unwrap_or(&path).display().to_string();
            Visitor { file: &file, source: &source, found }.visit_file(&syn::parse_file(&source).unwrap());
        }
    }
}

fn found() -> Found {
    let mut found = Found::default();
    sources(&root().join("src"), &mut found);
    found
}

#[test]
fn no_two_bodies_are_the_same() {
    let same: Vec<String> = found()
        .bodies
        .into_iter()
        .filter(|(_, at)| at.len() > 1)
        .map(|(body, at)| format!("{}\n    {body}", at.join(", ")))
        .collect();
    assert!(same.is_empty(), "the same body in several places:\n{}", same.join("\n"));
}

#[test]
fn no_chain_is_written_out_again_and_again() {
    let repeated: Vec<String> = found()
        .chains
        .into_iter()
        .filter(|(_, at)| at.len() >= CHAIN_REPEATS)
        .map(|(chain, at)| format!("{chain}\n    {}", at.join(", ")))
        .collect();
    assert!(repeated.is_empty(), "the same chain of calls in several places:\n{}", repeated.join("\n"));
}
