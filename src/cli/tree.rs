//! Lines drawn as a tree, below a heading.

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
