//! Primitive reversible edits. Each patch sets one slot (an ordered collection entry or an
//! editor path) and applying it returns its exact inverse, position included, so undo
//! restores the document byte for byte even though removal compacts order (12 §3.1).

use std::collections::BTreeMap;
use std::hash::Hash;

use indexmap::IndexMap;
use serde_json::{Map, Value as Json};

use crate::document::{
    BodyRef, Edge, EventDecl, FunctionDecl, GraphBody, GraphDocument, NodeInstance,
    SignatureSnapshot, VariableDecl,
};
use crate::error::TxError;
use crate::id::{EdgeId, EventDeclId, FunctionId, NodeId, SignatureKey, VariableId};

/// A value and its position in an ordered collection.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Placed<T> {
    pub index: usize,
    pub value: T,
}

/// `None` removes the slot's entry; `Some` inserts or replaces it at the given position.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Patch {
    Node {
        body: BodyRef,
        id: NodeId,
        entry: Option<Placed<Box<NodeInstance>>>,
    },
    Edge {
        body: BodyRef,
        id: EdgeId,
        entry: Option<Placed<Edge>>,
    },
    Variable {
        id: VariableId,
        entry: Option<Placed<VariableDecl>>,
    },
    Event {
        id: EventDeclId,
        entry: Option<Placed<EventDecl>>,
    },
    Function {
        id: FunctionId,
        entry: Option<Placed<Box<FunctionDecl>>>,
    },
    /// Signatures are keyed by content and sorted, so the position is ignored.
    Signature {
        key: SignatureKey,
        entry: Option<Placed<SignatureSnapshot>>,
    },
    Editor {
        path: Vec<String>,
        entry: Option<Placed<Json>>,
    },
}

impl Patch {
    /// Applies the patch and returns its inverse. Fails only if the target body or editor
    /// path cannot hold the slot; the document is then unchanged.
    pub fn apply(self, document: &mut GraphDocument) -> Result<Patch, TxError> {
        Ok(match self {
            Patch::Node { body, id, entry } => {
                let nodes = &mut body_mut(document, body)?.nodes;
                Patch::Node {
                    body,
                    id,
                    entry: set_boxed(nodes, id, entry),
                }
            }
            Patch::Edge { body, id, entry } => {
                let edges = &mut body_mut(document, body)?.edges;
                Patch::Edge {
                    body,
                    id,
                    entry: set_slot(edges, id, entry),
                }
            }
            Patch::Variable { id, entry } => Patch::Variable {
                id,
                entry: set_slot(&mut document.variables, id, entry),
            },
            Patch::Event { id, entry } => Patch::Event {
                id,
                entry: set_slot(&mut document.events, id, entry),
            },
            Patch::Function { id, entry } => Patch::Function {
                id,
                entry: set_boxed(&mut document.functions, id, entry),
            },
            Patch::Signature { key, entry } => {
                let old = set_slot(&mut document.signatures, key.clone(), entry);
                Patch::Signature { key, entry: old }
            }
            Patch::Editor { path, entry } => set_path(&mut document.editor, path, entry)?,
        })
    }

    /// True for edits that only change layout (12 §3.3 `layout_only`).
    pub fn is_layout(&self) -> bool {
        matches!(self, Patch::Editor { .. })
    }
}

fn body_mut(document: &mut GraphDocument, body: BodyRef) -> Result<&mut GraphBody, TxError> {
    match body {
        BodyRef::Root => Ok(&mut document.root),
        BodyRef::Function(id) => document
            .functions
            .get_mut(&id)
            .map(|function| &mut function.body)
            .ok_or(TxError::FunctionNotFound(id)),
    }
}

/// An ordered collection whose entries can be taken out and put back by position.
trait Slots<K, V> {
    fn take(&mut self, key: &K) -> Option<Placed<V>>;
    fn put(&mut self, key: K, entry: Placed<V>);
    fn position(&self, key: &K) -> Option<usize>;
    fn replace(&mut self, key: &K, value: V) -> Option<V>;
}

impl<K: Hash + Eq, V> Slots<K, V> for IndexMap<K, V> {
    fn take(&mut self, key: &K) -> Option<Placed<V>> {
        self.shift_remove_full(key)
            .map(|(index, _, value)| Placed { index, value })
    }

    fn put(&mut self, key: K, entry: Placed<V>) {
        let index = entry.index.min(self.len());
        self.shift_insert(index, key, entry.value);
    }

    fn position(&self, key: &K) -> Option<usize> {
        self.get_index_of(key)
    }

    fn replace(&mut self, key: &K, value: V) -> Option<V> {
        self.get_mut(key).map(|slot| std::mem::replace(slot, value))
    }
}

impl<K: Ord, V> Slots<K, V> for BTreeMap<K, V> {
    fn take(&mut self, key: &K) -> Option<Placed<V>> {
        self.remove(key).map(|value| Placed { index: 0, value })
    }

    fn put(&mut self, key: K, entry: Placed<V>) {
        self.insert(key, entry.value);
    }

    fn position(&self, key: &K) -> Option<usize> {
        self.contains_key(key).then_some(0)
    }

    fn replace(&mut self, key: &K, value: V) -> Option<V> {
        self.get_mut(key).map(|slot| std::mem::replace(slot, value))
    }
}

impl Slots<String, Json> for Map<String, Json> {
    fn take(&mut self, key: &String) -> Option<Placed<Json>> {
        let index = self.keys().position(|existing| existing == key)?;
        self.shift_remove(key).map(|value| Placed { index, value })
    }

    fn put(&mut self, key: String, entry: Placed<Json>) {
        let index = entry.index.min(self.len());
        self.shift_insert(index, key, entry.value);
    }

    fn position(&self, key: &String) -> Option<usize> {
        self.keys().position(|existing| existing == key)
    }

    fn replace(&mut self, key: &String, value: Json) -> Option<Json> {
        self.get_mut(key).map(|slot| std::mem::replace(slot, value))
    }
}

/// [`set_slot`] for large values, which patches hold boxed to keep undo history compact.
fn set_boxed<K: Hash + Eq, V>(
    map: &mut IndexMap<K, V>,
    key: K,
    entry: Option<Placed<Box<V>>>,
) -> Option<Placed<Box<V>>> {
    let entry = entry.map(|placed| Placed {
        index: placed.index,
        value: *placed.value,
    });
    set_slot(map, key, entry).map(|placed| Placed {
        index: placed.index,
        value: Box::new(placed.value),
    })
}

/// Replaces one slot and returns what it held before.
fn set_slot<K, V>(
    slots: &mut impl Slots<K, V>,
    key: K,
    entry: Option<Placed<V>>,
) -> Option<Placed<V>> {
    if let Some(entry) = entry {
        if slots.position(&key) == Some(entry.index) {
            let old = slots
                .replace(&key, entry.value)
                .expect("position implies key exists");
            return Some(Placed {
                index: entry.index,
                value: old,
            });
        }
        let old = slots.take(&key);
        slots.put(key, entry);
        return old;
    }
    slots.take(&key)
}

/// Position an entry should take: its current one when replacing, else the end.
pub(crate) fn placement<K: Hash + Eq, V>(map: &IndexMap<K, V>, key: &K) -> usize {
    map.get_index_of(key).unwrap_or(map.len())
}

/// Position the leaf of `path` should take in the editor map: its current one, else the end
/// of its parent (0 when the parent does not exist yet and will be created).
pub(crate) fn editor_placement(editor: &Map<String, Json>, path: &[String]) -> usize {
    let Some((leaf, parents)) = path.split_last() else {
        return 0;
    };
    let mut current = editor;
    for key in parents {
        match current.get(key).and_then(Json::as_object) {
            Some(child) => current = child,
            None => return 0,
        }
    }
    current
        .keys()
        .position(|key| key == leaf)
        .unwrap_or(current.len())
}

/// Sets an editor path. Missing parent objects are created; the inverse then removes the
/// outermost created parent, which removes everything this patch added.
fn set_path(
    editor: &mut Map<String, Json>,
    path: Vec<String>,
    entry: Option<Placed<Json>>,
) -> Result<Patch, TxError> {
    let Some((leaf, parents)) = path.split_last() else {
        return Err(TxError::EmptyLayoutPath);
    };
    let conflict = || TxError::LayoutPathConflict {
        path: path.join("/"),
    };
    let mut current = editor;
    for (depth, key) in parents.iter().enumerate() {
        match current.get(key) {
            Some(Json::Object(_)) => {}
            Some(_) => return Err(conflict()),
            None => {
                let Some(entry) = entry else {
                    // Removing below a missing parent changes nothing.
                    return Ok(Patch::Editor { path, entry: None });
                };
                let mut value = Json::Object(Map::from_iter([(leaf.clone(), entry.value)]));
                for parent in parents[depth + 1..].iter().rev() {
                    value = Json::Object(Map::from_iter([(parent.clone(), value)]));
                }
                current.insert(key.clone(), value);
                return Ok(Patch::Editor {
                    path: path[..=depth].to_vec(),
                    entry: None,
                });
            }
        }
        current = current
            .get_mut(key)
            .and_then(Json::as_object_mut)
            .ok_or_else(conflict)?;
    }
    let old = set_slot(current, leaf.clone(), entry);
    Ok(Patch::Editor { path, entry: old })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_ordered_eq(
        actual: &IndexMap<&'static str, i32>,
        expected: &IndexMap<&'static str, i32>,
    ) {
        assert_eq!(
            actual.iter().collect::<Vec<_>>(),
            expected.iter().collect::<Vec<_>>()
        );
    }

    #[derive(Default)]
    struct CountingSlots {
        values: IndexMap<&'static str, i32>,
        takes: usize,
        puts: usize,
        replacements: usize,
    }

    impl Slots<&'static str, i32> for CountingSlots {
        fn take(&mut self, key: &&'static str) -> Option<Placed<i32>> {
            self.takes += 1;
            self.values
                .shift_remove_full(key)
                .map(|(index, _, value)| Placed { index, value })
        }

        fn put(&mut self, key: &'static str, entry: Placed<i32>) {
            self.puts += 1;
            self.values
                .shift_insert(entry.index.min(self.values.len()), key, entry.value);
        }

        fn position(&self, key: &&'static str) -> Option<usize> {
            self.values.get_index_of(key)
        }

        fn replace(&mut self, key: &&'static str, value: i32) -> Option<i32> {
            self.replacements += 1;
            self.values
                .get_mut(key)
                .map(|slot| std::mem::replace(slot, value))
        }
    }

    #[test]
    fn same_position_replacement_does_not_remove_or_reinsert() {
        let mut slots = CountingSlots::default();
        slots.values.extend([("a", 1), ("b", 2), ("c", 3)]);

        let old = set_slot(
            &mut slots,
            "b",
            Some(Placed {
                index: 1,
                value: 20,
            }),
        );

        assert_eq!(old, Some(Placed { index: 1, value: 2 }));
        let expected = IndexMap::from([("a", 1), ("b", 20), ("c", 3)]);
        assert_ordered_eq(&slots.values, &expected);
        assert_eq!((slots.takes, slots.puts, slots.replacements), (0, 0, 1));
    }

    #[test]
    fn positional_move_and_inverse_restore_exact_order() {
        let mut slots = IndexMap::from([("a", 1), ("b", 2), ("c", 3), ("d", 4)]);
        let original = slots.clone();

        let inverse = set_slot(
            &mut slots,
            "b",
            Some(Placed {
                index: 3,
                value: 20,
            }),
        )
        .unwrap();
        assert_eq!(
            slots.keys().copied().collect::<Vec<_>>(),
            ["a", "c", "d", "b"]
        );
        let redo = set_slot(&mut slots, "b", Some(inverse)).unwrap();
        assert_ordered_eq(&slots, &original);

        // A rollback applies the inverse of a later edit before the earlier inverse.
        let earlier = set_slot(
            &mut slots,
            "a",
            Some(Placed {
                index: 2,
                value: 10,
            }),
        )
        .unwrap();
        let later = set_slot(&mut slots, "c", None).unwrap();
        set_slot(&mut slots, "c", Some(later));
        set_slot(&mut slots, "a", Some(earlier));
        assert_ordered_eq(&slots, &original);
        assert_eq!(
            redo,
            Placed {
                index: 3,
                value: 20
            }
        );
    }
}
