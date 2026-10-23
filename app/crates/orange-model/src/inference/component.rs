//! Read-only type inference over the generic components an edit touches (11 §5.5, 00c.4a).
//!
//! The caller names the nodes an edit touched (both endpoints of a connect or disconnect,
//! a node whose literal or pin changed). [`infer_components`] collects the connected
//! generic component of each, builds equations from pins, active typed literals, edges and
//! the concrete port types of non-generic neighbours, solves them, then checks constraints
//! and listed specializations. Persisted `type_args` are never an input, so a stale
//! inferred binding cannot act as a pin. The document is never modified; `persist` turns
//! the result into commands.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use thiserror::Error;

use super::unify::{
    ConcreteTypeError, EquationContext, EquationEvidence, ScopedTypeTerm, TypeEquation,
    TypeScopeError, TypeVariable, UnificationError, unify,
};
use crate::GraphDocument;
use crate::catalog::{NodeContract, NodeDefinition, ProviderKey, RegistrySnapshot, TypeArgs};
use crate::document::{
    DefinitionRef, GraphBody, NodeInstance, PortDescriptor, PortKind, PortRef, TypeParam,
    TypeParamName,
};
use crate::error::IdError;
use crate::id::{EdgeId, FunctionId, NodeId, TypeExpr, VariableId};
use crate::index::DocIndex;
use crate::type_term::TypeTerm;
use crate::types::{
    CapabilityError, CapabilityRequest, Constraint, PackageConstraintId, TypeRegistry,
    TypeResolveError,
};

/// Trusted, ephemeral host overlays for concrete graph nodes, consulted before the
/// registry's definition contract. The stored definition identity prevents an overlay
/// surviving a definition-identity rebind or version change.
#[derive(Debug, Clone, Default)]
pub struct NodeOverlays {
    nodes: BTreeMap<NodeId, (DefinitionRef, [u8; 32], NodeContract)>,
    generation: u64,
}

impl NodeOverlays {
    /// Identity of the current contents: zero while empty, process-unique after every
    /// insert and kept by clones. Connection plans record it, so a plan made against
    /// other overlays is stale.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Installs a host-trusted contract for one node without changing ordinary definition
    /// lookup. This is ephemeral evidence for the current document snapshot: rebuild it
    /// after config, provider, lock, or body changes.
    pub fn insert_node(
        &mut self,
        document: &GraphDocument,
        node: &NodeInstance,
        contract: NodeContract,
    ) -> Result<(), StaleNodeTemplate> {
        let Some(current) = document.node(node.id) else {
            return Err(StaleNodeTemplate { node: node.id });
        };
        if !node_metadata_matches(node, current) {
            return Err(StaleNodeTemplate { node: node.id });
        }
        let definition = node.definition.clone();
        let freshness = metadata_fingerprint(document, current)?;
        self.nodes
            .insert(node.id, (definition, freshness, contract));
        self.generation = crate::id::process_unique();
        Ok(())
    }

    /// Returns a node overlay only while its metadata fingerprint still matches. If that
    /// node has no overlay, falls back to the registry's definition contract; stale
    /// overlays return an error instead of falling back. The caller must keep the overlays
    /// scoped to the document metadata from which they were derived.
    pub fn get_for_node<'a>(
        &'a self,
        registry: &'a RegistrySnapshot,
        document: &GraphDocument,
        node: &NodeInstance,
    ) -> Result<Option<Cow<'a, NodeContract>>, StaleNodeTemplate> {
        if let Some(contract) = self.overlay(document, node)? {
            return Ok(Some(Cow::Borrowed(contract)));
        }
        Ok(registry
            .definition(&node.definition)
            .and_then(NodeDefinition::declared_contract)
            .map(Cow::Owned))
    }

    /// The node's own overlay, if any, while its fingerprint still matches.
    fn overlay(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
    ) -> Result<Option<&NodeContract>, StaleNodeTemplate> {
        let Some((definition, expected, contract)) = self.nodes.get(&node.id) else {
            return Ok(None);
        };
        let Some(current) = document.node(node.id) else {
            return Err(StaleNodeTemplate { node: node.id });
        };
        let actual = metadata_fingerprint(document, current)?;
        if definition != &current.definition
            || !node_metadata_matches(node, current)
            || actual != *expected
        {
            return Err(StaleNodeTemplate { node: node.id });
        }
        Ok(Some(contract))
    }

    /// The contract of a node whose definition declares at least one type parameter.
    fn generic<'a>(
        &'a self,
        registry: &'a RegistrySnapshot,
        document: &GraphDocument,
        node: &NodeInstance,
    ) -> Option<Cow<'a, NodeContract>> {
        self.get_for_node(registry, document, node)
            .ok()
            .flatten()
            .filter(|contract| !contract.type_params.is_empty())
    }

    /// Fails on the first stale overlay in stable NodeId order. Removed nodes are ignored:
    /// their overlays cannot be consulted by inference for this document.
    pub fn ensure_fresh(&self, document: &GraphDocument) -> Result<(), StaleNodeTemplate> {
        for id in self.nodes.keys() {
            if let Some(node) = document.node(*id) {
                self.overlay(document, node)?;
            }
        }
        Ok(())
    }
}

fn node_metadata_matches(candidate: &NodeInstance, current: &NodeInstance) -> bool {
    candidate.definition == current.definition
        && candidate.config_version == current.config_version
        && candidate.config == current.config
        && candidate.provider == current.provider
}

/// A contextual template no longer matches the graph metadata it was resolved against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("node-scoped template for node {node} is stale")]
pub struct StaleNodeTemplate {
    pub node: NodeId,
}

fn metadata_fingerprint(
    document: &GraphDocument,
    node: &NodeInstance,
) -> Result<[u8; 32], StaleNodeTemplate> {
    let definition = node.definition.id.as_str();
    let context = match definition {
        "core/call" => call_context(document, node),
        "core/get-variable"
        | "core/set-variable"
        | "core/update-variable"
        | "core/set-shared"
        | "core/update-shared" => {
            let declaration = node
                .config
                .get("variable")
                .and_then(serde_json::Value::as_str)
                .and_then(|id| id.parse::<VariableId>().ok())
                .and_then(|id| document.variables().get(&id));
            serde_json::json!({"kind":"variable","declaration":declaration})
        }
        "core/on-event" | "core/emit" | "core/await-event" => {
            let declaration = node
                .config
                .get("event")
                .and_then(serde_json::Value::as_str)
                .and_then(|id| id.parse::<crate::id::EventDeclId>().ok())
                .and_then(|id| document.events().get(&id));
            serde_json::json!({"kind":"event","declaration":declaration})
        }
        _ => serde_json::json!({"kind":"other"}),
    };
    let locks: Vec<_> = node
        .provider
        .as_ref()
        .map(|binding| {
            document
                .package_lock()
                .iter()
                .filter(|entry| entry.id == binding.package && entry.version == binding.version)
                .collect()
        })
        .unwrap_or_default();
    let evidence = serde_json::json!([
        document.graph_id(),
        node.definition,
        node.config_version,
        node.config,
        node.provider,
        locks,
        context
    ]);
    let bytes = serde_json::to_vec(&evidence).map_err(|_| StaleNodeTemplate { node: node.id })?;
    Ok(Sha256::digest(bytes).into())
}

fn call_context(document: &GraphDocument, node: &NodeInstance) -> serde_json::Value {
    let target_value = node.config.get("function");
    let Some(target) = target_value
        .and_then(serde_json::Value::as_str)
        .and_then(|id| id.parse::<FunctionId>().ok())
    else {
        return serde_json::json!({
            "kind":"call",
            "missing_target":target_value,
            "variables":document.variables(),
            "events":document.events(),
        });
    };

    let mut pending = BTreeSet::from([target]);
    let mut visited = BTreeSet::new();
    let mut declarations = Vec::new();
    while let Some(id) = pending.pop_first() {
        if !visited.insert(id) {
            continue;
        }
        let declaration = document.functions().get(&id);
        declarations.push(serde_json::json!({"id":id,"declaration":declaration}));
        let Some(declaration) = declaration else {
            continue;
        };
        for child in declaration.body.nodes.values() {
            if child.definition.id.as_str() != "core/call" {
                continue;
            }
            if let Some(callee) = child
                .config
                .get("function")
                .and_then(serde_json::Value::as_str)
                .and_then(|id| id.parse::<FunctionId>().ok())
            {
                pending.insert(callee);
            }
        }
    }
    serde_json::json!({
        "kind":"call",
        "functions":declarations,
        "variables":document.variables(),
        "events":document.events(),
    })
}

/// A typed inference outcome, mapped to its 11 §17 code by [`InferenceIssue::code`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InferenceIssue {
    /// Conflicting or recursive equations, or an analysis budget was exhausted.
    #[error(transparent)]
    Unification(#[from] UnificationError),
    /// A template, snapshot, literal or pin type is not valid type syntax, or a pin or
    /// literal names a type parameter.
    #[error("type `{ty}` at node {node} cannot be analyzed")]
    InvalidTerm { node: NodeId, ty: TypeExpr },
    /// A parameter has no evidence after solving; the user pins it.
    #[error("type parameter `{parameter}` of node {node} is ambiguous")]
    Ambiguous {
        node: NodeId,
        parameter: TypeParamName,
    },
    /// The solved type is not registered or not valid in this registry snapshot.
    #[error("type `{ty}` for `{parameter}` of node {node} is not valid: {error}")]
    UnknownType {
        node: NodeId,
        parameter: TypeParamName,
        ty: TypeExpr,
        error: Box<TypeResolveError>,
    },
    /// The solved type does not satisfy a declared parameter constraint.
    #[error("`{ty}` does not satisfy `{constraint}` for `{parameter}` of node {node}")]
    ConstraintUnsatisfied {
        node: NodeId,
        parameter: TypeParamName,
        ty: TypeExpr,
        constraint: String,
        reason: Option<Box<CapabilityError>>,
    },
    /// Every parameter resolved, but the definition lists no matching specialization.
    #[error("node {node} has no specialization for its resolved type arguments")]
    NoSpecialization {
        node: NodeId,
        type_args: BTreeMap<TypeParamName, TypeExpr>,
    },
    /// A contextual node template no longer matches the graph metadata it was resolved from.
    #[error(transparent)]
    StaleNodeTemplate(#[from] StaleNodeTemplate),
}

impl InferenceIssue {
    /// The 11 §17 diagnostic code for this outcome.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unification(
                UnificationError::TooDeep { .. } | UnificationError::ExpansionLimit { .. },
            ) => "E-TYPE-COMPLEXITY",
            Self::Unification(_) | Self::InvalidTerm { .. } | Self::UnknownType { .. } => {
                "E-TYPE-MISMATCH"
            }
            Self::Ambiguous { .. } => "E-TYPE-AMBIGUOUS",
            Self::ConstraintUnsatisfied { .. } => "E-CONSTRAINT-UNSATISFIED",
            Self::NoSpecialization { .. } => "E-NO-SPECIALIZATION",
            Self::StaleNodeTemplate(_) => "E-METADATA-STALE",
        }
    }

    /// True for every outcome except an unresolved parameter: the types are invalid, not
    /// merely incomplete.
    pub fn is_conflict(&self) -> bool {
        !matches!(self, Self::Ambiguous { .. })
    }
}

/// The inference result for one connected generic component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentInference {
    /// Generic nodes in the component, ascending.
    pub nodes: BTreeSet<NodeId>,
    /// Resolved, registry-valid, constraint-satisfying arguments per node. Ambiguous or
    /// invalid parameters are absent; the map is empty when the equations conflict.
    pub type_args: BTreeMap<NodeId, BTreeMap<TypeParamName, TypeExpr>>,
    /// Outcomes in deterministic node and parameter order.
    pub issues: Vec<InferenceIssue>,
}

/// Infers the generic components that contain or neighbour `touched` nodes.
///
/// `index` must be built for `document`. Touched nodes that are not generic contribute
/// their generic data neighbours, so a changed concrete source re-infers its consumers.
/// Components are returned ordered by their smallest node ID; nodes in no returned
/// component, and their persisted `type_args`, are not inspected.
pub fn infer_components(
    document: &GraphDocument,
    index: &DocIndex,
    touched: impl IntoIterator<Item = NodeId>,
    registry: &RegistrySnapshot,
    overlays: &NodeOverlays,
) -> Vec<ComponentInference> {
    if let Err(stale) = overlays.ensure_fresh(document) {
        return vec![ComponentInference {
            nodes: BTreeSet::from([stale.node]),
            type_args: BTreeMap::new(),
            issues: vec![InferenceIssue::StaleNodeTemplate(stale)],
        }];
    }
    let graph = Graph {
        document,
        index,
        registry,
        overlays,
    };
    let mut seeds = BTreeSet::new();
    for node in touched {
        if graph.generic(node).is_some() {
            seeds.insert(node);
        } else {
            seeds.extend(graph.generic_neighbours(node));
        }
    }
    let mut visited = BTreeSet::new();
    let mut components = Vec::new();
    for seed in seeds {
        if visited.contains(&seed) {
            continue;
        }
        let nodes = graph.component(seed);
        visited.extend(nodes.iter().copied());
        components.push(graph.solve(nodes, registry.types()));
    }
    components
}

struct Graph<'a> {
    document: &'a GraphDocument,
    index: &'a DocIndex,
    registry: &'a RegistrySnapshot,
    overlays: &'a NodeOverlays,
}

/// A port's type as seen by inference: scoped to its node when generic, concrete otherwise.
type PortTerm = Result<Option<ScopedTypeTerm>, InferenceIssue>;

impl Graph<'_> {
    fn node(&self, id: NodeId) -> Option<(&GraphBody, &NodeInstance)> {
        let body = self.document.body(self.index.body_of(id)?)?;
        Some((body, body.nodes.get(&id)?))
    }

    fn generic(&self, id: NodeId) -> Option<(&NodeInstance, Cow<'_, NodeContract>)> {
        let (_, node) = self.node(id)?;
        Some((
            node,
            self.overlays.generic(self.registry, self.document, node)?,
        ))
    }

    /// True unless the node's exact provider lists its supported argument sets and `args` is
    /// not one of them. A binding without a lock entry has no exact identity, so it
    /// delegates to constraints; reporting the missing lock entry is the validator's job (I-4).
    fn specialized(&self, node: &NodeInstance, args: &TypeArgs) -> bool {
        let provider = match &node.provider {
            None => None,
            Some(binding) => match ProviderKey::of(self.document, binding) {
                Some(key) => Some(key),
                None => return true,
            },
        };
        self.registry
            .specializations(&node.definition, provider.as_ref())
            .is_none_or(|supported| supported.contains(args))
    }

    /// Generic nodes joined to `id` by a data edge whose type mentions a parameter.
    fn generic_neighbours(&self, id: NodeId) -> BTreeSet<NodeId> {
        let mut neighbours = BTreeSet::new();
        for (_, from, to) in self.data_edges(id) {
            let other = if from.node == id { to.node } else { from.node };
            if self.generic(other).is_some() && self.edge_links_variables(&from, &to) {
                neighbours.insert(other);
            }
        }
        neighbours
    }

    /// Breadth-first in ascending ID order over generic nodes linked by variable-typed edges.
    fn component(&self, seed: NodeId) -> BTreeSet<NodeId> {
        let mut nodes = BTreeSet::from([seed]);
        let mut frontier = BTreeSet::from([seed]);
        while let Some(id) = frontier.pop_first() {
            for neighbour in self.generic_neighbours(id) {
                if nodes.insert(neighbour) {
                    frontier.insert(neighbour);
                }
            }
        }
        nodes
    }

    /// Incident edges of `id` that connect data ports, sorted by edge ID.
    fn data_edges(&self, id: NodeId) -> Vec<(EdgeId, PortRef, PortRef)> {
        let Some((body, _)) = self.node(id) else {
            return Vec::new();
        };
        let edges: Vec<_> = self
            .index
            .incident(id)
            .filter_map(|edge_id| body.edges.get(&edge_id))
            .filter(|edge| self.is_data_port(&edge.from) && self.is_data_port(&edge.to))
            .map(|edge| (edge.id, edge.from.clone(), edge.to.clone()))
            .collect();
        edges
    }

    fn is_data_port(&self, port: &PortRef) -> bool {
        self.declared_port(port)
            .is_some_and(|declared| declared.kind == PortKind::Data)
    }

    fn declared_port(&self, port: &PortRef) -> Option<PortDescriptor> {
        declared_port(self.document, self.registry, self.overlays, port)
    }

    fn edge_links_variables(&self, from: &PortRef, to: &PortRef) -> bool {
        [from, to].into_iter().any(
            |port| matches!(self.port_term(port), Ok(Some(term)) if !term.variables().is_empty()),
        )
    }

    fn port_term(&self, port: &PortRef) -> PortTerm {
        let Some(ty) = self.declared_port(port).and_then(|declared| declared.ty) else {
            return Ok(None);
        };
        let term = parse(port.node, &ty)?;
        let generic = self.generic(port.node).is_some();
        if !generic && term.contains_param() {
            // An unresolved non-generic snapshot carries no usable evidence.
            return Ok(None);
        }
        ScopedTypeTerm::from_template(port.node, &term)
            .map(Some)
            .map_err(budget_exceeded)
    }

    fn solve(&self, nodes: BTreeSet<NodeId>, types: &TypeRegistry) -> ComponentInference {
        let mut result = ComponentInference {
            nodes,
            type_args: BTreeMap::new(),
            issues: Vec::new(),
        };
        let equations = match self.equations(&result.nodes) {
            Ok(equations) => equations,
            Err(issue) => {
                result.issues.push(issue);
                return result;
            }
        };
        let unification = match unify(equations) {
            Ok(unification) => unification,
            Err(error) => {
                result.issues.push(error.into());
                return result;
            }
        };
        for &id in &result.nodes {
            let Some((node, template)) = self.generic(id) else {
                continue;
            };
            let mut args = BTreeMap::new();
            let mut complete = true;
            for parameter in &template.type_params {
                let variable = ScopedTypeTerm::Variable(TypeVariable::new(id, &parameter.name));
                let resolved = unification.resolve(&variable).map_err(InferenceIssue::from);
                match resolved.map(|term| term.to_concrete_type_term()) {
                    Ok(Ok(term)) => match check_argument(types, id, parameter, &term) {
                        Ok(ty) => {
                            args.insert(parameter.name.clone(), ty);
                        }
                        Err(issue) => {
                            complete = false;
                            result.issues.push(issue);
                        }
                    },
                    Ok(Err(ConcreteTypeError::Unresolved { .. })) => {
                        complete = false;
                        result.issues.push(InferenceIssue::Ambiguous {
                            node: id,
                            parameter: parameter.name.clone(),
                        });
                    }
                    Ok(Err(ConcreteTypeError::TooDeep)) => {
                        complete = false;
                        result.issues.push(budget_exceeded(TypeScopeError::TooDeep));
                    }
                    Ok(Err(ConcreteTypeError::TooLarge { max_nodes })) => {
                        complete = false;
                        result
                            .issues
                            .push(budget_exceeded(TypeScopeError::TooLarge { max_nodes }));
                    }
                    Err(issue) => {
                        complete = false;
                        result.issues.push(issue);
                    }
                }
            }
            if complete && !self.specialized(node, &args) {
                result.issues.push(InferenceIssue::NoSpecialization {
                    node: id,
                    type_args: args.clone(),
                });
            }
            if !args.is_empty() {
                result.type_args.insert(id, args);
            }
        }
        result
    }

    /// Equations in node, then pin, literal and edge order. Each edge appears once.
    fn equations(&self, nodes: &BTreeSet<NodeId>) -> Result<Vec<TypeEquation>, InferenceIssue> {
        let mut equations = Vec::new();
        let mut seen_edges = BTreeSet::new();
        for &id in nodes {
            let Some((node, template)) = self.generic(id) else {
                continue;
            };
            for (parameter, ty) in &node.type_pins {
                if !template
                    .type_params
                    .iter()
                    .any(|declared| &declared.name == parameter)
                {
                    continue;
                }
                equations.push(TypeEquation::new(
                    ScopedTypeTerm::Variable(TypeVariable::new(id, parameter)),
                    concrete(id, ty)?,
                    evidence(EquationEvidence::Pin {
                        node: id,
                        parameter: parameter.clone(),
                    }),
                ));
            }
            for (key, literal) in &node.inputs {
                let port = PortRef {
                    node: id,
                    port: key.clone(),
                };
                if self.index.incoming(&port).next().is_some() {
                    continue; // shadowed by a connected edge (11 §4.4)
                }
                if let Some(term) = self.port_term(&port)?
                    && !term.variables().is_empty()
                {
                    equations.push(TypeEquation::new(
                        term,
                        concrete(id, &literal.ty)?,
                        evidence(EquationEvidence::Literal { port }),
                    ));
                }
            }
            for (edge, from, to) in self.data_edges(id) {
                if !seen_edges.insert(edge) {
                    continue;
                }
                let (Some(left), Some(right)) = (self.port_term(&from)?, self.port_term(&to)?)
                else {
                    continue;
                };
                if left.variables().is_empty() && right.variables().is_empty() {
                    continue; // concrete connection checks belong to the validator
                }
                equations.push(TypeEquation::new(
                    left,
                    right,
                    evidence(EquationEvidence::Edge { edge, from, to }),
                ));
            }
        }
        Ok(equations)
    }
}

/// The declared port of a node: its generic template, else its signature snapshot. A
/// generic port's type may still mention `$name` parameters.
pub(crate) fn declared_port<'a>(
    document: &'a GraphDocument,
    registry: &'a RegistrySnapshot,
    overlays: &NodeOverlays,
    port: &PortRef,
) -> Option<PortDescriptor> {
    let node = document.node(port.node)?;
    let template = overlays.get_for_node(registry, document, node);
    let ports = match &template {
        Ok(Some(template)) => &template.ports,
        Ok(None) => &document.signatures().get(node.signature.as_ref()?)?.ports,
        Err(_) => return None,
    };
    ports
        .iter()
        .find(|declared| declared.port_id == *port.port.port())
        .cloned()
}

fn evidence(evidence: EquationEvidence) -> EquationContext {
    EquationContext {
        evidence: Some(evidence),
    }
}

fn parse(node: NodeId, ty: &TypeExpr) -> Result<TypeTerm, InferenceIssue> {
    TypeTerm::parse(ty).map_err(|error| match error {
        IdError::TypeTooDeep { .. } => budget_exceeded(TypeScopeError::TooDeep),
        _ => InferenceIssue::InvalidTerm {
            node,
            ty: ty.clone(),
        },
    })
}

/// A pin or literal type: concrete syntax, scoped to `node` only for error context.
fn concrete(node: NodeId, ty: &TypeExpr) -> Result<ScopedTypeTerm, InferenceIssue> {
    let term = parse(node, ty)?;
    if term.contains_param() {
        return Err(InferenceIssue::InvalidTerm {
            node,
            ty: ty.clone(),
        });
    }
    ScopedTypeTerm::from_template(node, &term).map_err(budget_exceeded)
}

/// A nesting or size budget failure is `E-TYPE-COMPLEXITY`, never a type mismatch.
fn budget_exceeded(error: TypeScopeError) -> InferenceIssue {
    InferenceIssue::Unification(match error {
        TypeScopeError::TooDeep => UnificationError::TooDeep { context: None },
        TypeScopeError::TooLarge { max_nodes } => UnificationError::ExpansionLimit {
            max_nodes,
            context: None,
        },
    })
}

/// Validates a solved argument against the registry and the parameter's constraints.
fn check_argument(
    types: &TypeRegistry,
    node: NodeId,
    parameter: &TypeParam,
    term: &TypeTerm,
) -> Result<TypeExpr, InferenceIssue> {
    let ty: TypeExpr = term
        .to_string()
        .parse()
        .expect("a concrete type term prints as a nonblank type expression");
    types
        .validate(term)
        .map_err(|error| InferenceIssue::UnknownType {
            node,
            parameter: parameter.name.clone(),
            ty: ty.clone(),
            error: Box::new(error),
        })?;
    for constraint in &parameter.constraints {
        let unsatisfied = |reason| InferenceIssue::ConstraintUnsatisfied {
            node,
            parameter: parameter.name.clone(),
            ty: ty.clone(),
            constraint: constraint.clone(),
            reason,
        };
        let Some(request) = capability_request(constraint) else {
            return Err(unsatisfied(None));
        };
        match types.supports_term(term, request) {
            Ok(Some(_)) => {}
            Ok(None) => return Err(unsatisfied(None)),
            Err(error) => return Err(unsatisfied(Some(Box::new(error)))),
        }
    }
    Ok(ty)
}

/// Parses a declared constraint: a core name (`num`) or a package ID (`ns/name@1`).
fn capability_request(constraint: &str) -> Option<CapabilityRequest> {
    if let Some(core) = Constraint::from_id_name(constraint) {
        return Some(CapabilityRequest::Constraint(core));
    }
    let TypeTerm::Named(named) = TypeTerm::parse(&constraint.parse().ok()?).ok()? else {
        return None;
    };
    if !named.args.is_empty() {
        return None;
    }
    PackageConstraintId::new(named.namespace, named.name, named.major)
        .ok()
        .map(CapabilityRequest::PackageConstraint)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn constraint_names_parse_as_core_or_package_requests() {
        assert_eq!(
            capability_request("num"),
            Some(CapabilityRequest::Constraint(Constraint::Num))
        );
        assert!(matches!(
            capability_request("org.example/smooth@1"),
            Some(CapabilityRequest::PackageConstraint(_))
        ));
        assert_eq!(capability_request("numeric"), None);
        assert_eq!(capability_request("core/num@1"), None);
    }

    #[test]
    fn every_issue_maps_to_a_catalogued_code() {
        let node: NodeId = "10000000-0000-4000-8000-000000000001".parse().unwrap();
        let issue = InferenceIssue::Ambiguous {
            node,
            parameter: "T".into(),
        };
        assert_eq!(issue.code(), "E-TYPE-AMBIGUOUS");
        let complexity = InferenceIssue::Unification(UnificationError::TooDeep { context: None });
        assert_eq!(complexity.code(), "E-TYPE-COMPLEXITY");
    }
}
