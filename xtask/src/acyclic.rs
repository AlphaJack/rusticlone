//! `cargo xtask acyclic`: fails if modules or items depend on each other in a cycle. The edges come
//! from rust-analyzer's [SCIP](https://github.com/sourcegraph/scip) index of the library with every
//! feature and without tests. A module uses what its items name, function bodies included; an item uses
//! what its signatures and fields name, so that a type and a function it returns may call each other.
//! A method, field or variant counts as its type. With `--mermaid`, prints the module graph as a
//! Mermaid flowchart instead.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::Path,
    process::Command,
};

use anyhow::{Context, Result, bail};

/// rust-analyzer's settings: those of `rust-analyzer.*` in an editor.
const CONFIG: &str = r#"{"cargo": {"features": "all"}, "cfg": {"setTest": false}}"#;
/// The binary target, which only calls the library.
const MAIN: &str = "src/main.rs";

type Graph = BTreeMap<String, BTreeSet<String>>;

/// Start line, start column, end line, end column.
type Range = [u64; 4];

struct Occurrence {
    range: Range,
    /// Without the scheme, the package and its version: `engine/Engine#due().`.
    symbol: String,
    definition: bool,
    /// Of a definition: the whole item, body included.
    enclosing: Option<Range>,
    in_body: bool,
}

pub(crate) fn check(mermaid: bool) -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let manifest: toml_edit::DocumentMut = fs::read_to_string(root.join("Cargo.toml"))?.parse()?;
    let name = manifest["package"]["name"].as_str().context("no package name in Cargo.toml")?;
    let (modules, items) = graphs(name, &index(&root, name)?);
    if mermaid {
        print_mermaid(&modules);
        return Ok(());
    }
    for (what, graph) in [("modules", &modules), ("items", &items)] {
        let cycles = cycles(graph);
        if !cycles.is_empty() {
            // Each cycle's uses, one per line.
            let uses = |cycle: &Vec<String>| {
                let within = |from: &&String| graph[*from].iter().filter(|to| cycle.contains(to));
                cycle
                    .iter()
                    .flat_map(|from| within(&from).map(move |to| format!("  {from} → {to}")))
                    .collect::<Vec<_>>()
            };
            let cycles: Vec<String> = cycles.iter().map(|c| uses(c).join("\n")).collect();
            bail!("{what} that depend on each other:\n{}", cycles.join("\n\n"));
        }
        println!("no cycles between {what}");
    }
    Ok(())
}

/// The module graph as a Mermaid flowchart, each module that has others below it a box around them. The
/// graph links only siblings, so a use of a module's child shows as a use of the box.
fn print_mermaid(modules: &Graph) {
    let nodes: BTreeSet<&str> =
        modules.iter().flat_map(|(from, tos)| tos.iter().chain([from])).map(String::as_str).collect();
    let (mut children, mut roots) = (BTreeMap::<&str, Vec<&str>>::new(), Vec::new());
    for &node in &nodes {
        let mut ancestors = std::iter::successors(Some(node), |m| Some(m.rsplit_once("::")?.0)).skip(1);
        match ancestors.find(|a| nodes.contains(a)) {
            Some(parent) => children.entry(parent).or_default().push(node),
            None => roots.push(node),
        }
    }
    println!("flowchart LR");
    for root in roots {
        print_node(root, &children, 1);
    }
    for (from, to) in modules.iter().flat_map(|(from, tos)| tos.iter().map(move |to| (from, to))) {
        println!("    {} --> {}", mermaid_id(from), mermaid_id(to));
    }
}

/// A module, or a box around the modules below it, named as in its parent.
fn print_node(module: &str, children: &BTreeMap<&str, Vec<&str>>, depth: usize) {
    let (indent, name) = ("    ".repeat(depth), module.rsplit("::").next().unwrap_or(module));
    let Some(inner) = children.get(module) else { return println!("{indent}{}[{name}]", mermaid_id(module)) };
    println!("{indent}subgraph {} [{name}]", mermaid_id(module));
    for child in inner {
        print_node(child, children, depth + 1);
    }
    println!("{indent}end");
}

/// Without the crate's name, which every module shares: `engine::status` is `engine_status`.
fn mermaid_id(module: &str) -> String {
    module.split_once("::").map_or(module, |(_, path)| path).replace("::", "_")
}

/// The occurrences in each file of the library that rust-analyzer indexes.
fn index(root: &Path, name: &str) -> Result<Vec<Vec<Occurrence>>> {
    let dir = env::temp_dir().join(format!("{name}-acyclic"));
    fs::create_dir_all(&dir)?;
    let (config, index) = (dir.join("config.json"), dir.join("index.scip"));
    fs::write(&config, CONFIG)?;
    let out = Command::new("rust-analyzer")
        .arg("scip")
        .arg(root)
        .arg("--output")
        .arg(&index)
        .arg("--config-path")
        .arg(&config)
        .arg("--exclude-vendored-libraries")
        .output()
        .context("cannot run rust-analyzer (rustup component add rust-analyzer)")?;
    if !out.status.success() {
        bail!("rust-analyzer failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    let prefix = format!("rust-analyzer cargo {name} ");
    let local = |symbol: &str| Some(symbol.strip_prefix(&prefix)?.split_once(' ')?.1.to_owned());
    let documents: Vec<Vec<Occurrence>> = documents(&fs::read(&index)?)
        .into_iter()
        .filter(|(path, _)| path.starts_with("src/") && path != MAIN)
        .map(|(path, occurrences)| {
            let mut occurrences: Vec<Occurrence> =
                occurrences.into_iter().filter_map(|o| Some(Occurrence { symbol: local(&o.symbol)?, ..o })).collect();
            mark_bodies(&mut occurrences, &fs::read_to_string(root.join(path))?);
            Ok(occurrences)
        })
        .collect::<Result<_>>()?;
    if documents.is_empty() {
        bail!("{} indexes no file of {name}", index.display());
    }
    Ok(documents)
}

/// The modules and the items that use each other.
fn graphs(name: &str, documents: &[Vec<Occurrence>]) -> (Graph, Graph) {
    // Each definition's item: the outermost definition around it that is not a module, and for a method
    // of an inherent `impl` block, which is no definition, its type. A trait's `impl` is an item of its
    // own, which may be in another module than its type.
    let mut owners = BTreeMap::new();
    for occurrences in documents {
        for (definition, range) in definitions(occurrences) {
            let outermost = definitions(occurrences)
                .filter(|(d, r)| !d.symbol.ends_with('/') && contains(*r, range))
                .min_by_key(|(_, r)| ((r[0], r[1]), Reverse((r[2], r[3]))));
            if let Some((item, _)) = outermost {
                let owner = self_type(&item.symbol, occurrences).unwrap_or(&item.symbol);
                owners.insert(definition.symbol.as_str(), owner.as_str());
            }
        }
    }
    let (mut modules, mut items) = (Graph::new(), Graph::new());
    for occurrences in documents {
        for used in occurrences.iter().filter(|o| !o.definition) {
            // What items name, not `use` declarations: those of test modules are indexed too.
            let within = definitions(occurrences)
                .filter(|(_, r)| contains(*r, used.range))
                .max_by_key(|(_, r)| ((r[0], r[1]), Reverse((r[2], r[3]))));
            let Some((within, _)) = within else { continue };
            let (Some(from), Some(to)) = (owners.get(within.symbol.as_str()), owners.get(used.symbol.as_str())) else {
                continue;
            };
            if from == to {
                continue;
            }
            // Between sibling modules below their common ancestor: a module and its ancestors may use each
            // other, as a parent re-exports its children and children use the parent's items. By where the
            // code is, which for a trait's `impl` may not be where its type is.
            let (a, b) = (module(&within.symbol), module(&used.symbol));
            let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
            if common < a.len() && common < b.len() {
                let path = |m: &[&str]| [name].iter().chain(&m[..=common]).copied().collect::<Vec<_>>().join("::");
                modules.entry(path(&a)).or_default().insert(path(&b));
            }
            if !used.in_body {
                let path = |item: &str| {
                    format!("{name}::{}", item.replace('/', "::").trim_end_matches(['#', '(', ')', '.', '!']))
                };
                items.entry(path(from)).or_default().insert(path(to));
            }
        }
    }
    (modules, items)
}

/// Marks what a function's body names: from the first `{` after the function's name to its end, which
/// SCIP does not tell apart from the signature.
fn mark_bodies(occurrences: &mut [Occurrence], source: &str) {
    let lines: Vec<&str> = source.lines().collect();
    let body = |function: &Occurrence| {
        let (end, line, column) = (function.enclosing?, function.range[2], function.range[3]);
        let line = usize::try_from(line).ok()?;
        let (open_line, open_column) = lines.iter().enumerate().skip(line).find_map(|(i, text)| {
            let from = if i == line { usize::try_from(column).ok()? } else { 0 };
            Some((i, from + text.get(from..)?.find('{')?))
        })?;
        let open = [u64::try_from(open_line).ok()?, u64::try_from(open_column).ok()?];
        // A declaration without a body ends before the next item's.
        (open[..] <= end[2..]).then_some([open[0], open[1], end[2], end[3]])
    };
    let bodies: Vec<Range> =
        occurrences.iter().filter(|o| o.definition && o.symbol.ends_with(").")).filter_map(body).collect();
    for o in occurrences {
        o.in_body = bodies.iter().any(|b| contains(*b, o.range));
    }
}

fn definitions(occurrences: &[Occurrence]) -> impl Iterator<Item = (&Occurrence, Range)> {
    occurrences.iter().filter(|o| o.definition).filter_map(|o| Some((o, o.enclosing?)))
}

fn contains(outer: Range, inner: Range) -> bool {
    (outer[0], outer[1]) <= (inner[0], inner[1]) && (inner[2], inner[3]) <= (outer[2], outer[3])
}

/// The type of a method in `impl#[Type]`, not `impl#[Type][Trait]`: the type of that name in the crate
/// that the method's file names, as its `impl` does.
fn self_type<'o>(method: &str, occurrences: &'o [Occurrence]) -> Option<&'o String> {
    let after = method.split_once("impl#[")?.1;
    // `Run<'a>` and `[T]` are quoted.
    let (named, rest) = match after.strip_prefix('`') {
        Some(quoted) => quoted.split_once("`]")?,
        None => after.split_once(']')?,
    };
    if rest.starts_with('[') {
        return None;
    }
    let ty = format!("{}#", named.split('<').next()?);
    occurrences.iter().map(|o| &o.symbol).find(|s| s.rsplit_once('/').map_or(s.as_str(), |(_, last)| last) == ty)
}

/// The modules an item is in: `engine/status/Status#` is in `engine` and `status`.
fn module(item: &str) -> Vec<&str> {
    let mut path: Vec<&str> = item.split(['#', '(', '.', '!', '[']).next().unwrap_or_default().split('/').collect();
    path.pop();
    path
}

/// The path and the occurrences of each file in a SCIP index, which is Protocol Buffers.
fn documents(index: &[u8]) -> Vec<(String, Vec<Occurrence>)> {
    let documents = fields(index).filter_map(|field| if let (2, Value::Bytes(d)) = field { Some(d) } else { None });
    documents
        .map(|document| {
            let (mut path, mut occurrences) = (String::new(), Vec::new());
            for field in fields(document) {
                match field {
                    (1, Value::Bytes(p)) => path = String::from_utf8_lossy(p).into_owned(),
                    (2, Value::Bytes(o)) => occurrences.extend(occurrence(o)),
                    _ => {}
                }
            }
            (path, occurrences)
        })
        .collect()
}

fn occurrence(bytes: &[u8]) -> Option<Occurrence> {
    let (mut range, mut symbol, mut definition, mut enclosing) = (None, String::new(), false, None);
    for field in fields(bytes) {
        match field {
            (1, Value::Bytes(r)) => range = self::range(r),
            (2, Value::Bytes(s)) => symbol = String::from_utf8_lossy(s).into_owned(),
            // The `Definition` bit of `symbol_roles`.
            (3, Value::Int(roles)) => definition = roles & 1 == 1,
            (7, Value::Bytes(r)) => enclosing = self::range(r),
            _ => {}
        }
    }
    Some(Occurrence { range: range?, symbol, definition, enclosing, in_body: false })
}

/// From `[line, column, end column]` on one line, else `[line, column, end line, end column]`.
fn range(mut packed: &[u8]) -> Option<Range> {
    let mut values = Vec::new();
    while !packed.is_empty() {
        values.push(varint(&mut packed)?);
    }
    match values[..] {
        [line, start, end] => Some([line, start, line, end]),
        [line, start, end_line, end] => Some([line, start, end_line, end]),
        _ => None,
    }
}

enum Value<'b> {
    Int(u64),
    Bytes(&'b [u8]),
}

/// A message's field numbers and values, until the bytes end or stop making sense. Fixed-size numbers,
/// which SCIP does not use, are skipped.
fn fields(mut bytes: &[u8]) -> impl Iterator<Item = (u64, Value<'_>)> {
    std::iter::from_fn(move || {
        loop {
            let key = varint(&mut bytes)?;
            let size = match key & 0b111 {
                0 => return Some((key >> 3, Value::Int(varint(&mut bytes)?))),
                1 => 8,
                2 => usize::try_from(varint(&mut bytes)?).ok()?,
                5 => 4,
                _ => return None,
            };
            let (value, rest) = bytes.split_at_checked(size)?;
            bytes = rest;
            if key & 0b111 == 2 {
                return Some((key >> 3, Value::Bytes(value)));
            }
        }
    })
}

fn varint(bytes: &mut &[u8]) -> Option<u64> {
    let mut value = 0;
    for shift in (0..u64::BITS).step_by(7) {
        let (&byte, rest) = bytes.split_first()?;
        *bytes = rest;
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Some(value);
        }
    }
    None
}

/// The strongly connected components of `graph` with more than one node (Tarjan's algorithm).
fn cycles(graph: &Graph) -> Vec<Vec<String>> {
    struct Search<'g> {
        graph: &'g Graph,
        index: BTreeMap<&'g str, usize>,
        stack: Vec<&'g str>,
        found: Vec<Vec<String>>,
    }
    impl<'g> Search<'g> {
        fn visit(&mut self, node: &'g str) -> usize {
            let order = self.index.len();
            self.index.insert(node, order);
            self.stack.push(node);
            let mut low = order;
            for next in self.graph.get(node).into_iter().flatten() {
                low = low.min(match self.index.get(next.as_str()) {
                    None => self.visit(next),
                    Some(&o) if self.stack.contains(&next.as_str()) => o,
                    Some(_) => low,
                });
            }
            if low == order {
                let at = self.stack.iter().rposition(|n| *n == node).unwrap_or_default();
                let component: Vec<String> = self.stack.split_off(at).into_iter().map(str::to_owned).collect();
                if component.len() > 1 {
                    self.found.push(component);
                }
            }
            low
        }
    }
    let mut search = Search { graph, index: BTreeMap::new(), stack: Vec::new(), found: Vec::new() };
    for node in graph.keys() {
        if !search.index.contains_key(node.as_str()) {
            search.visit(node);
        }
    }
    search.found
}

#[cfg(test)]
mod tests {
    #[test]
    fn mutual_uses_are_a_cycle() {
        let edges = [("a", "b"), ("b", "a"), ("c", "a"), ("b", "d")];
        let mut graph = super::Graph::new();
        for (from, to) in edges {
            graph.entry(from.into()).or_default().insert(to.into());
        }
        let mut found = super::cycles(&graph);
        for c in &mut found {
            c.sort();
        }
        assert_eq!(found, [["a", "b"]]);
    }
}
