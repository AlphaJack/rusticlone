//! Text for users, from the English catalog `i18n/en/<package>.ftl`, checked at build time: each message
//! exists and is given exactly the variables it uses, and each translation has only English messages, which
//! use only English's variables.

#![expect(clippy::disallowed_methods, reason = "reads the catalog and Cargo's variables at build time")]

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::LazyLock,
};

use fluent_syntax::ast::{Entry, Expression, InlineExpression, Pattern, PatternElement};
use heck::ToKebabCase;
use proc_macro2::{Span, TokenStream, TokenTree};
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Expr, Fields, Ident, LitStr, Token, parse::Parse, parse_macro_input};

/// Where the catalogs are, one folder per language.
const FOLDER: &str = "i18n";
/// The language of the catalog that holds every message, which the others fall back to.
const FALLBACK: &str = "en";

fn folder() -> PathBuf {
    PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default()).join(FOLDER)
}

/// The path of `language`'s catalog.
fn catalog_path(language: &str) -> PathBuf {
    folder().join(language).join(format!("{}.ftl", std::env::var("CARGO_PKG_NAME").unwrap_or_default()))
}

/// Each message's variables in the English catalog, or why it cannot be read.
static CATALOG: LazyLock<Result<Catalog, String>> = LazyLock::new(|| read(&catalog_path(FALLBACK)));

/// Each message's variables.
type Catalog = BTreeMap<String, BTreeSet<String>>;

fn read(path: &Path) -> Result<Catalog, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let resource =
        fluent_syntax::parser::parse(text.as_str()).map_err(|(_, errors)| format!("{}: {errors:?}", path.display()))?;
    let patterns: BTreeMap<&str, &Pattern<&str>> = (resource.body.iter())
        .filter_map(|e| match e {
            Entry::Message(m) => Some((m.id.name, m.value.as_ref()?)),
            Entry::Term(_)
            | Entry::Comment(_)
            | Entry::GroupComment(_)
            | Entry::ResourceComment(_)
            | Entry::Junk { .. } => None,
        })
        .collect();
    Ok(patterns.keys().map(|id| ((*id).to_owned(), variables(&patterns, id))).collect())
}

/// The variables of message `id`, and of the messages it references, which take the same arguments.
fn variables(patterns: &BTreeMap<&str, &Pattern<&str>>, id: &str) -> BTreeSet<String> {
    fn inline(e: &InlineExpression<&str>, p: &BTreeMap<&str, &Pattern<&str>>, out: &mut BTreeSet<String>) {
        match e {
            InlineExpression::VariableReference { id } => {
                out.insert(id.name.to_owned());
            }
            InlineExpression::MessageReference { id, .. } => out.extend(variables(p, id.name)),
            InlineExpression::FunctionReference { arguments, .. }
            | InlineExpression::TermReference { arguments: Some(arguments), .. } => {
                for a in (arguments.positional.iter()).chain(arguments.named.iter().map(|n| &n.value)) {
                    inline(a, p, out);
                }
            }
            InlineExpression::Placeable { expression } => expression_vars(expression, p, out),
            // A term takes only the arguments it is given.
            InlineExpression::StringLiteral { .. }
            | InlineExpression::NumberLiteral { .. }
            | InlineExpression::TermReference { arguments: None, .. } => {}
        }
    }
    fn expression_vars(e: &Expression<&str>, p: &BTreeMap<&str, &Pattern<&str>>, out: &mut BTreeSet<String>) {
        match e {
            Expression::Inline(e) => inline(e, p, out),
            Expression::Select { selector, variants } => {
                inline(selector, p, out);
                for v in variants {
                    pattern(&v.value, p, out);
                }
            }
        }
    }
    fn pattern(pat: &Pattern<&str>, p: &BTreeMap<&str, &Pattern<&str>>, out: &mut BTreeSet<String>) {
        for element in &pat.elements {
            if let PatternElement::Placeable { expression } = element {
                expression_vars(expression, p, out);
            }
        }
    }
    let mut out = BTreeSet::new();
    if let Some(pat) = patterns.get(id) {
        pattern(pat, patterns, &mut out);
    }
    out
}

/// `crate::i18n::text(id, args)`, once `id` is known to take exactly the variables `given` and `fields`
/// supply: `given` by name, or else a field of that name.
fn message(id: &LitStr, given: &[(Ident, Expr)], fields: &[Ident]) -> syn::Result<TokenStream> {
    let catalog = CATALOG.as_ref().map_err(|e| syn::Error::new(id.span(), e))?;
    let path = catalog_path(FALLBACK);
    let path = path.to_string_lossy();
    let wanted = catalog
        .get(&id.value())
        .ok_or_else(|| syn::Error::new(id.span(), format!("no message `{}` in {path}", id.value())))?;
    if let Some((name, _)) = given.iter().find(|(name, _)| !wanted.contains(&name.to_string())) {
        return Err(syn::Error::new(name.span(), format!("message `{}` has no `${name}`", id.value())));
    }
    let args = wanted.iter().map(|name| {
        let value = match given.iter().find(|(n, _)| n == name) {
            Some((_, value)) => quote!(#value),
            None => match fields.iter().find(|f| *f == name) {
                Some(field) => quote!(#field),
                None => {
                    let e =
                        format!("message `{}` uses `${name}`, which is neither a field nor an argument", id.value());
                    return syn::Error::new(id.span(), e).to_compile_error();
                }
            },
        };
        quote!((#name, {
            use crate::i18n::{Number as _, Text as _};
            (&&crate::i18n::Arg(&(#value))).value()
        }))
    });
    // Rebuilds the crate when the catalog changes.
    Ok(quote!({
        const _: &[u8] = include_bytes!(#path);
        crate::i18n::text(#id, [#(#args),*])
    }))
}

/// `t!("id", name = value, name)`: message `id`, where a bare `name` is a variable of that name.
#[proc_macro]
pub fn t(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    struct Call(LitStr, Vec<(Ident, Expr)>);
    impl Parse for Call {
        fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
            let id = input.parse()?;
            let mut args = Vec::new();
            while input.parse::<Option<Token![,]>>()?.is_some() && !input.is_empty() {
                let name: Ident = input.parse()?;
                let value = if input.parse::<Option<Token![=]>>()?.is_some() {
                    input.parse()?
                } else {
                    syn::parse_quote!(#name)
                };
                args.push((name, value));
            }
            Ok(Self(id, args))
        }
    }
    let Call(id, args) = parse_macro_input!(input as Call);
    message(&id, &args, &[]).unwrap_or_else(syn::Error::into_compile_error).into()
}

/// `Display` from the catalog: a struct's message is its type's name in kebab case, an enum variant's is
/// prefixed by it, `day-of-week-monday`. Fields are the message's variables.
///
/// On the type: `#[fluent(prefix = "…")]` replaces the type's name. On a variant: `#[fluent(transparent)]`
/// shows its first field as it is, `#[fluent(transparent = expr)]` shows expr, without a message, and `#[fluent(name = expr)]` gives variable `name`, where expr can use the
/// fields by name, or `v0`, `v1` for a tuple's.
#[proc_macro_derive(Fluent, attributes(fluent))]
pub fn derive_fluent(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    derive(&parse_macro_input!(input as DeriveInput)).unwrap_or_else(syn::Error::into_compile_error).into()
}

/// What `#[fluent(…)]` says.
#[derive(Default)]
struct Attrs {
    prefix: Option<LitStr>,
    /// What to show instead of a message: the first field, or an expression.
    transparent: Option<Expr>,
    args: Vec<(Ident, Expr)>,
}

fn attrs(attrs: &[syn::Attribute]) -> syn::Result<Attrs> {
    let mut out = Attrs::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("fluent")) {
        attr.parse_nested_meta(|meta| {
            let name = meta.path.require_ident()?.clone();
            match name.to_string().as_str() {
                "transparent" => {
                    let value = if meta.input.peek(Token![=]) { meta.value()?.parse()? } else { syn::parse_quote!(v0) };
                    out.transparent = Some(value);
                }
                "prefix" => out.prefix = Some(meta.value()?.parse()?),
                _ => out.args.push((name, meta.value()?.parse()?)),
            }
            Ok(())
        })?;
    }
    Ok(out)
}

/// The fields' bindings, `v0` and on for a tuple's, and the pattern that binds those in `used`.
fn bindings(path: &TokenStream, fields: &Fields, used: &BTreeSet<String>) -> (Vec<Ident>, TokenStream) {
    let names: Vec<Ident> = match fields {
        Fields::Named(f) => f.named.iter().filter_map(|f| f.ident.clone()).collect(),
        Fields::Unnamed(f) => (0..f.unnamed.len()).map(|i| format_ident!("v{i}")).collect(),
        Fields::Unit => Vec::new(),
    };
    let bound = |n: &Ident| used.contains(&n.to_string());
    let pattern = match fields {
        Fields::Named(_) => {
            let bound = names.iter().filter(|n| bound(n));
            quote!(#path { #(#bound,)* .. })
        }
        Fields::Unnamed(_) => {
            let each = names.iter().map(|n| if bound(n) { quote!(#n) } else { quote!(_) });
            quote!(#path(#(#each),*))
        }
        Fields::Unit => quote!(#path),
    };
    (names, pattern)
}

/// The identifiers in `tokens`: the fields that an argument's expression uses.
fn idents(tokens: TokenStream, out: &mut BTreeSet<String>) {
    for token in tokens {
        match token {
            TokenTree::Ident(i) => {
                out.insert(i.to_string());
            }
            TokenTree::Group(g) => idents(g.stream(), out),
            TokenTree::Punct(_) | TokenTree::Literal(_) => {}
        }
    }
}

/// An arm's pattern and text, for `fields` at `path` with message `id`.
fn arm(path: &TokenStream, fields: &Fields, attrs: &Attrs, id: &str) -> syn::Result<TokenStream> {
    let mut used: BTreeSet<String> = BTreeSet::new();
    if let Some(shown) = &attrs.transparent {
        idents(quote!(#shown), &mut used);
        let (_, pattern) = bindings(path, fields, &used);
        return Ok(quote!(#pattern => std::fmt::Display::fmt(&#shown, f)));
    }
    let id = LitStr::new(id, Span::call_site());
    if let Ok(catalog) = CATALOG.as_ref()
        && let Some(wanted) = catalog.get(&id.value())
    {
        used.extend(wanted.iter().cloned());
    }
    attrs.args.iter().for_each(|(_, e)| idents(quote!(#e), &mut used));
    let (names, pattern) = bindings(path, fields, &used);
    let text = message(&id, &attrs.args, &names)?;
    Ok(quote!(#pattern => f.write_str(&#text)))
}

fn derive(input: &DeriveInput) -> syn::Result<TokenStream> {
    let ty = &input.ident;
    let top = attrs(&input.attrs)?;
    let prefix = top.prefix.as_ref().map_or_else(|| ty.to_string().to_kebab_case(), LitStr::value);
    let arms = match &input.data {
        Data::Enum(e) => (e.variants.iter())
            .map(|v| {
                let name = &v.ident;
                let id = format!("{prefix}-{}", name.to_string().to_kebab_case());
                arm(&quote!(Self::#name), &v.fields, &attrs(&v.attrs)?, &id)
            })
            .collect::<syn::Result<Vec<_>>>()?,
        Data::Struct(s) => vec![arm(&quote!(Self), &s.fields, &top, &prefix)?],
        Data::Union(u) => return Err(syn::Error::new(u.union_token.span, "unions have no text")),
    };
    let (generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #generics std::fmt::Display for #ty #ty_generics #where_clause {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self { #(#arms,)* }
            }
        }
    })
}

/// What is wrong with the translation at `path`: a message that English lacks, which may be misspelled, or a
/// variable that English does not pass, which would show as `{$name}`. A message it lacks shows in English.
fn mistakes(path: &Path, english: &Catalog) -> Vec<String> {
    let shown = path.display();
    let translation = match read(path) {
        Ok(translation) => translation,
        Err(e) => return vec![e],
    };
    (translation.iter())
        .filter_map(|(id, variables)| {
            let Some(passed) = english.get(id) else {
                return Some(format!("{shown}: `{id}` is not an English message"));
            };
            let unknown: Vec<&String> = variables.difference(passed).collect();
            (!unknown.is_empty()).then(|| format!("{shown}: `{id}` uses {unknown:?}, which English does not pass"))
        })
        .collect()
}

/// `struct Catalogs`, the embedded catalogs, and `const FALLBACK`, the language every message is in. Fails on
/// each translation's [`mistakes`].
#[proc_macro]
pub fn catalogs(_: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let english = match CATALOG.as_ref() {
        Ok(english) => english,
        Err(e) => return syn::Error::new(Span::call_site(), e).into_compile_error().into(),
    };
    let translations: Vec<PathBuf> = (std::fs::read_dir(folder()).into_iter().flatten().flatten())
        .filter(|entry| entry.file_name() != FALLBACK)
        .map(|entry| catalog_path(&entry.file_name().to_string_lossy()))
        .collect();
    let errors = translations.iter().flat_map(|path| mistakes(path, english));
    // Rebuilds the crate when a translation changes.
    let paths = translations.iter().map(|path| path.to_string_lossy().into_owned());
    quote! {
        #(compile_error!(#errors);)*
        #(const _: &[u8] = include_bytes!(#paths);)*
        #[derive(rust_embed::RustEmbed)]
        #[folder = #FOLDER]
        pub(super) struct Catalogs;
        pub(super) const FALLBACK: &str = #FALLBACK;
    }
    .into()
}
