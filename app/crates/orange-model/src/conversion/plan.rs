//! Connection planning (11 §6, 00c.6a): a direct wire, a replaced wire, or a visible
//! `core/convert` node, planned without modifying the document.
//!
//! [`plan_connection`] resolves both endpoint types, selects a converter when they differ,
//! and builds the edit as ordinary [`DocCommand`]s. It then reuses
//! [`plan_edit`](crate::inference::plan_edit) so the inferred `type_args` and signature
//! updates travel with the edit. A wire that would make the touched types invalid is
//! refused. Identities are generated once, here, and the plan records the document content
//! state and registry snapshot it was made against.

use std::collections::BTreeMap;

use serde_json::{Map, json};
use thiserror::Error;
use uuid::Uuid;

use super::{
    ChoiceReason, ConverterCandidate, ConverterDefinition, ConverterProvider, ConverterRegistry,
    ExplicitConverterChoice, ResolvedConverter, SelectionOutcome, SelectionRejection,
};
use crate::catalog::RegistrySnapshot;
use crate::document::{
    DefinitionRef, Edge, Extensions, GraphDocument, NodeInstance, NodeKind, OutputSource,
    PackageLockEntry, PortDescriptor, PortDirection, PortKind, PortRef, ProviderBinding,
    SignatureSnapshot,
};
use crate::edit::{Committed, DocCommand};
use crate::error::{IdError, TxError};
use crate::id::{EdgeId, NodeId, PortKey, TypeExpr};
use crate::index::DocIndex;
use crate::inference::component::declared_port;
use crate::inference::{
    ComponentInference, InferenceIssue, NodeOverlays, infer_components, plan_edit,
};
use crate::signature::{SignatureSubstitutionError, semantic_key, try_substitute_params};
use crate::type_term::TypeTerm;

/// What to connect. `choice` is the candidate the user picked after a
/// [`ConnectionOutcome::ChoiceRequired`] answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRequest {
    pub from: PortRef,
    pub to: PortRef,
    pub choice: Option<ExplicitConverterChoice>,
}

/// The planner's answer for a well-formed request. Only `Ready` carries commands.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionOutcome {
    Ready(Box<ConnectionPlan>),
    ChoiceRequired {
        candidates: Vec<ConverterCandidate>,
        reason: ChoiceReason,
    },
    Rejected(SelectionRejection),
}

/// A converter node the plan inserts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertedConverter {
    pub node: NodeId,
    pub resolved: ResolvedConverter,
}

/// An applicable edit: the commands, the issues inference found, and the identities the
/// commands create. Valid only for the document state and registry snapshot
/// it records; its commands are read-only so they stay consistent with those.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionPlan {
    commands: Vec<DocCommand>,
    /// Outcomes after the edit: unresolved parameters, and conflicts only when the
    /// endpoints' components already had them (new conflicts are refused).
    pub issues: Vec<InferenceIssue>,
    /// Existing edges into the target port that the plan removes (11 §4.3).
    pub replaced: Vec<EdgeId>,
    /// One edge for a direct wire; `from → converter` and `converter → to` otherwise.
    pub edges: Vec<EdgeId>,
    pub converter: Option<InsertedConverter>,
    preconditions: Preconditions,
}

impl ConnectionPlan {
    /// Wire replacement, converter and edges, then the inferred updates (one transaction).
    pub fn commands(&self) -> &[DocCommand] {
        &self.commands
    }
}

/// What a plan was made against. Revision alone is insufficient across clones and reloads,
/// so the document side is its content-state identity.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Preconditions {
    document_state: u64,
    registry_generation: u64,
}

/// A request that cannot be planned at all, as opposed to one that needs a choice.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConnectionError {
    #[error("node `{0}` does not exist")]
    NodeNotFound(NodeId),
    #[error("node `{node}` declares no typed data port `{port}`")]
    NotADataPort { node: NodeId, port: PortKey },
    #[error("port `{port}` of node `{node}` is not an {expected:?} port")]
    WrongDirection {
        node: NodeId,
        port: PortKey,
        expected: PortDirection,
    },
    #[error("nodes `{from}` and `{to}` are in different bodies")]
    CrossBody { from: NodeId, to: NodeId },
    #[error("edge `{0}` already connects these ports")]
    AlreadyConnected(EdgeId),
    /// The wire would make the touched generic components invalid (`E-TYPE-MISMATCH`,
    /// `E-CONSTRAINT-UNSATISFIED`, `E-NO-SPECIALIZATION` or `E-TYPE-COMPLEXITY`).
    #[error("the connection would make the connected types invalid")]
    TypeConflict(Vec<InferenceIssue>),
    #[error(transparent)]
    Type(#[from] SignatureSubstitutionError),
    #[error(transparent)]
    Id(#[from] IdError),
    #[error(transparent)]
    Edit(#[from] TxError),
}

/// Plans connecting output `request.from` to input `request.to`.
///
/// Generic endpoint types are inferred from current evidence after removing replaced edges,
/// excluding the target literal that the new wire will shadow; cached `type_args` are not
/// evidence. Pins and remaining edges/literals still constrain selection. If
/// either still mentions an unresolved parameter the wire is planned directly and inference
/// decides; otherwise equal types connect directly and differing types go through
/// [`ConverterRegistry::select_for_connection`] (or the explicit choice). An existing edge
/// into `to` is replaced. Literals on the target port are untouched and stay shadowed.
///
/// A wire whose inference reports a conflict is refused with
/// [`ConnectionError::TypeConflict`], unless the endpoints' components already reported
/// one before the edit: a broken document (for example an imported one) stays editable.
///
/// `new_id` must produce v4 or v7 UUIDs and is called once per created node or edge. The
/// document is never modified, and `ChoiceRequired`/`Rejected` carry no commands.
pub fn plan_connection(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    overlays: &NodeOverlays,
    request: ConnectionRequest,
    mut new_id: impl FnMut() -> Uuid,
) -> Result<ConnectionOutcome, ConnectionError> {
    let ConnectionRequest { from, to, choice } = request;
    let converters = registry.converters();
    let body = document
        .locate_node(from.node)
        .ok_or(ConnectionError::NodeNotFound(from.node))?;
    if document
        .locate_node(to.node)
        .ok_or(ConnectionError::NodeNotFound(to.node))?
        != body
    {
        return Err(ConnectionError::CrossBody {
            from: from.node,
            to: to.node,
        });
    }
    // Validate the request before analyzing or allocating any inserted identities.
    endpoint_type(
        document,
        registry,
        overlays,
        &from,
        PortDirection::Output,
        &[],
    )?;
    endpoint_type(document, registry, overlays, &to, PortDirection::Input, &[])?;

    let index = DocIndex::build(document);
    let replaced: Vec<EdgeId> = index.incoming(&to).collect();
    for &id in &replaced {
        if document.edge(id).is_some_and(|edge| edge.from == from) {
            return Err(ConnectionError::AlreadyConnected(id));
        }
    }

    let mut edit: Vec<DocCommand> = replaced
        .iter()
        .map(|&edge| DocCommand::Disconnect { edge })
        .collect();
    // Analyze selection against the evidence that survives the replacement. The literal
    // is removed only on this scratch copy: the accepted edit preserves it, shadowed.
    let (from_type, to_type) = {
        let mut selection_document = document.clone();
        let _ = selection_document.apply(edit.iter().cloned().chain([DocCommand::SetLiteral {
            port: to.clone(),
            literal: None,
        }]))?;
        let components = infer_components(
            &selection_document,
            &DocIndex::build(&selection_document),
            [from.node, to.node],
            registry,
            overlays,
        );
        (
            endpoint_type(
                &selection_document,
                registry,
                overlays,
                &from,
                PortDirection::Output,
                &components,
            )?,
            endpoint_type(
                &selection_document,
                registry,
                overlays,
                &to,
                PortDirection::Input,
                &components,
            )?,
        )
    };
    let wire = match select(converters, &from_type, &to_type, choice) {
        Ok(wire) => wire,
        Err(outcome) => return Ok(outcome),
    };
    let mut edges = Vec::new();
    let converter = match wire {
        Wire::Direct => {
            edges.push(fresh_edge(&mut new_id, &from, &to, &mut edit)?);
            None
        }
        Wire::Convert(resolved) => {
            let definition = converters
                .entry(&resolved.entry)
                .expect("a resolved converter names a registered entry");
            let node = NodeId::from_uuid(new_id())?;
            let (instance, snapshot) = convert_node(node, &resolved, definition);
            let key = semantic_key(&snapshot);
            if let ConverterProvider::Package(package) = &resolved.provider {
                edit.push(DocCommand::PutPackageLock(PackageLockEntry {
                    id: package.package.clone(),
                    version: package.version.clone(),
                    content_digest: package.content_digest.clone(),
                    extensions: Extensions::new(),
                }));
            }
            edit.push(DocCommand::PutSignature {
                key: key.clone(),
                snapshot,
            });
            edit.push(DocCommand::AddNode {
                body,
                node: NodeInstance {
                    signature: Some(key),
                    ..instance
                },
            });
            let into = PortRef {
                node,
                port: key_of("value"),
            };
            let out_of = PortRef {
                node,
                port: key_of("result"),
            };
            edges.push(fresh_edge(&mut new_id, &from, &into, &mut edit)?);
            edges.push(fresh_edge(&mut new_id, &out_of, &to, &mut edit)?);
            Some(InsertedConverter { node, resolved })
        }
    };

    let inferred = plan_edit(document, edit, registry, overlays)?;
    if inferred.issues.iter().any(InferenceIssue::is_conflict) {
        let already_broken =
            infer_components(document, &index, [from.node, to.node], registry, overlays)
                .iter()
                .any(|component| component.issues.iter().any(InferenceIssue::is_conflict));
        if !already_broken {
            let conflicts = inferred
                .issues
                .into_iter()
                .filter(InferenceIssue::is_conflict);
            return Err(ConnectionError::TypeConflict(conflicts.collect()));
        }
    }
    Ok(ConnectionOutcome::Ready(Box::new(ConnectionPlan {
        commands: inferred.commands,
        issues: inferred.issues,
        replaced,
        edges,
        converter,
        preconditions: Preconditions {
            document_state: document.state_id(),
            registry_generation: registry.generation(),
        },
    })))
}

/// How the endpoints are joined.
enum Wire {
    Direct,
    Convert(ResolvedConverter),
}

/// Decides the wire, or returns the outcome that stops planning.
fn select(
    converters: &ConverterRegistry,
    from: &TypeExpr,
    to: &TypeExpr,
    choice: Option<ExplicitConverterChoice>,
) -> Result<Wire, ConnectionOutcome> {
    let unresolved = |ty: &TypeExpr| TypeTerm::parse(ty).map_or(true, |term| term.contains_param());
    if unresolved(from) || unresolved(to) {
        // Inference decides generic endpoints; a converter choice has nothing to bind to.
        return match choice {
            None => Ok(Wire::Direct),
            Some(_) => Err(ConnectionOutcome::Rejected(SelectionRejection::StaleChoice)),
        };
    }
    if let Some(choice) = choice {
        return converters
            .resolve_explicit_choice(from, to, choice)
            .map(Wire::Convert)
            .map_err(ConnectionOutcome::Rejected);
    }
    match converters.select_for_connection(from, to, &BTreeMap::new()) {
        SelectionOutcome::Compatible => Ok(Wire::Direct),
        SelectionOutcome::Automatic(resolved) => Ok(Wire::Convert(resolved)),
        SelectionOutcome::ChoiceRequired { candidates, reason } => {
            Err(ConnectionOutcome::ChoiceRequired { candidates, reason })
        }
        SelectionOutcome::Rejected(rejection) => Err(ConnectionOutcome::Rejected(rejection)),
    }
}

/// The resolved type of a typed data port that has `direction`.
fn endpoint_type(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    overlays: &NodeOverlays,
    port: &PortRef,
    direction: PortDirection,
    components: &[ComponentInference],
) -> Result<TypeExpr, ConnectionError> {
    let node = document
        .node(port.node)
        .ok_or(ConnectionError::NodeNotFound(port.node))?;
    let not_data = || ConnectionError::NotADataPort {
        node: port.node,
        port: port.port.clone(),
    };
    let contract = overlays
        .get_for_node(registry, document, node)
        .map_err(|_| TxError::StaleNodeTemplate { node: node.id })?;
    let declared = declared_port(document, registry, overlays, port).ok_or_else(not_data)?;
    let ty = declared
        .ty
        .as_ref()
        .filter(|_| declared.kind == PortKind::Data)
        .ok_or_else(not_data)?;
    if declared.direction != direction {
        return Err(ConnectionError::WrongDirection {
            node: port.node,
            port: port.port.clone(),
            expected: direction,
        });
    }
    let empty = BTreeMap::new();
    let args = if contract.is_some_and(|template| !template.type_params.is_empty()) {
        components
            .iter()
            .find(|component| component.nodes.contains(&node.id))
            .and_then(|component| component.type_args.get(&node.id))
            .unwrap_or(&empty)
    } else {
        &node.type_args
    };
    Ok(try_substitute_params(ty, args)?)
}

fn fresh_edge(
    new_id: &mut impl FnMut() -> Uuid,
    from: &PortRef,
    to: &PortRef,
    edit: &mut Vec<DocCommand>,
) -> Result<EdgeId, ConnectionError> {
    let id = EdgeId::from_uuid(new_id())?;
    edit.push(DocCommand::Connect {
        edge: Edge {
            id,
            from: from.clone(),
            to: to.clone(),
            extensions: Extensions::new(),
        },
    });
    Ok(id)
}

fn key_of(port: &str) -> PortKey {
    PortKey::new(port.parse().expect("fixed port IDs are valid"), None)
}

/// The persisted `core/convert` instance (12 §5) and its signature snapshot. The caller
/// stores the snapshot and sets the instance's `signature` to its key.
fn convert_node(
    id: NodeId,
    resolved: &ResolvedConverter,
    definition: &ConverterDefinition,
) -> (NodeInstance, SignatureSnapshot) {
    let mut config = Map::new();
    config.insert("converter".to_owned(), json!(resolved.entry.id));
    config.insert(
        "contract_version".to_owned(),
        json!(resolved.entry.contract_version),
    );
    config.insert("options".to_owned(), json!(resolved.options));
    let provider = match &resolved.provider {
        ConverterProvider::Core => None,
        ConverterProvider::Package(package) => Some(ProviderBinding {
            package: package.package.clone(),
            version: package.version.clone(),
            id: package.id.clone(),
            extensions: Extensions::new(),
        }),
    };
    let instance = NodeInstance {
        id,
        definition: DefinitionRef {
            id: "core/convert".parse().expect("core/convert is canonical"),
            version: 1,
        },
        config_version: 1,
        config,
        type_args: BTreeMap::new(),
        type_pins: BTreeMap::new(),
        provider,
        signature: None,
        inputs: BTreeMap::new(),
        variadic: BTreeMap::new(),
        activation: None,
        extensions: Extensions::new(),
    };
    let mut ports = vec![
        data_port("value", PortDirection::Input, &definition.from),
        data_port("result", PortDirection::Output, &definition.to),
    ];
    if let Some(loss) = definition.loss_output_type() {
        ports.push(data_port("loss", PortDirection::Output, &loss));
    }
    let snapshot = SignatureSnapshot {
        kind: NodeKind::Pure,
        suspends: None,
        ports,
        extensions: Extensions::new(),
    };
    (instance, snapshot)
}

fn data_port(id: &str, direction: PortDirection, ty: &TypeExpr) -> PortDescriptor {
    match direction {
        PortDirection::Input => PortDescriptor::data_in(id, ty.clone()),
        PortDirection::Output => PortDescriptor::data_out(id, ty.clone(), OutputSource::Computed),
    }
}

/// Why a plan could not be applied.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlanApplyError {
    #[error("the document changed since the plan was made (state {expected}, now {actual})")]
    StaleDocument { expected: u64, actual: u64 },
    #[error("the registry snapshot was replaced since the plan was made")]
    StaleRegistry,
    #[error(transparent)]
    Edit(#[from] TxError),
}

/// Applies `plan` as one transaction: one revision bump and one undo step covering the
/// replaced wire, the converter, its edges and the inferred updates.
///
/// The document content state and the registry snapshot must still
/// be the ones the plan was made against; a stale plan is rejected without changing the
/// document, because its selection and inference results may no longer hold. A registry
/// snapshot is immutable, so a matching generation also means the selected provider is
/// still available. If a command fails, nothing changes, revision included.
pub fn apply_connection(
    document: &mut GraphDocument,
    registry: &RegistrySnapshot,
    plan: ConnectionPlan,
) -> Result<Committed, PlanApplyError> {
    let Preconditions {
        document_state,
        registry_generation,
    } = plan.preconditions;
    if document.state_id() != document_state {
        return Err(PlanApplyError::StaleDocument {
            expected: document_state,
            actual: document.state_id(),
        });
    }
    if registry.generation() != registry_generation {
        return Err(PlanApplyError::StaleRegistry);
    }
    Ok(document.apply(plan.commands)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::HostOrigin;

    /// Commands are read-only outside this module, so a command failure inside a fresh plan
    /// can only be injected here.
    #[test]
    fn an_injected_failure_rolls_back_every_command() {
        let registry = RegistrySnapshot::core(
            HostOrigin::new("1.0.0", format!("sha256:{}", "0".repeat(64))).unwrap(),
        );
        let mut document =
            GraphDocument::new("a0f3822f-9cf8-4ab6-a2bf-e21fd345c3c6".parse().unwrap());
        let snapshot = SignatureSnapshot {
            kind: NodeKind::Pure,
            suspends: None,
            ports: vec![],
            extensions: Extensions::new(),
        };
        let missing =
            EdgeId::from_uuid(uuid::Builder::from_random_bytes([7; 16]).into_uuid()).unwrap();
        let plan = ConnectionPlan {
            commands: vec![
                DocCommand::PutSignature {
                    key: semantic_key(&snapshot),
                    snapshot,
                },
                DocCommand::Disconnect { edge: missing },
            ],
            issues: vec![],
            replaced: vec![],
            edges: vec![],
            converter: None,
            preconditions: Preconditions {
                document_state: document.state_id(),
                registry_generation: registry.generation(),
            },
        };
        let (revision, state) = (document.revision(), document.state_id());

        let result = apply_connection(&mut document, &registry, plan);
        assert_eq!(
            result,
            Err(PlanApplyError::Edit(TxError::EdgeNotFound(missing)))
        );
        assert!(document.signatures().is_empty());
        assert_eq!(
            (document.revision(), document.state_id()),
            (revision, state)
        );
    }
}
