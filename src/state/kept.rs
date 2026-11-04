//! A stored value, parsed once and kept until its text changes.

/// A value and the stored text it was parsed from.
pub(super) type Kept<V> = Option<(Box<str>, V)>;

/// `kept` if it was parsed from `text`, else `text` parsed.
pub(super) fn reparse<V>(kept: Kept<V>, text: &str, parse: impl FnOnce(&str) -> Option<V>) -> Kept<V> {
    match kept {
        Some((was, value)) if *was == *text => Some((was, value)),
        _ => Some((text.into(), parse(text)?)),
    }
}

#[cfg(test)]
mod tests {
    /// A value is parsed again only when its text changed.
    #[test]
    fn a_value_is_parsed_again_only_when_its_text_changed() {
        let kept = super::reparse(None, "1", |t| t.parse::<u32>().ok());
        let kept = super::reparse(kept, "1", |_| panic!("parsed again"));
        assert_eq!(kept, Some(("1".into(), 1)));
        assert_eq!(super::reparse(kept, "2", |t| t.parse().ok()), Some(("2".into(), 2)));
    }
}
