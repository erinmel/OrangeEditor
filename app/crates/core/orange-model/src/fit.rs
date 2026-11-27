//! Whether a wire of one type fits a port a definition declares, before a node exists:
//! what an editor needs to offer only the kinds that can take a wire, or that can be
//! put into one.
//!
//! A declared type may name the definition's type parameters (`$T`). Each parameter is
//! bound the first time a port meets a concrete type, and every later port of the same
//! candidate must agree with that binding, so `list<$T> -> $T` taking `list<i64>`
//! offers `i64` and nothing else. A binding must satisfy the parameter's declared
//! constraints (11 §5.3), proven by the type registry, and, when the definition lists
//! the argument sets it implements, be one of them. Named types match only by
//! namespace, name, major version and arity: no conversion is assumed. A side that is
//! itself still open (a wire from an unsettled generic node) fits anything here, and
//! the language's checks decide when the wire is made.
//!
//! This is a shortlist for people. Connecting still runs validation and refuses what
//! does not fit with its reason.

use std::collections::BTreeMap;

use crate::catalog::{NodeDefinition, RegistrySnapshot, TypeArgs};
use crate::document::TypeParam;
use crate::id::TypeExpr;
use crate::type_term::TypeTerm;
use crate::types::{CapabilityRequest, TypeRegistry};

/// The type parameter bindings of one candidate node while its ports are matched.
#[derive(Debug, Clone)]
pub struct PortFit<'a> {
    params: &'a [TypeParam],
    specializations: Option<&'a [TypeArgs]>,
    types: &'a TypeRegistry,
    bindings: BTreeMap<String, TypeTerm>,
}

impl<'a> PortFit<'a> {
    /// Starts matching ports of `definition` against `registry`'s types, with nothing
    /// bound yet.
    pub fn new(definition: &'a NodeDefinition, registry: &'a RegistrySnapshot) -> Self {
        Self {
            params: &definition.type_params,
            specializations: registry.specializations(&definition.definition, None),
            types: registry.types(),
            bindings: BTreeMap::new(),
        }
    }

    /// Whether a port declared as `declared` meets a wire of type `wire`, keeping the
    /// bindings this needs. A refusal leaves the bindings as they were. `declared` is
    /// an input taking the wire, or an output going on into an input of type `wire`:
    /// both ask whether the two are the same type once the parameters are bound.
    pub fn meets(&mut self, declared: &TypeExpr, wire: &TypeExpr) -> bool {
        let (Ok(declared), Ok(wire)) = (TypeTerm::parse(declared), TypeTerm::parse(wire)) else {
            return false;
        };
        let mut trial = self.bindings.clone();
        if self.unify(&declared, &wire, &mut trial) && self.specialized(&trial) {
            self.bindings = trial;
            true
        } else {
            false
        }
    }

    /// The type a parameter is bound to so far.
    pub fn bound(&self, param: &str) -> Option<&TypeTerm> {
        self.bindings.get(param)
    }

    fn unify(
        &self,
        declared: &TypeTerm,
        wire: &TypeTerm,
        bindings: &mut BTreeMap<String, TypeTerm>,
    ) -> bool {
        match (declared, wire) {
            // A wire whose own type is still open is decided when it is made.
            (_, TypeTerm::Param(_)) => true,
            (TypeTerm::Param(name), concrete) => match bindings.get(name) {
                Some(bound) => self.unify(&bound.clone(), concrete, bindings),
                None => {
                    if concrete.contains_param() || !self.satisfies(name, concrete) {
                        // An open part inside a concrete wire is left to validation.
                        return concrete.contains_param();
                    }
                    let _ = bindings.insert(name.clone(), concrete.clone());
                    true
                }
            },
            (TypeTerm::Named(left), TypeTerm::Named(right)) => {
                left.namespace == right.namespace
                    && left.name == right.name
                    && left.major == right.major
                    && left.args.len() == right.args.len()
                    && left
                        .args
                        .iter()
                        .zip(&right.args)
                        .all(|(left, right)| self.unify(left, right, bindings))
            }
        }
    }

    /// Whether `ty` satisfies every constraint parameter `name` declares. A parameter
    /// the definition does not declare is not this definition's, and fits nothing.
    fn satisfies(&self, name: &str, ty: &TypeTerm) -> bool {
        let Some(param) = self.params.iter().find(|param| param.name.as_str() == name) else {
            return false;
        };
        param.constraints.iter().all(|constraint| {
            CapabilityRequest::from_declared(constraint)
                .is_some_and(|request| matches!(self.types.supports_term(ty, request), Ok(Some(_))))
        })
    }

    /// With a list of implemented argument sets, the bindings must agree with one.
    fn specialized(&self, bindings: &BTreeMap<String, TypeTerm>) -> bool {
        let Some(sets) = self.specializations else {
            return true;
        };
        sets.iter().any(|set| {
            bindings.iter().all(|(name, bound)| {
                set.iter()
                    .find(|(param, _)| param.as_str() == name)
                    .is_none_or(|(_, ty)| TypeTerm::parse(ty).is_ok_and(|ty| ty == *bound))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::HostOrigin;

    fn registry() -> std::sync::Arc<RegistrySnapshot> {
        RegistrySnapshot::core(
            HostOrigin::new("1.0.0", format!("sha256:{}", "0".repeat(64))).unwrap(),
        )
    }

    fn ty(text: &str) -> TypeExpr {
        text.parse().unwrap()
    }

    #[test]
    fn a_parameter_binds_once_and_later_ports_must_agree() {
        let registry = registry();
        let params = [TypeParam {
            name: "T".parse().unwrap(),
            constraints: Vec::new(),
            extensions: Default::default(),
        }];
        let mut fit = PortFit {
            params: &params,
            specializations: None,
            types: registry.types(),
            bindings: BTreeMap::new(),
        };
        let list = ty("core/list@1<$T>");
        assert!(
            !fit.meets(&list, &ty("core/i64@1")),
            "a scalar is not a list"
        );
        assert!(fit.bound("T").is_none(), "a refusal binds nothing");
        assert!(fit.meets(&list, &ty("core/list@1<core/i64@1>")));
        assert_eq!(
            fit.bound("T").map(ToString::to_string).as_deref(),
            Some("core/i64@1")
        );
        // List Length goes on as an integer count, not as the list it took.
        assert!(fit.clone().meets(&ty("core/u64@1"), &ty("core/u64@1")));
        assert!(
            !fit.clone()
                .meets(&ty("core/u64@1"), &ty("core/list@1<core/i64@1>"))
        );
        // List Reverse gives back the list it took.
        assert!(fit.clone().meets(&list, &ty("core/list@1<core/i64@1>")));
        assert!(!fit.clone().meets(&list, &ty("core/list@1<core/string@1>")));
        // An open wire is left to validation.
        assert!(fit.meets(&ty("core/string@1"), &ty("$U")));
    }

    #[test]
    fn a_constraint_refuses_a_type_that_does_not_satisfy_it() {
        let registry = registry();
        let params = [TypeParam {
            name: "N".parse().unwrap(),
            constraints: vec!["num".to_owned()],
            extensions: Default::default(),
        }];
        let mut fit = PortFit {
            params: &params,
            specializations: None,
            types: registry.types(),
            bindings: BTreeMap::new(),
        };
        assert!(!fit.meets(&ty("$N"), &ty("core/string@1")));
        assert!(fit.meets(&ty("$N"), &ty("core/f64@1")));
        // A parameter the definition does not declare fits nothing.
        assert!(!fit.meets(&ty("$X"), &ty("core/f64@1")));
    }
}
