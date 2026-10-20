//! Serde adapter: an `IndexMap` keyed by each item's own `id`, persisted as a JSON array in
//! map (creation) order. Use with `#[serde(with = "crate::keyed")]`.

use std::fmt::Display;
use std::hash::Hash;

use indexmap::IndexMap;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An item that carries its own identity.
pub trait Keyed {
    type Key: Copy + Eq + Hash + Display;

    fn key(&self) -> Self::Key;
}

/// Builds a keyed map from items in order; returns the first duplicate key on failure.
pub fn collect<T: Keyed>(
    items: impl IntoIterator<Item = T>,
) -> Result<IndexMap<T::Key, T>, T::Key> {
    let mut map = IndexMap::new();
    for item in items {
        let key = item.key();
        if map.insert(key, item).is_some() {
            return Err(key);
        }
    }
    Ok(map)
}

pub(crate) fn serialize<S, T>(map: &IndexMap<T::Key, T>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Keyed + Serialize,
{
    serializer.collect_seq(map.values())
}

pub(crate) fn deserialize<'de, D, T>(deserializer: D) -> Result<IndexMap<T::Key, T>, D::Error>
where
    D: Deserializer<'de>,
    T: Keyed + Deserialize<'de>,
{
    collect(Vec::<T>::deserialize(deserializer)?)
        .map_err(|key| D::Error::custom(format!("duplicate id `{key}`")))
}
