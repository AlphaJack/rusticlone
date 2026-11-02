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
