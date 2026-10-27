//! JSON that other versions read: the stored ledger, and the status that `/status` and `collections --json`
//! give to programs, such as one that watches several machines.

use serde::{Serialize, Serializer, de::DeserializeOwned};
use serde_json::Value;

/// A shape that changes only through [`Self::MIGRATIONS`], written with `version`: how many it has had.
pub(crate) trait Versioned: Serialize {
    /// Each turns the shape of its index into the next one, for a change that would not read back.
    const MIGRATIONS: &'static [Migration];
}

pub(crate) struct Migration {
    pub up: fn(&mut Value),
    /// The version before it already reads the new shape, and writes the old one: so that going back one
    /// version keeps working.
    pub read_by_previous: bool,
}

/// How many of `T`'s migrations a version must know to read what this version writes.
pub(crate) fn readable_from<T: Versioned>() -> usize {
    let needs = |(i, m): (usize, &Migration)| if m.read_by_previous { i } else { i + 1 };
    T::MIGRATIONS.iter().enumerate().map(needs).max().unwrap_or(0)
}

/// For `#[serde(serialize_with)]`.
pub(crate) fn serialize<T: Versioned, S: Serializer>(value: &T, s: S) -> Result<S::Ok, S::Error> {
    to_value(value).map_err(serde::ser::Error::custom)?.serialize(s)
}

pub(crate) fn to_value<T: Versioned>(value: &T) -> serde_json::Result<Value> {
    let mut value = serde_json::to_value(value)?;
    value["version"] = T::MIGRATIONS.len().into();
    Ok(value)
}

/// Turns `value` into this version's shape. One that a newer version wrote is left as it is, and read as far
/// as this version knows it, without the fields it does not know.
pub(crate) fn migrate<T: Versioned>(value: &mut Value) {
    let version = version(value).unwrap_or(0);
    migrate_from::<T>(value, version);
}

/// [`migrate`], for a value whose version is written elsewhere, such as in the first line of its file.
pub(crate) fn migrate_from<T: Versioned>(value: &mut Value, version: usize) {
    T::MIGRATIONS.iter().skip(version).for_each(|m| (m.up)(value));
}

/// Renames the tag of an enum as serde tags it by default, `"Structure"` or `{"Subset": ..}`, with `name`.
/// Returns what the tag holds.
pub(crate) fn retag(value: &mut Value, name: impl FnOnce(&str) -> String) -> Option<&mut Value> {
    match value {
        Value::String(tag) => {
            *tag = name(tag);
            None
        }
        Value::Object(map) if map.len() == 1 => {
            let tag = map.keys().next()?.clone();
            let held = map.remove(&tag)?;
            Some(map.entry(name(&tag)).or_insert(held))
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => None,
    }
}

/// [`retag`] in kebab-case: `TestRestore` is `test-restore`.
pub(crate) fn kebab(value: &mut Value) -> Option<&mut Value> {
    retag(value, |tag| {
        tag.chars().enumerate().fold(String::new(), |mut kebab, (i, c)| {
            if c.is_uppercase() && i > 0 {
                kebab.push('-');
            }
            kebab.push(c.to_ascii_lowercase());
            kebab
        })
    })
}

pub(crate) fn version(value: &Value) -> Option<usize> {
    value.get("version")?.as_u64().map(|v| usize::try_from(v).unwrap_or(usize::MAX))
}

pub(crate) fn read<T: Versioned + DeserializeOwned>(mut value: Value) -> Option<T> {
    migrate::<T>(&mut value);
    serde_json::from_value(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, PartialEq, Serialize, serde::Deserialize)]
    struct Shape {
        kept: u32,
    }

    impl Versioned for Shape {
        const MIGRATIONS: &'static [Migration] =
            &[Migration { up: |v| v["kept"] = v["old"].take(), read_by_previous: false }];
    }

    #[test]
    fn older_shapes_are_migrated_and_newer_ones_read_as_far_as_known() {
        assert_eq!(read(json!({"old": 1})), Some(Shape { kept: 1 }));
        assert_eq!(read(json!({"version": 1, "kept": 1})), Some(Shape { kept: 1 }));
        assert_eq!(read(json!({"version": 2, "kept": 1, "added": 2})), Some(Shape { kept: 1 }));
    }

    #[test]
    fn a_migration_the_previous_version_reads_does_not_shut_it_out() {
        struct Prepared;
        impl Serialize for Prepared {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_unit()
            }
        }
        impl Versioned for Prepared {
            const MIGRATIONS: &'static [Migration] =
                &[Migration { up: |_| {}, read_by_previous: false }, Migration { up: |_| {}, read_by_previous: true }];
        }
        assert_eq!((readable_from::<Shape>(), readable_from::<Prepared>()), (1, 1));
    }
}
