//! Registered type resolution and snapshot-local interning (00c.1).
//!
//! [`TypeRegistry`] resolves the existing parsed [`TypeTerm`] syntax against the v1 core
//! constructor table and interns each resolved expression. A [`TypeKey`] is opaque and
//! carries its registry identity; only the registry that produced it can look it up. Keys
//! are transient implementation identities and must never be persisted in a document,
//! type lock, signature, or other wire format.

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::type_term::{MAX_NESTING, NamedTerm, TypeTerm};

/// The canonical expression's identity within one [`TypeRegistry`].
///
/// The numeric slot is private, and equality includes the owning registry identity. Keys
/// from separate registries never compare equal, even if both registries assigned slot 0.
#[derive(Clone)]
pub struct TypeKey {
    owner: Arc<RegistryIdentity>,
    slot: u32,
}

impl fmt::Debug for TypeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TypeKey")
            .field("slot", &self.slot)
            .finish_non_exhaustive()
    }
}

impl PartialEq for TypeKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.owner, &other.owner) && self.slot == other.slot
    }
}

impl Eq for TypeKey {}

impl Hash for TypeKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.owner) as usize).hash(state);
        self.slot.hash(state);
    }
}

#[derive(Debug)]
struct RegistryIdentity;

/// A small v1 type table and its interner.
///
/// Each instance represents one registry snapshot's type identity space. Resolution
/// accepts parsed terms; it does not parse text or resolve package types. Package type
/// registration is added by 00c.1b using this same representation.
#[derive(Debug)]
pub struct TypeRegistry {
    owner: Arc<RegistryIdentity>,
    interned: HashMap<TypeTerm, TypeKey>,
    expressions: Vec<TypeTerm>,
}

impl Default for TypeRegistry {
    fn default() -> Self {
        Self::core()
    }
}

impl TypeRegistry {
    /// Creates a registry containing the v1 core types and constructors (11 §5.2).
    pub fn core() -> Self {
        Self {
            owner: Arc::new(RegistryIdentity),
            interned: HashMap::new(),
            expressions: Vec::new(),
        }
    }

    /// Resolves and interns one concrete type expression.
    ///
    /// Type parameters remain solver terms and therefore cannot be resolved here. The
    /// nesting budget is checked again during traversal so programmatically constructed
    /// terms receive the same bound as parsed expressions.
    pub fn resolve(&mut self, term: &TypeTerm) -> Result<TypeKey, TypeResolveError> {
        self.validate(term, 0)?;
        if let Some(key) = self.interned.get(term) {
            return Ok(key.clone());
        }
        let slot =
            u32::try_from(self.expressions.len()).map_err(|_| TypeResolveError::TooManyTypes)?;
        let key = TypeKey {
            owner: Arc::clone(&self.owner),
            slot,
        };
        self.expressions.push(term.clone());
        self.interned.insert(term.clone(), key.clone());
        Ok(key)
    }

    /// Returns a key's canonical structural expression, rejecting foreign-registry keys.
    pub fn expression(&self, key: &TypeKey) -> Result<&TypeTerm, TypeResolveError> {
        if !Arc::ptr_eq(&self.owner, &key.owner) {
            return Err(TypeResolveError::ForeignKey);
        }
        self.expressions
            .get(key.slot as usize)
            .ok_or(TypeResolveError::ForeignKey)
    }

    fn validate(&mut self, term: &TypeTerm, depth: usize) -> Result<(), TypeResolveError> {
        if depth > MAX_NESTING {
            return Err(TypeResolveError::TooDeep { max: MAX_NESTING });
        }
        match term {
            TypeTerm::Param(name) => {
                Err(TypeResolveError::UnresolvedParameter { name: name.clone() })
            }
            TypeTerm::Named(named) => {
                let expected = core_arity(named).ok_or_else(|| TypeResolveError::UnknownType {
                    namespace: named.namespace.clone(),
                    name: named.name.clone(),
                    major: named.major,
                })?;
                if named.args.len() != expected {
                    return Err(TypeResolveError::WrongArity {
                        namespace: named.namespace.clone(),
                        name: named.name.clone(),
                        major: named.major,
                        expected,
                        found: named.args.len(),
                    });
                }
                for arg in &named.args {
                    self.validate(arg, depth + 1)?;
                }
                Ok(())
            }
        }
    }
}

fn core_arity(named: &NamedTerm) -> Option<usize> {
    if named.namespace != "core" || named.major != 1 {
        return None;
    }
    match named.name.as_str() {
        "bool" | "i32" | "i64" | "u32" | "u64" | "f32" | "f64" | "string" | "bytes"
        | "duration" | "timestamp" | "error" | "json" | "secret-ref" => Some(0),
        "list" | "option" | "result" | "stream" | "handle" => Some(1),
        "map" => Some(2),
        _ => None,
    }
}

/// Failure to resolve or look up a type in one registry identity space.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TypeResolveError {
    #[error("type `{namespace}/{name}@{major}` is not registered in this registry")]
    UnknownType {
        namespace: String,
        name: String,
        major: u32,
    },
    #[error("type `{namespace}/{name}@{major}` expects {expected} argument(s), found {found}")]
    WrongArity {
        namespace: String,
        name: String,
        major: u32,
        expected: usize,
        found: usize,
    },
    #[error("type parameter `${name}` must be inferred before resolution")]
    UnresolvedParameter { name: String },
    #[error("type expression exceeds the nesting limit of {max}")]
    TooDeep { max: usize },
    #[error("this key does not belong to the registry being queried")]
    ForeignKey,
    #[error("registry contains more types than can be represented by a type key")]
    TooManyTypes,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::TypeExpr;

    fn term(text: &str) -> TypeTerm {
        TypeTerm::parse(&text.parse::<TypeExpr>().unwrap()).unwrap()
    }

    #[test]
    fn resolves_nested_core_types_and_interns_them_per_registry() {
        let expression = term("core/map@1<core/string@1,core/list@1<core/i64@1>>");
        let mut registry = TypeRegistry::core();

        let first = registry.resolve(&expression).unwrap();
        let second = registry.resolve(&expression).unwrap();

        assert_eq!(first, second);
        assert_eq!(registry.expression(&first).unwrap(), &expression);
    }

    #[test]
    fn keeps_numeric_widths_and_float_precisions_distinct() {
        let mut registry = TypeRegistry::core();
        for (left, right) in [
            ("core/i32@1", "core/i64@1"),
            ("core/u32@1", "core/u64@1"),
            ("core/f32@1", "core/f64@1"),
        ] {
            assert_ne!(
                registry.resolve(&term(left)).unwrap(),
                registry.resolve(&term(right)).unwrap()
            );
        }
    }

    #[test]
    fn rejects_unknown_types_and_wrong_constructor_arities() {
        let mut registry = TypeRegistry::core();
        assert!(matches!(
            registry.resolve(&term("core/decimal@1")),
            Err(TypeResolveError::UnknownType { .. })
        ));
        assert!(matches!(
            registry.resolve(&term("core/i64@2")),
            Err(TypeResolveError::UnknownType { major: 2, .. })
        ));
        assert!(matches!(
            registry.resolve(&term("core/list@1<core/i64@1,core/i32@1>")),
            Err(TypeResolveError::WrongArity {
                expected: 1,
                found: 2,
                ..
            })
        ));
        assert!(matches!(
            registry.resolve(&term("core/map@1<core/i64@1>")),
            Err(TypeResolveError::WrongArity {
                expected: 2,
                found: 1,
                ..
            })
        ));
    }

    #[test]
    fn rejects_unresolved_parameters_and_foreign_registry_keys() {
        let mut first_registry = TypeRegistry::core();
        let mut second_registry = TypeRegistry::core();
        let key = first_registry.resolve(&term("core/i64@1")).unwrap();

        assert!(matches!(
            first_registry.resolve(&term("core/list@1<$T>")),
            Err(TypeResolveError::UnresolvedParameter { name }) if name == "T"
        ));
        let other_key = second_registry.resolve(&term("core/i64@1")).unwrap();
        assert_ne!(other_key, key);
        assert!(matches!(
            second_registry.expression(&key),
            Err(TypeResolveError::ForeignKey)
        ));
        assert_eq!(
            second_registry.expression(&other_key).unwrap(),
            &term("core/i64@1")
        );
    }

    #[test]
    fn bounds_programmatically_constructed_terms() {
        let mut deep = TypeTerm::Named(NamedTerm {
            namespace: "core".to_owned(),
            name: "i64".to_owned(),
            major: 1,
            args: Vec::new(),
        });
        for _ in 0..=MAX_NESTING {
            deep = TypeTerm::Named(NamedTerm {
                namespace: "core".to_owned(),
                name: "list".to_owned(),
                major: 1,
                args: vec![deep],
            });
        }
        let mut registry = TypeRegistry::core();

        assert!(matches!(
            registry.resolve(&deep),
            Err(TypeResolveError::TooDeep { max: MAX_NESTING })
        ));
    }
}
