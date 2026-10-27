//! Text for users is in the catalog, `i18n/en/rusticlone.ftl`, which `#[derive(Fluent)]` and `t!` read: none is
//! in the code, the templates or `app.js`. Constants under `src/rustic` hold rustic's text, which rusticlone
//! matches, `native` what a user types on each system, and attributes developers' reasons, such as
//! `#[expect(reason)]`. Numbers are named constants or
//! `match self` arms,
//! except -1, 0, 1, 2 and 100: identities, pairs and halves, percent, and C's error return. Code
//! elsewhere passes values.

#![expect(clippy::unwrap_used, reason = "a test fails loudly")]
#![expect(clippy::disallowed_methods, reason = "reads the sources")]

use std::{fs, path::Path};

use proc_macro2::{TokenStream, TokenTree};
use syn::visit::{self, Visit};

/// Two words: English, not a key, a path or a format argument.
fn is_text(literal: &str) -> bool {
    let word = |w: &&str| w.len() > 1 && w.chars().all(char::is_alphabetic);
    literal.split(' ').collect::<Vec<_>>().windows(2).any(|pair| pair.iter().all(word))
}

/// `#[test]`, or `#[cfg(test)]`.
fn is_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| match &a.meta {
        syn::Meta::Path(path) => path.is_ident("test"),
        syn::Meta::List(list) => list.path.is_ident("cfg") && list.tokens.to_string() == "test",
        syn::Meta::NameValue(_) => false,
    })
}

/// Literals outside the places they belong.
#[derive(Default)]
struct Literals {
    texts: Vec<String>,
    numbers: Vec<String>,
    /// Under `src/rustic`, whose constants may hold rustic's text.
    rustic: bool,
}

impl Literals {
    fn literal(&mut self, literal: &syn::Lit) {
        match literal {
            // Raw strings are file contents, such as the example profiles.
            syn::Lit::Str(s) if !s.token().to_string().starts_with('r') && is_text(&s.value()) => {
                self.texts.push(s.value());
            }
            syn::Lit::Int(n) if !matches!(n.base10_digits(), "-1" | "0" | "1" | "2" | "100") => {
                self.numbers.push(n.to_string());
            }
            syn::Lit::Float(n) if !matches!(n.base10_digits(), "0.0" | "1.0" | "100.0") => {
                self.numbers.push(n.to_string());
            }
            // `Lit` is non-exhaustive.
            syn::Lit::Str(_)
            | syn::Lit::ByteStr(_)
            | syn::Lit::CStr(_)
            | syn::Lit::Byte(_)
            | syn::Lit::Char(_)
            | syn::Lit::Int(_)
            | syn::Lit::Float(_)
            | syn::Lit::Bool(_)
            | syn::Lit::Verbatim(_)
            | _ => {}
        }
    }

    /// A constant names a number, and under `src/rustic` rustic's text.
    fn constant(&mut self, visit: impl FnOnce(&mut Self)) {
        let (texts, numbers) = (self.texts.len(), self.numbers.len());
        visit(self);
        self.numbers.truncate(numbers);
        if self.rustic {
            self.texts.truncate(texts);
        }
    }

    fn tokens(&mut self, tokens: TokenStream) {
        let mut tokens = tokens.into_iter().peekable();
        while let Some(token) = tokens.next() {
            match token {
                // Attributes, such as doc comments in a macro's input.
                TokenTree::Punct(p) if p.as_char() == '#' => {
                    tokens.next_if(
                        |t| matches!(t, TokenTree::Group(g) if g.delimiter() == proc_macro2::Delimiter::Bracket),
                    );
                }
                // A lint's reason, for developers.
                TokenTree::Ident(i) if i == "reason" => {
                    tokens.next_if(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == '='));
                    tokens.next_if(|t| matches!(t, TokenTree::Literal(_)));
                }
                TokenTree::Group(group) => self.tokens(group.stream()),
                TokenTree::Literal(literal) => {
                    if let Ok(literal) = syn::parse_str::<syn::Lit>(&literal.to_string()) {
                        self.literal(&literal);
                    }
                }
                TokenTree::Ident(_) | TokenTree::Punct(_) => {}
            }
        }
    }
}

impl<'ast> Visit<'ast> for Literals {
    /// The attributes that once held text for users.
    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        if let syn::Meta::List(list) = &attr.meta
            && ["strum", "error", "cfg_attr"].iter().any(|name| list.path.is_ident(name))
        {
            self.tokens(list.tokens.clone());
        }
    }

    fn visit_item_const(&mut self, c: &'ast syn::ItemConst) {
        self.constant(|v| visit::visit_item_const(v, c));
    }

    fn visit_item_static(&mut self, s: &'ast syn::ItemStatic) {
        self.constant(|v| visit::visit_item_static(v, s));
    }

    fn visit_impl_item_const(&mut self, c: &'ast syn::ImplItemConst) {
        self.constant(|v| visit::visit_impl_item_const(v, c));
    }

    /// Array lengths in types follow from the values.
    fn visit_type(&mut self, _: &'ast syn::Type) {}

    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        if !is_test(&m.attrs) {
            visit::visit_item_mod(self, m);
        }
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if !is_test(&f.attrs) {
            visit::visit_item_fn(self, f);
        }
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if !is_test(&f.attrs) {
            visit::visit_impl_item_fn(self, f);
        }
    }

    /// A `match self` arm may name a number, such as a choice's default.
    fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
        let on_self = matches!(&*m.expr, syn::Expr::Path(p) if p.path.is_ident("self"))
            || matches!(&*m.expr, syn::Expr::Unary(u) if matches!(&*u.expr, syn::Expr::Path(p) if p.path.is_ident("self")));
        let numbers = self.numbers.len();
        visit::visit_expr_match(self, m);
        if on_self {
            self.numbers.truncate(numbers);
        }
    }

    fn visit_lit(&mut self, literal: &'ast syn::Lit) {
        self.literal(literal);
    }

    /// `native(windows, macos, other)` takes what a user types on each system, such as a command.
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        let native = matches!(&*call.func, syn::Expr::Path(p) if p.path.is_ident("native"));
        if !native {
            visit::visit_expr_call(self, call);
        }
    }

    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        // Messages for developers.
        let skipped = ["panic", "unreachable", "debug_assert", "assert"];
        if !skipped.iter().any(|name| m.path.is_ident(name)) {
            self.tokens(m.tokens.clone());
        }
    }
}

/// Every literal out of place in `src`, as `file: literal`.
fn sources(dir: &Path, found: &mut Literals) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") && path.file_name().is_some_and(|f| f != "tests.rs") {
            let rustic = path.starts_with(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/rustic"));
            let mut literals = Literals { rustic, ..Literals::default() };
            literals.visit_file(&syn::parse_file(&fs::read_to_string(&path).unwrap()).unwrap());
            let file = path.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap_or(&path).display().to_string();
            found.texts.extend(literals.texts.into_iter().map(|t| format!("{file}: {t}")));
            found.numbers.extend(literals.numbers.into_iter().map(|n| format!("{file}: {n}")));
        }
    }
}

fn found() -> Literals {
    let mut found = Literals::default();
    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut found);
    found
}

#[test]
fn code_contains_no_literal_text() {
    let texts = found().texts;
    assert!(texts.is_empty(), "text outside the catalog:\n{}", texts.join("\n"));
}

/// A word of at least two letters.
fn has_words(text: &str) -> bool {
    text.split(|c: char| !c.is_alphabetic()).any(|w| w.chars().nth(1).is_some())
}

/// `template` without what askama fills in: `{{ }}`, `{% %}` and `{# #}`.
fn markup(template: &str) -> String {
    let (mut out, mut rest) = (String::new(), template);
    let next = |rest: &str| {
        let opens = [("{{", "}}"), ("{%", "%}"), ("{#", "#}")].into_iter();
        opens.filter_map(|(open, close)| Some((rest.find(open)?, close))).min()
    };
    while let Some((at, close)) = next(rest) {
        let (before, after) = rest.split_at(at);
        out += before;
        rest = after.split_once(close).map_or("", |(_, after)| after);
    }
    out + rest
}

/// The words of `templates` outside askama's expressions: in text, or in the attributes that people read.
#[test]
fn templates_contain_no_text() {
    let mut found = Vec::new();
    for entry in fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("templates")).unwrap() {
        let path = entry.unwrap().path();
        let html = scraper::Html::parse_fragment(&markup(&fs::read_to_string(&path).unwrap()));
        let path = path.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap_or(&path).to_owned();
        for node in html.root_element().descendants() {
            let code = node
                .parent()
                .and_then(|p| p.value().as_element())
                .is_some_and(|e| ["script", "style"].contains(&e.name()));
            // A macro's attributes, such as `hx-post="…"`, read as text outside their element.
            let attributes = |t: &str| t.contains("=\"") || t.contains("='");
            if let Some(text) = node.value().as_text().filter(|t| !code && !attributes(t) && has_words(t)) {
                found.push(format!("{}: {}", path.display(), text.trim()));
            }
            let read = ["title", "aria-label", "placeholder", "alt"];
            let attrs = node.value().as_element().into_iter().flat_map(|e| e.attrs());
            found.extend(
                attrs
                    .filter(|(name, value)| read.contains(name) && has_words(value))
                    .map(|(name, value)| format!("{}: {name}=\"{value}\"", path.display())),
            );
        }
    }
    assert!(found.is_empty(), "text outside the catalog:\n{}", found.join("\n"));
}

/// The sentences of `app.js`: its strings that start with a capital and have two words.
#[test]
fn scripts_contain_no_text() {
    let script = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app.js")).unwrap();
    let code: String =
        script.lines().filter(|l| !l.trim_start().starts_with(['/', '*'])).collect::<Vec<_>>().join("\n");
    let strings = code.split(['"', '`']).skip(1).step_by(2);
    let found: Vec<&str> = strings.filter(|s| s.starts_with(char::is_uppercase) && is_text(s)).collect();
    assert!(found.is_empty(), "text outside the catalog:\n{}", found.join("\n"));
}

#[test]
fn code_contains_no_magic_numbers() {
    let numbers = found().numbers;
    assert!(numbers.is_empty(), "numbers without a name:\n{}", numbers.join("\n"));
}
