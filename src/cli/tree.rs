//! Lines drawn as a tree, below a heading.

use crate::log::{self, Level, Tone};
/// A line, and the lines below it.
#[derive(Clone)]
pub(super) struct Tree {
    pub(super) line: String,
    pub(super) below: Vec<Tree>,
}

impl Tree {
    pub(super) fn leaf(line: String) -> Self {
        Self { line, below: Vec::new() }
    }
}

/// Branches below a heading, drawn as a tree.
pub(super) fn tree(branches: &[Tree]) -> String {
    draw(branches, "")
}

fn draw(branches: &[Tree], indent: &str) -> String {
    let last = branches.len().saturating_sub(1);
    let mut out = String::new();
    for (i, b) in branches.iter().enumerate() {
        let (stem, next) = if i == last { ("└──", "   ") } else { ("├──", "│  ") };
        let fork = if b.below.is_empty() { " " } else { "┐" };
        out += &format!("{indent}{stem}{fork}{}\n", b.line);
        out += &draw(&b.below, &format!("{indent}{next}"));
    }
    out
}

/// A message at `level`: its first line with its dot, then the others below it.
pub(in crate::cli) fn branch(level: Level, text: &str) -> Tree {
    let mut text = text.lines();
    let first = log::paint(level, text.next().unwrap_or_default());
    Tree { line: format!("{} {first}", Tone::from(level).dot()), below: below(text) }
}

/// A message's lines after its first. A line that starts with a level's emoji, as a notification's issues,
/// goes below the line before it that does not, with that level's dot instead.
pub(in crate::cli) fn below(lines: std::str::Lines<'_>) -> Vec<Tree> {
    let mut branches: Vec<Tree> = Vec::new();
    for line in lines {
        let dotted = [Level::Success, Level::Warning, Level::Failure]
            .into_iter()
            .find_map(|level| Some((level, line.strip_prefix(level.emoji())?.trim_start())));
        let Some((level, text)) = dotted else {
            branches.push(Tree::leaf(line.to_owned()));
            continue;
        };
        let leaf = Tree::leaf(format!("{} {}", level.dot(), log::paint(level, text)));
        match branches.last_mut() {
            Some(parent) => parent.below.push(leaf),
            None => branches.push(leaf),
        }
    }
    branches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_branches_keep_their_stems() {
        let branch = |line: &str, below| Tree { line: line.into(), below };
        let leaf = |line: &str| Tree::leaf(line.into());
        let drawn = tree(&[branch("a", vec![leaf("a1"), branch("a2", vec![leaf("a2i")])]), leaf("b")]);
        assert_eq!(drawn, "├──┐a\n│  ├── a1\n│  └──┐a2\n│     └── a2i\n└── b\n");
        assert_eq!(tree(&[]), "");
    }
}
