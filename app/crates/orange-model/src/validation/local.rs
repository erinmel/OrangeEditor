//! Retained results for the two passes whose dependencies are wholly journal-owned.
//!
//! This is deliberately not a second validator. The full pass calls the same unit
//! functions. Refresh still visits current IDs to assemble its result; cache hits
//! avoid schema/type validation, not whole-document traversal.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::RegistrySnapshot;
use crate::document::{BodyRef, GraphDocument};
use crate::edit::{ChangeSet, DeclId};
use crate::id::NodeId;

use super::config::{ConfigSchemas, validate_node_config};
use super::connections::EdgeCache;
use super::declarations::validate_declaration_type;
use super::inputs::InputCache;
use super::nodes::ComponentCache;
use super::{Diagnostic, Subject};

/// Actual unit work during one refresh, including visits that reused a result.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LocalWork {
    /// Current nodes inspected while assembling the configuration buckets.
    pub nodes_visited: usize,
    /// Current variable/event declarations inspected while assembling their buckets.
    pub declarations_visited: usize,
    /// Configuration units whose shared validator ran.
    pub config_computed: usize,
    /// Configuration units reused after their unchanged inputs were established.
    pub config_hits: usize,
    /// Declaration type units whose shared validator ran.
    pub declarations_computed: usize,
    /// Declaration type units reused after their unchanged inputs were established.
    pub declaration_hits: usize,
    /// Node input checks that ran, during acceptance and the final round together.
    pub inputs_computed: usize,
    /// Node input checks reused because the node and everything the check reads were
    /// unchanged.
    pub input_hits: usize,
    /// Edge checks that ran, during acceptance and the final round together.
    pub edges_computed: usize,
    /// Edge checks reused because the edge, its endpoints and their resolutions were
    /// unchanged.
    pub edge_hits: usize,
    /// Generic components whose type inference ran.
    pub components_computed: usize,
    /// Generic components whose inference was reused because nothing they read changed.
    pub component_hits: usize,
}

/// Per-unit results the analysis passes reuse and refill within a session.
#[derive(Default, Clone)]
pub(crate) struct PassCaches {
    pub(crate) inputs: InputCache,
    pub(crate) edges: EdgeCache,
    pub(crate) components: ComponentCache,
}

#[derive(Clone)]
struct ConfigResult {
    body: BodyRef,
    diagnostic: Option<Diagnostic>,
}

/// Registry- and content-bound buckets. No configuration or declaration payload is
/// cloned into the cache. The journal invalidates exactly those owned inputs.
#[derive(Default, Clone)]
pub(crate) struct LocalValidation {
    state: Option<u64>,
    registry_generation: Option<u64>,
    config: BTreeMap<NodeId, ConfigResult>,
    body_nodes: BTreeMap<BodyRef, BTreeSet<NodeId>>,
    declarations: BTreeMap<DeclId, Option<Diagnostic>>,
    schemas: ConfigSchemas,
    passes: PassCaches,
}

impl std::fmt::Debug for LocalValidation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalValidation")
            .field("config_units", &self.config.len())
            .field("declaration_units", &self.declarations.len())
            .field("input_units", &self.passes.inputs.len())
            .field("edge_units", &self.passes.edges.len())
            .field("component_units", &self.passes.components.len())
            .finish_non_exhaustive()
    }
}

impl LocalValidation {
    /// Moves only the content binding after a proven layout transaction. No pass
    /// result depends on layout, and no nodes or declarations are visited here.
    /// A mismatch drops the cache rather than manufacturing continuity.
    pub(crate) fn advance_layout(&mut self, document: &GraphDocument, change: &ChangeSet) -> bool {
        if change.is_committed()
            && change.layout_only()
            && !change.global_dirty()
            && self.state == Some(change.from_state())
            && change.to_state() == document.state_id()
            && change.from_semantic_state() == change.to_semantic_state()
            && change.to_semantic_state() == document.semantic_state_id()
        {
            self.state = Some(document.state_id());
            true
        } else {
            *self = Self::default();
            false
        }
    }

    /// Moves the binding past a node or edge edit the session already checked. A node edit
    /// forgets the edited nodes' input, edge and component results, and the configuration
    /// result of a node whose configuration changed; declaration types read none of them.
    /// An edge edit invalidates through the journal as a full refresh would, without
    /// revisiting configuration. A mismatch drops the cache.
    pub(crate) fn advance_scoped(
        &mut self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        change: &ChangeSet,
    ) {
        let bound = change.is_committed()
            && !change.global_dirty()
            && self.state == Some(change.from_state())
            && change.to_state() == document.state_id()
            && change.to_semantic_state() == document.semantic_state_id()
            && self.registry_generation == Some(registry.generation());
        if bound && change.edge_edit().is_some() {
            self.invalidate(document, change);
            self.state = Some(document.state_id());
            return;
        }
        let Some(edit) = change.node_edit().filter(|_| {
            change.is_committed()
                && !change.global_dirty()
                && self.state == Some(change.from_state())
                && change.to_state() == document.state_id()
                && change.to_semantic_state() == document.semantic_state_id()
                && self.registry_generation == Some(registry.generation())
        }) else {
            *self = Self::default();
            return;
        };
        let nodes: BTreeSet<NodeId> = edit.before.keys().copied().collect();
        for id in &nodes {
            self.passes.inputs.forget(id);
        }
        // A node whose configuration changed is checked again by the next refresh.
        for (id, before) in &edit.before {
            let changed = document.node(*id).is_none_or(|node| {
                node.config != before.config || node.config_version != before.config_version
            });
            if changed
                && let Some(previous) = self.config.remove(id)
                && let Some(body_nodes) = self.body_nodes.get_mut(&previous.body)
            {
                body_nodes.remove(id);
            }
        }
        self.passes.edges.forget_nodes(&nodes);
        self.passes.components.forget(&nodes, &BTreeSet::new());
        self.state = Some(document.state_id());
    }

    /// Refreshes only after the caller's quota and domain prerequisites succeed.
    /// Missing/mismatched provenance or global metadata makes this a full reset.
    pub(crate) fn refresh(
        &mut self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        change: Option<&ChangeSet>,
    ) -> LocalWork {
        let incremental = change.filter(|change| {
            change.is_committed()
                && !change.global_dirty()
                && self.state == Some(change.from_state())
                && change.to_state() == document.state_id()
                && change.to_semantic_state() == document.semantic_state_id()
                && self.registry_generation == Some(registry.generation())
        });
        match incremental {
            Some(change) => self.invalidate(document, change),
            None => {
                self.config.clear();
                self.body_nodes.clear();
                self.declarations.clear();
                self.passes.inputs.clear();
                self.passes.edges.clear();
                self.passes.components.clear();
            }
        }
        let passes = &mut self.passes;
        (passes.inputs.computed, passes.inputs.hits) = (0, 0);
        (passes.edges.computed, passes.edges.hits) = (0, 0);
        (passes.components.computed, passes.components.hits) = (0, 0);

        let mut work = LocalWork::default();
        for (body_ref, body) in document.bodies_with_refs() {
            for node in body.nodes.values() {
                work.nodes_visited += 1;
                match self.config.entry(node.id) {
                    std::collections::btree_map::Entry::Occupied(_) => work.config_hits += 1,
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        work.config_computed += 1;
                        entry.insert(ConfigResult {
                            body: body_ref,
                            diagnostic: validate_node_config(
                                body_ref,
                                node,
                                registry,
                                &mut self.schemas,
                            ),
                        });
                        self.body_nodes.entry(body_ref).or_default().insert(node.id);
                    }
                }
            }
        }
        for variable in document.variables().values() {
            work.declarations_visited += 1;
            self.declaration(
                DeclId::Variable(variable.id),
                &variable.ty,
                Subject::default().with_variable(variable.id),
                registry,
                &mut work,
            );
        }
        for event in document.events().values() {
            work.declarations_visited += 1;
            self.declaration(
                DeclId::Event(event.id),
                &event.payload,
                Subject::default().with_event(event.id),
                registry,
                &mut work,
            );
        }
        self.state = Some(document.state_id());
        self.registry_generation = Some(registry.generation());
        work
    }

    fn declaration(
        &mut self,
        id: DeclId,
        ty: &crate::id::TypeExpr,
        subject: Subject,
        registry: &RegistrySnapshot,
        work: &mut LocalWork,
    ) {
        match self.declarations.entry(id) {
            std::collections::btree_map::Entry::Occupied(_) => work.declaration_hits += 1,
            std::collections::btree_map::Entry::Vacant(entry) => {
                work.declarations_computed += 1;
                entry.insert(validate_declaration_type(ty, subject, registry));
            }
        }
    }

    fn invalidate(&mut self, document: &GraphDocument, change: &ChangeSet) {
        let nodes = change.nodes();
        for id in nodes
            .added
            .iter()
            .chain(&nodes.removed)
            .chain(&nodes.changed)
        {
            self.passes.inputs.forget(id);
            if let Some(previous) = self.config.remove(id)
                && let Some(body_nodes) = self.body_nodes.get_mut(&previous.body)
            {
                body_nodes.remove(id);
                if body_nodes.is_empty() {
                    self.body_nodes.remove(&previous.body);
                }
            }
        }
        let touched: BTreeSet<NodeId> = nodes
            .added
            .iter()
            .chain(&nodes.removed)
            .chain(&nodes.changed)
            .copied()
            .collect();
        self.passes.edges.forget_nodes(&touched);
        let edges = change.edges();
        let touched_edges: BTreeSet<_> = edges
            .added
            .iter()
            .chain(&edges.removed)
            .chain(&edges.changed)
            .copied()
            .collect();
        for id in &touched_edges {
            self.passes.edges.forget(id);
        }
        // A new or rewired edge joins its current endpoints' components.
        let mut reached = touched.clone();
        for id in edges.added.iter().chain(&edges.changed) {
            if let Some(edge) = document.edge(*id) {
                reached.insert(edge.from.node);
                reached.insert(edge.to.node);
            }
        }
        self.passes.components.forget(&reached, &touched_edges);
        let declarations = change.decls();
        // Contracts and call interfaces read declarations, so inference cannot outlive them.
        if !declarations.added.is_empty()
            || !declarations.removed.is_empty()
            || !declarations.changed.is_empty()
        {
            self.passes.components.clear();
        }
        for id in declarations
            .added
            .iter()
            .chain(&declarations.removed)
            .chain(&declarations.changed)
        {
            if let DeclId::Function(function) = id {
                // Edge checks read a body's rigid type parameters from its function.
                self.passes.edges.clear();
                if let Some(nodes) = self.body_nodes.remove(&BodyRef::Function(*function)) {
                    for node in nodes {
                        self.config.remove(&node);
                    }
                }
            } else {
                self.declarations.remove(id);
            }
        }
    }

    /// The retained unit results the analysis passes reuse and fill.
    pub(crate) fn passes(&mut self) -> &mut PassCaches {
        &mut self.passes
    }

    /// Adds the input, edge and inference work of the analysis that just ran to `work`.
    pub(crate) fn record_pass_work(&self, work: &mut LocalWork) {
        work.inputs_computed = self.passes.inputs.computed;
        work.input_hits = self.passes.inputs.hits;
        work.edges_computed = self.passes.edges.computed;
        work.edge_hits = self.passes.edges.hits;
        work.components_computed = self.passes.components.computed;
        work.component_hits = self.passes.components.hits;
    }

    pub(crate) fn config_diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.config
            .values()
            .filter_map(|unit| unit.diagnostic.as_ref())
    }

    pub(crate) fn declaration_diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.declarations.values().filter_map(Option::as_ref)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Determinism, NodeDefinition, Ports, RegistryBuilder};
    use crate::document::{FunctionDecl, NodeInstance, NodeKind, VariableDecl};
    use crate::{DocCommand, GraphDocument};
    use serde_json::{Value, json};

    fn id(n: u8) -> String {
        format!("10000000-0000-4000-8000-{n:012}")
    }

    fn registry() -> std::sync::Arc<RegistrySnapshot> {
        RegistryBuilder::core(
            super::super::HostOrigin::new("1.0.0", format!("sha256:{}", "0".repeat(64))).unwrap(),
        )
        .definition(NodeDefinition {
            definition: crate::document::DefinitionRef {
                id: "example/config".parse().unwrap(),
                version: 1,
            },
            config_version: 1,
            kind: NodeKind::Pure,
            volatility: None,
            determinism: Determinism::Deterministic,
            suspends: None,
            type_params: Vec::new(),
            ports: Ports::Declared(Vec::new()),
            references: Vec::new(),
            specializations: None,
            config_schema: Some(json!({
                "type":"object", "required":["enabled"],
                "properties":{"enabled":{"type":"boolean"}}
            })),
        })
        .build()
        .unwrap()
    }

    fn node(n: u8, config: Value) -> NodeInstance {
        serde_json::from_value(json!({
            "id":id(n), "definition_id":"example/config", "definition_version":1,
            "config_version":1, "config":config
        }))
        .unwrap()
    }

    fn variable(n: u8, ty: &str) -> VariableDecl {
        serde_json::from_value(json!({
            "id":id(n), "name":"not retained in the cache", "type":ty, "scope":"run"
        }))
        .unwrap()
    }

    fn function(n: u8, nodes: Vec<NodeInstance>) -> FunctionDecl {
        serde_json::from_value(json!({
            "id":id(n), "name":"not retained in the cache", "kind":"pure",
            "inputs":[], "outputs":[], "nodes":nodes, "edges":[]
        }))
        .unwrap()
    }

    fn assert_equivalent(
        cache: &LocalValidation,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
    ) {
        let mut config: Vec<_> = cache.config_diagnostics().cloned().collect();
        super::super::sort_diagnostics(&mut config);
        assert_eq!(config, super::super::validate_config(document, registry));
        let mut declarations: Vec<_> = cache.declaration_diagnostics().cloned().collect();
        super::super::sort_diagnostics(&mut declarations);
        assert_eq!(
            declarations,
            super::super::validate_declaration_types(document, registry)
        );
    }

    #[test]
    fn changed_nodes_and_declarations_replace_only_their_own_results() {
        let registry = registry();
        let mut document = GraphDocument::new(id(0).parse().unwrap());
        let _ = document
            .apply([
                DocCommand::AddNode {
                    body: BodyRef::Root,
                    node: node(1, json!({})),
                },
                DocCommand::AddNode {
                    body: BodyRef::Root,
                    node: node(2, json!({"enabled":true})),
                },
                DocCommand::UpsertVariable(variable(3, "unknown/type@1")),
                DocCommand::UpsertVariable(variable(4, "core/bool@1")),
            ])
            .unwrap();
        let mut cache = LocalValidation::default();
        let work = cache.refresh(&document, &registry, None);
        assert_eq!((work.config_computed, work.declarations_computed), (2, 2));
        assert_eq!(cache.config_diagnostics().count(), 1);
        assert_eq!(cache.declaration_diagnostics().count(), 1);
        assert_equivalent(&cache, &document, &registry);

        let committed = document
            .apply([
                DocCommand::SetConfig {
                    node: id(1).parse().unwrap(),
                    config_version: 1,
                    config: serde_json::from_value(json!({"enabled":false})).unwrap(),
                },
                DocCommand::UpsertVariable(variable(3, "core/i64@1")),
            ])
            .unwrap();
        let work = cache.refresh(&document, &registry, Some(&committed.change_set));
        assert_eq!((work.config_computed, work.config_hits), (1, 1));
        assert_eq!((work.declarations_computed, work.declaration_hits), (1, 1));
        assert_eq!((work.nodes_visited, work.declarations_visited), (2, 2));
        assert_equivalent(&cache, &document, &registry);
        assert_eq!(cache.config_diagnostics().count(), 0);
        assert_eq!(cache.declaration_diagnostics().count(), 0);

        let undone = document.revert(committed.inverse).unwrap();
        cache.refresh(&document, &registry, Some(&undone.change_set));
        assert_equivalent(&cache, &document, &registry);
        assert_eq!(cache.config_diagnostics().count(), 1);
        assert_eq!(cache.declaration_diagnostics().count(), 1);
    }

    #[test]
    fn replacing_and_removing_functions_retires_old_subjects_and_nodes() {
        let registry = registry();
        let mut document = GraphDocument::new(id(0).parse().unwrap());
        let _ = document
            .apply([
                DocCommand::AddNode {
                    body: BodyRef::Root,
                    node: node(1, json!({"enabled":true})),
                },
                DocCommand::UpsertFunction(function(9, vec![node(2, json!({}))])),
            ])
            .unwrap();
        let mut cache = LocalValidation::default();
        cache.refresh(&document, &registry, None);
        assert_eq!(
            cache.config_diagnostics().next().unwrap().subject.function,
            Some(id(9).parse().unwrap())
        );

        let replacement = document
            .apply([DocCommand::UpsertFunction(function(
                9,
                vec![node(3, json!({"enabled":true}))],
            ))])
            .unwrap();
        let work = cache.refresh(&document, &registry, Some(&replacement.change_set));
        assert_eq!((work.config_computed, work.config_hits), (1, 1));
        assert_eq!(cache.config_diagnostics().count(), 0);
        assert!(!cache.config.contains_key(&id(2).parse().unwrap()));
        assert_equivalent(&cache, &document, &registry);

        let removal = document
            .apply([DocCommand::RemoveFunction(id(9).parse().unwrap())])
            .unwrap();
        let work = cache.refresh(&document, &registry, Some(&removal.change_set));
        assert_eq!((work.config_computed, work.config_hits), (0, 1));
        assert_eq!(cache.config.len(), 1);
        assert_equivalent(&cache, &document, &registry);

        let undone = document.revert(removal.inverse).unwrap();
        let work = cache.refresh(&document, &registry, Some(&undone.change_set));
        assert_eq!((work.config_computed, work.config_hits), (1, 1));
        assert_equivalent(&cache, &document, &registry);
    }

    #[test]
    fn skipped_lineage_registry_and_global_edits_cannot_reuse_stale_buckets() {
        let registry = registry();
        let mut document = GraphDocument::new(id(0).parse().unwrap());
        let _ = document
            .apply([DocCommand::AddNode {
                body: BodyRef::Root,
                node: node(1, json!({})),
            }])
            .unwrap();
        let mut cache = LocalValidation::default();
        cache.refresh(&document, &registry, None);
        // Skip the semantic edit and present only its following layout edit.
        let _ = document
            .apply([DocCommand::SetConfig {
                node: id(1).parse().unwrap(),
                config_version: 1,
                config: serde_json::from_value(json!({"enabled":true})).unwrap(),
            }])
            .unwrap();
        let change = document
            .apply([DocCommand::Layout(crate::LayoutCommand::Set {
                path: vec!["x".into()],
                value: Some(json!(1)),
            })])
            .unwrap();
        let work = cache.refresh(&document, &registry, Some(&change.change_set));
        assert_eq!((work.config_computed, work.config_hits), (1, 0));
        assert_equivalent(&cache, &document, &registry);

        let change = document
            .apply([DocCommand::PutPackageLock(
                crate::document::PackageLockEntry {
                    id: "example.package".into(),
                    version: "1.0.0".into(),
                    content_digest: format!("sha256:{}", "a".repeat(64)),
                    extensions: Default::default(),
                },
            )])
            .unwrap();
        let work = cache.refresh(&document, &registry, Some(&change.change_set));
        assert_eq!((work.config_computed, work.config_hits), (1, 0));
        assert_equivalent(&cache, &document, &registry);

        let replacement = self::registry();
        let work = cache.refresh(&document, &replacement, Some(&change.change_set));
        assert_eq!((work.config_computed, work.config_hits), (1, 0));
        assert_equivalent(&cache, &document, &replacement);
        assert!(!format!("{cache:?}").contains("enabled"));
    }

    #[test]
    fn layout_binding_advances_without_visits_and_cannot_hide_an_omitted_edit() {
        let registry = registry();
        let mut document = GraphDocument::new(id(0).parse().unwrap());
        let _ = document
            .apply([DocCommand::AddNode {
                body: BodyRef::Root,
                node: node(1, json!({})),
            }])
            .unwrap();
        let mut cache = LocalValidation::default();
        cache.refresh(&document, &registry, None);
        let change = document
            .apply([DocCommand::Layout(crate::LayoutCommand::Set {
                path: vec!["x".into()],
                value: Some(json!(1)),
            })])
            .unwrap();
        assert!(cache.advance_layout(&document, &change.change_set));
        let change = document
            .apply([DocCommand::SetConfig {
                node: id(1).parse().unwrap(),
                config_version: 1,
                config: serde_json::from_value(json!({"enabled":true})).unwrap(),
            }])
            .unwrap();
        let work = cache.refresh(&document, &registry, Some(&change.change_set));
        assert_eq!((work.config_computed, work.config_hits), (1, 0));
        assert_equivalent(&cache, &document, &registry);

        let _ = document
            .apply([DocCommand::SetConfig {
                node: id(1).parse().unwrap(),
                config_version: 1,
                config: serde_json::from_value(json!({})).unwrap(),
            }])
            .unwrap();
        let change = document
            .apply([DocCommand::Layout(crate::LayoutCommand::Set {
                path: vec!["x".into()],
                value: Some(json!(2)),
            })])
            .unwrap();
        assert!(!cache.advance_layout(&document, &change.change_set));
        assert!(cache.config.is_empty());
        let work = cache.refresh(&document, &registry, Some(&change.change_set));
        assert_eq!((work.config_computed, work.config_hits), (1, 0));
        assert_equivalent(&cache, &document, &registry);
    }
}
