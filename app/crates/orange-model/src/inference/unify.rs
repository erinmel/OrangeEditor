//! Node-scoped structural type unification (WU-00c.3b).
//!
//! This module treats named type expressions as nominal constructors and scopes each
//! definition parameter by its owning node. It does not resolve registered types, evaluate
//! constraints, traverse a graph or mutate a document.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use thiserror::Error;

use crate::document::{PortRef, TypeParamName};
use crate::id::{EdgeId, NodeId};
use crate::type_term::{MAX_NESTING, NamedTerm, TypeTerm};

/// Provisional per-term allocation budget for scoped or substitution-expanded type trees.
/// This is an analysis resource guard, not a restriction on the graph language's valid types.
pub const MAX_EXPANDED_TYPE_NODES: usize = 16_384;

/// One definition parameter in one node instance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeVariable {
    pub node: NodeId,
    pub parameter: TypeParamName,
}

impl TypeVariable {
    pub fn new(node: NodeId, parameter: impl Into<TypeParamName>) -> Self {
        Self {
            node,
            parameter: parameter.into(),
        }
    }
}

impl fmt::Display for TypeVariable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${}@{}", self.parameter, self.node)
    }
}

/// A type term whose parameters are qualified by the node that owns the definition.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScopedTypeTerm {
    Named {
        namespace: String,
        name: String,
        major: u32,
        args: Vec<ScopedTypeTerm>,
    },
    Variable(TypeVariable),
}

impl ScopedTypeTerm {
    /// Scopes every parameter in a definition template to `node` and checks both nesting and
    /// provisional per-term node budgets before constructing the output tree. Terms are
    /// structural; registry arity and ownership checks come later.
    pub fn from_template(node: NodeId, term: &TypeTerm) -> Result<Self, TypeScopeError> {
        fn scope(
            node: NodeId,
            term: &TypeTerm,
            depth: usize,
            node_count: &mut usize,
        ) -> Result<ScopedTypeTerm, TypeScopeError> {
            if depth > MAX_NESTING {
                return Err(TypeScopeError::TooDeep);
            }
            if *node_count == MAX_EXPANDED_TYPE_NODES {
                return Err(TypeScopeError::TooLarge {
                    max_nodes: MAX_EXPANDED_TYPE_NODES,
                });
            }
            *node_count += 1;
            match term {
                TypeTerm::Param(parameter) => Ok(ScopedTypeTerm::Variable(TypeVariable::new(
                    node,
                    parameter.clone(),
                ))),
                TypeTerm::Named(named) => Ok(ScopedTypeTerm::Named {
                    namespace: named.namespace.clone(),
                    name: named.name.clone(),
                    major: named.major,
                    args: named
                        .args
                        .iter()
                        .map(|arg| scope(node, arg, depth + 1, node_count))
                        .collect::<Result<_, _>>()?,
                }),
            }
        }

        scope(node, term, 0, &mut 0)
    }

    /// The variables appearing in this term, before applying a unifier.
    pub fn variables(&self) -> BTreeSet<TypeVariable> {
        let mut variables = BTreeSet::new();
        self.collect_variables(&mut variables);
        variables
    }

    fn collect_variables(&self, variables: &mut BTreeSet<TypeVariable>) {
        match self {
            ScopedTypeTerm::Variable(variable) => {
                variables.insert(variable.clone());
            }
            ScopedTypeTerm::Named { args, .. } => {
                for arg in args {
                    arg.collect_variables(variables);
                }
            }
        }
    }

    fn depth_checked(&self, depth: usize) -> bool {
        if depth > MAX_NESTING {
            return false;
        }
        match self {
            ScopedTypeTerm::Variable(_) => true,
            ScopedTypeTerm::Named { args, .. } => {
                args.iter().all(|arg| arg.depth_checked(depth + 1))
            }
        }
    }

    /// Converts a fully concrete scoped term back to the canonical unscoped syntax tree.
    /// A remaining node-scoped variable is reported instead of losing its owner identity.
    pub fn to_concrete_type_term(&self) -> Result<TypeTerm, ConcreteTypeError> {
        fn convert(
            term: &ScopedTypeTerm,
            depth: usize,
            node_count: &mut usize,
        ) -> Result<TypeTerm, ConcreteTypeError> {
            if depth > MAX_NESTING {
                return Err(ConcreteTypeError::TooDeep);
            }
            if *node_count == MAX_EXPANDED_TYPE_NODES {
                return Err(ConcreteTypeError::TooLarge {
                    max_nodes: MAX_EXPANDED_TYPE_NODES,
                });
            }
            *node_count += 1;
            match term {
                ScopedTypeTerm::Variable(variable) => Err(ConcreteTypeError::Unresolved {
                    variable: variable.clone(),
                }),
                ScopedTypeTerm::Named {
                    namespace,
                    name,
                    major,
                    args,
                } => Ok(TypeTerm::Named(NamedTerm {
                    namespace: namespace.clone(),
                    name: name.clone(),
                    major: *major,
                    args: args
                        .iter()
                        .map(|arg| convert(arg, depth + 1, node_count))
                        .collect::<Result<_, _>>()?,
                })),
            }
        }

        convert(self, 0, &mut 0)
    }
}

impl fmt::Display for ScopedTypeTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopedTypeTerm::Variable(variable) => variable.fmt(f),
            ScopedTypeTerm::Named {
                namespace,
                name,
                major,
                args,
            } => {
                write!(f, "{namespace}/{name}@{major}")?;
                if let Some((first, rest)) = args.split_first() {
                    write!(f, "<{first}")?;
                    for arg in rest {
                        write!(f, ",{arg}")?;
                    }
                    f.write_str(">")?;
                }
                Ok(())
            }
        }
    }
}

/// Why two port/type terms were equated. IDs are retained for typed diagnostics; localized
/// messages are supplied by the host presentation layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EquationEvidence {
    Edge {
        edge: EdgeId,
        from: PortRef,
        to: PortRef,
    },
    Literal {
        port: PortRef,
    },
    Pin {
        node: NodeId,
        parameter: TypeParamName,
    },
    Other,
}

/// Optional graph context attached by the caller to a structural equation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EquationContext {
    pub evidence: Option<EquationEvidence>,
}

/// Equality constraint between two already-scoped terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeEquation {
    pub left: ScopedTypeTerm,
    pub right: ScopedTypeTerm,
    pub context: EquationContext,
}

impl TypeEquation {
    pub fn new(left: ScopedTypeTerm, right: ScopedTypeTerm, context: EquationContext) -> Self {
        Self {
            left,
            right,
            context,
        }
    }
}

/// Failure to scope a manually-constructed type term beyond the parser's nesting budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TypeScopeError {
    #[error("type term exceeds the parser nesting limit")]
    TooDeep,
    #[error("scoped type term exceeds the provisional {max_nodes}-node analysis budget")]
    TooLarge { max_nodes: usize },
}

/// A scoped term cannot be emitted as canonical type syntax until all variables are bound.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConcreteTypeError {
    #[error("type term still contains unresolved parameter `{variable}`")]
    Unresolved { variable: TypeVariable },
    #[error("type term exceeds the parser nesting limit")]
    TooDeep,
    #[error("concrete type term exceeds the provisional {max_nodes}-node analysis budget")]
    TooLarge { max_nodes: usize },
}

/// A structural unification conflict with the originating equation IDs and terms.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UnificationError {
    #[error("incompatible type terms `{left}` and `{right}`")]
    TypeMismatch {
        left: Box<ScopedTypeTerm>,
        right: Box<ScopedTypeTerm>,
        context: Box<EquationContext>,
    },
    #[error("recursive type binding for `{variable}` through `{term}`")]
    OccursCheck {
        variable: TypeVariable,
        term: Box<ScopedTypeTerm>,
        context: Box<EquationContext>,
    },
    #[error("type term exceeds the parser nesting limit")]
    TooDeep {
        context: Option<Box<EquationContext>>,
    },
    #[error("expanded type term exceeds the provisional {max_nodes}-node analysis budget")]
    ExpansionLimit {
        max_nodes: usize,
        context: Option<Box<EquationContext>>,
    },
}

/// A most-general structural substitution. Unbound parameters are absent from `bindings`;
/// aliases are normalized to a deterministic representative.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Unification {
    bindings: BTreeMap<TypeVariable, ScopedTypeTerm>,
}

impl Unification {
    /// Fully applies the substitution to a term. Unbound variables remain node-scoped.
    pub fn resolve(&self, term: &ScopedTypeTerm) -> Result<ScopedTypeTerm, UnificationError> {
        let mut lookup = |variable: &TypeVariable| self.bindings.get(variable).cloned();
        apply_bindings(term, 0, None, &mut 0, &mut lookup)
    }

    /// Node-scoped variables still present after applying this substitution.
    pub fn unresolved_variables(
        &self,
        term: &ScopedTypeTerm,
    ) -> Result<BTreeSet<TypeVariable>, UnificationError> {
        Ok(self.resolve(term)?.variables())
    }

    /// Bindings normalized so each key maps directly to its representative or concrete term.
    pub fn bindings(&self) -> &BTreeMap<TypeVariable, ScopedTypeTerm> {
        &self.bindings
    }
}

/// Unifies an equation set without registry lookups, constraint evaluation or graph access.
///
/// Named constructors match only when namespace, name and major version all agree and their
/// arity is equal. Variable aliases choose the lower `(NodeId, parameter)` as representative,
/// so equivalent equation orders produce the same successful substitution.
pub fn unify(
    equations: impl IntoIterator<Item = TypeEquation>,
) -> Result<Unification, UnificationError> {
    let equations: Vec<TypeEquation> = equations.into_iter().collect();
    let mut variables = BTreeSet::new();
    for equation in &equations {
        if !equation.left.depth_checked(0) || !equation.right.depth_checked(0) {
            return Err(UnificationError::TooDeep {
                context: Some(Box::new(equation.context.clone())),
            });
        }
        variables.extend(equation.left.variables());
        variables.extend(equation.right.variables());
    }

    let mut solver = Solver::default();
    for equation in equations {
        solver.equate(equation.left, equation.right, equation.context)?;
    }

    let mut normalized = BTreeMap::new();
    for variable in variables {
        let resolved = solver.apply(&ScopedTypeTerm::Variable(variable.clone()), 0, None)?;
        if resolved != ScopedTypeTerm::Variable(variable.clone()) {
            normalized.insert(variable, resolved);
        }
    }
    Ok(Unification {
        bindings: normalized,
    })
}

#[derive(Default)]
struct Solver {
    bindings: BTreeMap<TypeVariable, ScopedTypeTerm>,
}

impl Solver {
    fn equate(
        &mut self,
        left: ScopedTypeTerm,
        right: ScopedTypeTerm,
        context: EquationContext,
    ) -> Result<(), UnificationError> {
        let left = self.apply(&left, 0, Some(context.clone()))?;
        let right = self.apply(&right, 0, Some(context.clone()))?;
        match (left, right) {
            (ScopedTypeTerm::Variable(left), ScopedTypeTerm::Variable(right)) => {
                self.bind_variables(left, right);
                Ok(())
            }
            (ScopedTypeTerm::Variable(variable), term)
            | (term, ScopedTypeTerm::Variable(variable)) => self.bind_term(variable, term, context),
            (
                ScopedTypeTerm::Named {
                    namespace: left_namespace,
                    name: left_name,
                    major: left_major,
                    args: left_args,
                },
                ScopedTypeTerm::Named {
                    namespace: right_namespace,
                    name: right_name,
                    major: right_major,
                    args: right_args,
                },
            ) => {
                if left_namespace != right_namespace
                    || left_name != right_name
                    || left_major != right_major
                    || left_args.len() != right_args.len()
                {
                    return Err(UnificationError::TypeMismatch {
                        left: Box::new(ScopedTypeTerm::Named {
                            namespace: left_namespace,
                            name: left_name,
                            major: left_major,
                            args: left_args,
                        }),
                        right: Box::new(ScopedTypeTerm::Named {
                            namespace: right_namespace,
                            name: right_name,
                            major: right_major,
                            args: right_args,
                        }),
                        context: Box::new(context),
                    });
                }
                for (left, right) in left_args.into_iter().zip(right_args) {
                    self.equate(left, right, context.clone())?;
                }
                Ok(())
            }
        }
    }

    fn bind_variables(&mut self, left: TypeVariable, right: TypeVariable) {
        if left == right {
            return;
        }
        let (bound, representative) = if left < right {
            (right, left)
        } else {
            (left, right)
        };
        self.bindings
            .insert(bound, ScopedTypeTerm::Variable(representative));
    }

    fn bind_term(
        &mut self,
        variable: TypeVariable,
        term: ScopedTypeTerm,
        context: EquationContext,
    ) -> Result<(), UnificationError> {
        if term == ScopedTypeTerm::Variable(variable.clone()) {
            return Ok(());
        }
        if occurs_in(&variable, &term) {
            return Err(UnificationError::OccursCheck {
                variable,
                term: Box::new(term),
                context: Box::new(context),
            });
        }
        self.bindings.insert(variable, term);
        Ok(())
    }

    fn apply(
        &mut self,
        term: &ScopedTypeTerm,
        depth: usize,
        context: Option<EquationContext>,
    ) -> Result<ScopedTypeTerm, UnificationError> {
        let mut lookup = |variable: &TypeVariable| self.lookup_and_compress(variable);
        apply_bindings(term, depth, context, &mut 0, &mut lookup)
    }

    /// Follows variable aliases iteratively and compresses the path so large flat generic
    /// components do not consume call-stack depth or repeatedly walk long alias chains.
    fn lookup_and_compress(&mut self, variable: &TypeVariable) -> Option<ScopedTypeTerm> {
        let mut current = variable.clone();
        let mut path = Vec::new();
        loop {
            match self.bindings.get(&current).cloned() {
                Some(ScopedTypeTerm::Variable(next)) if next != current => {
                    path.push(current);
                    current = next;
                }
                Some(bound) => {
                    for alias in path {
                        self.bindings.insert(alias, bound.clone());
                    }
                    return Some(bound);
                }
                None => {
                    let representative = ScopedTypeTerm::Variable(current.clone());
                    for alias in path {
                        self.bindings.insert(alias, representative.clone());
                    }
                    return Some(representative);
                }
            }
        }
    }
}

/// Applies one substitution by resolving variable aliases iteratively and descending only
/// through named-type arguments. The same traversal is used during solving and by consumers.
fn apply_bindings(
    term: &ScopedTypeTerm,
    depth: usize,
    context: Option<EquationContext>,
    expanded_nodes: &mut usize,
    lookup: &mut impl FnMut(&TypeVariable) -> Option<ScopedTypeTerm>,
) -> Result<ScopedTypeTerm, UnificationError> {
    if depth > MAX_NESTING {
        return Err(UnificationError::TooDeep {
            context: context.map(Box::new),
        });
    }
    match term {
        ScopedTypeTerm::Variable(variable) => {
            let mut current = variable.clone();
            loop {
                match lookup(&current) {
                    Some(ScopedTypeTerm::Variable(next)) if next != current => current = next,
                    Some(bound) if bound != ScopedTypeTerm::Variable(current.clone()) => {
                        return apply_bindings(&bound, depth, context, expanded_nodes, lookup);
                    }
                    Some(_) | None => {
                        charge_expanded_node(expanded_nodes, &context)?;
                        return Ok(ScopedTypeTerm::Variable(current));
                    }
                }
            }
        }
        ScopedTypeTerm::Named {
            namespace,
            name,
            major,
            args,
        } => {
            charge_expanded_node(expanded_nodes, &context)?;
            Ok(ScopedTypeTerm::Named {
                namespace: namespace.clone(),
                name: name.clone(),
                major: *major,
                args: args
                    .iter()
                    .map(|arg| {
                        apply_bindings(arg, depth + 1, context.clone(), expanded_nodes, lookup)
                    })
                    .collect::<Result<_, _>>()?,
            })
        }
    }
}

fn charge_expanded_node(
    expanded_nodes: &mut usize,
    context: &Option<EquationContext>,
) -> Result<(), UnificationError> {
    if *expanded_nodes == MAX_EXPANDED_TYPE_NODES {
        return Err(UnificationError::ExpansionLimit {
            max_nodes: MAX_EXPANDED_TYPE_NODES,
            context: context.clone().map(Box::new),
        });
    }
    *expanded_nodes += 1;
    Ok(())
}

fn occurs_in(variable: &TypeVariable, term: &ScopedTypeTerm) -> bool {
    match term {
        ScopedTypeTerm::Variable(candidate) => candidate == variable,
        ScopedTypeTerm::Named { args, .. } => args.iter().any(|arg| occurs_in(variable, arg)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::TypeExpr;
    use proptest::prelude::*;

    fn node(last: u8) -> NodeId {
        format!("10000000-0000-4000-8000-{last:012x}")
            .parse()
            .unwrap()
    }

    fn term(text: &str) -> TypeTerm {
        TypeTerm::parse(&text.parse::<TypeExpr>().unwrap()).unwrap()
    }

    fn scoped(node: NodeId, text: &str) -> ScopedTypeTerm {
        ScopedTypeTerm::from_template(node, &term(text)).unwrap()
    }

    fn equation(left: ScopedTypeTerm, right: ScopedTypeTerm) -> TypeEquation {
        TypeEquation::new(left, right, EquationContext::default())
    }

    #[test]
    fn scopes_nested_parameters_to_their_node_instances() {
        let first = scoped(node(1), "core/map@1<$T,core/list@1<$U>>");
        let second = scoped(node(2), "core/map@1<$T,core/list@1<$U>>");
        assert_ne!(first, second);
        assert_eq!(first.variables().len(), 2);
        assert!(
            first
                .variables()
                .iter()
                .all(|variable| variable.node == node(1))
        );
        assert!(
            second
                .variables()
                .iter()
                .all(|variable| variable.node == node(2))
        );
    }

    #[test]
    fn unifies_nested_terms_and_preserves_unrelated_node_parameters() {
        let first = node(1);
        let second = node(2);
        let third = node(3);
        let equations = [
            equation(
                scoped(first, "core/list@1<$T>"),
                scoped(second, "core/list@1<$U>"),
            ),
            equation(scoped(second, "$U"), scoped(third, "core/i64@1")),
        ];
        let result = unify(equations).unwrap();
        assert_eq!(
            result.resolve(&scoped(first, "core/list@1<$T>")).unwrap(),
            scoped(first, "core/list@1<core/i64@1>")
        );
        assert_eq!(
            result
                .resolve(&scoped(first, "core/list@1<$T>"))
                .unwrap()
                .to_concrete_type_term()
                .unwrap()
                .to_string(),
            "core/list@1<core/i64@1>"
        );
        assert!(
            result
                .bindings()
                .contains_key(&TypeVariable::new(first, "T"))
        );
        assert!(
            !result
                .bindings()
                .contains_key(&TypeVariable::new(node(4), "T"))
        );
        assert_eq!(
            result.unresolved_variables(&scoped(node(4), "$T")).unwrap(),
            BTreeSet::from([TypeVariable::new(node(4), "T")])
        );
    }

    #[test]
    fn conflicts_and_recursive_bindings_keep_equation_context() {
        let edge: EdgeId = "20000000-0000-4000-8000-000000000001".parse().unwrap();
        let from = PortRef {
            node: node(1),
            port: "out".parse().unwrap(),
        };
        let to = PortRef {
            node: node(2),
            port: "input".parse().unwrap(),
        };
        let context = EquationContext {
            evidence: Some(EquationEvidence::Edge {
                edge,
                from: from.clone(),
                to: to.clone(),
            }),
        };
        let mismatch = unify([TypeEquation::new(
            scoped(node(1), "core/i32@1"),
            scoped(node(2), "core/i64@1"),
            context.clone(),
        )])
        .unwrap_err();
        assert!(matches!(
            mismatch,
            UnificationError::TypeMismatch { context: found, .. } if found.as_ref() == &context
        ));

        let variable = scoped(node(1), "$T");
        let recursive = scoped(node(1), "core/list@1<$T>");
        let occurs = unify([TypeEquation::new(variable, recursive, context.clone())]).unwrap_err();
        assert!(matches!(
            occurs,
            UnificationError::OccursCheck { context: found, .. } if found.as_ref() == &context
        ));
    }

    proptest! {
        #[test]
        fn successful_bindings_do_not_depend_on_equation_order(permutation in 0usize..6) {
            let first = node(1);
            let second = node(2);
            let third = node(3);
            let equations = vec![
                equation(scoped(first, "$T"), scoped(second, "$U")),
                equation(scoped(second, "$U"), scoped(third, "core/i64@1")),
                equation(scoped(first, "core/list@1<$T>"), scoped(third, "core/list@1<core/i64@1>")),
            ];
            let forward = unify(equations.clone()).unwrap();
            let permutations = [
                [0, 1, 2], [0, 2, 1], [1, 0, 2],
                [1, 2, 0], [2, 0, 1], [2, 1, 0],
            ];
            let reordered = unify(permutations[permutation].map(|i| equations[i].clone())).unwrap();
            prop_assert_eq!(forward.bindings(), reordered.bindings());
        }
    }

    #[test]
    fn manually_constructed_terms_still_obey_the_nesting_budget() {
        let mut deep = TypeTerm::Named(NamedTerm {
            namespace: "core".into(),
            name: "i64".into(),
            major: 1,
            args: Vec::new(),
        });
        for _ in 0..=MAX_NESTING {
            deep = TypeTerm::Named(NamedTerm {
                namespace: "core".into(),
                name: "list".into(),
                major: 1,
                args: vec![deep],
            });
        }
        assert_eq!(
            ScopedTypeTerm::from_template(node(1), &deep),
            Err(TypeScopeError::TooDeep)
        );

        let mut scoped_deep = ScopedTypeTerm::Variable(TypeVariable::new(node(1), "T"));
        for _ in 0..=MAX_NESTING {
            scoped_deep = ScopedTypeTerm::Named {
                namespace: "core".into(),
                name: "list".into(),
                major: 1,
                args: vec![scoped_deep],
            };
        }
        assert!(matches!(
            unify([equation(
                scoped_deep,
                ScopedTypeTerm::Variable(TypeVariable::new(node(2), "T"))
            )]),
            Err(UnificationError::TooDeep { .. })
        ));
    }

    #[test]
    fn scoping_programmatic_wide_terms_stops_at_the_node_budget() {
        let wide = TypeTerm::Named(NamedTerm {
            namespace: "core".into(),
            name: "tuple".into(),
            major: 1,
            args: (0..=MAX_EXPANDED_TYPE_NODES)
                .map(|_| term("core/i64@1"))
                .collect(),
        });
        assert_eq!(
            ScopedTypeTerm::from_template(node(1), &wide),
            Err(TypeScopeError::TooLarge {
                max_nodes: MAX_EXPANDED_TYPE_NODES
            })
        );
    }

    #[test]
    fn shared_substitutions_stop_before_expanding_past_the_node_budget() {
        const DEPTH: usize = 20;
        let variables: Vec<_> = (0..=DEPTH)
            .map(|index| {
                let scoped_node: NodeId = format!("10000000-0000-4000-8000-{index:012x}")
                    .parse()
                    .unwrap();
                TypeVariable::new(scoped_node, "T")
            })
            .collect();
        let equations = (0..DEPTH).map(|index| {
            equation(
                ScopedTypeTerm::Variable(variables[index].clone()),
                ScopedTypeTerm::Named {
                    namespace: "core".into(),
                    name: "pair".into(),
                    major: 1,
                    args: vec![
                        ScopedTypeTerm::Variable(variables[index + 1].clone()),
                        ScopedTypeTerm::Variable(variables[index + 1].clone()),
                    ],
                },
            )
        });

        assert!(matches!(
            unify(equations),
            Err(UnificationError::ExpansionLimit {
                max_nodes: MAX_EXPANDED_TYPE_NODES,
                ..
            })
        ));
    }

    #[test]
    fn long_alias_chains_resolve_iteratively_and_are_order_independent() {
        const ALIASES: usize = 10_000;
        let variables: Vec<_> = (0..ALIASES)
            .map(|index| {
                let node: NodeId = format!("10000000-0000-4000-8000-{index:012x}")
                    .parse()
                    .unwrap();
                TypeVariable::new(node, "T")
            })
            .collect();
        let equations = |reverse: bool| {
            let indexes: Box<dyn Iterator<Item = usize>> = if reverse {
                Box::new((1..ALIASES).rev())
            } else {
                Box::new(1..ALIASES)
            };
            indexes
                .map(|index| {
                    let (left, right) = if reverse {
                        (index - 1, index)
                    } else {
                        (ALIASES - index, ALIASES - index - 1)
                    };
                    equation(
                        ScopedTypeTerm::Variable(variables[left].clone()),
                        ScopedTypeTerm::Variable(variables[right].clone()),
                    )
                })
                .collect::<Vec<_>>()
        };

        let reverse_order = unify(equations(false)).unwrap();
        let forward_order = unify(equations(true)).unwrap();
        assert_eq!(reverse_order.bindings(), forward_order.bindings());
        assert_eq!(reverse_order.bindings().len(), ALIASES - 1);
        assert_eq!(
            reverse_order
                .resolve(&ScopedTypeTerm::Variable(variables.last().unwrap().clone()))
                .unwrap(),
            ScopedTypeTerm::Variable(variables[0].clone())
        );
    }
}
