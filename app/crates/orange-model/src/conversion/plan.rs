//! Connection planning (11 §6, 00c.6a): a direct wire, a replaced wire, or a visible
//! `core/convert` node, planned without modifying the document.
//!
//! [`plan_connection`] resolves both endpoint types, selects a converter when they differ,
//! and builds the edit as ordinary [`DocCommand`]s. It then reuses
//! [`plan_edit`](crate::inference::plan_edit) so the inferred `type_args` and signature
//! updates travel with the edit. A wire that would make the touched types invalid is
//! refused. Identities are generated once, here, and the plan records the document content
//! state and registry snapshot it was made against.

use crate::catalog::intrinsic_id as ids;
use std::collections::BTreeMap;

use thiserror::Error;
use uuid::Uuid;

use super::{
    ChoiceReason, ConverterCandidate, ConverterConfig, ConverterDefinition, ConverterProvider,
    ConverterRegistry, ExplicitConverterChoice, ResolvedConverter, SelectionOutcome,
    SelectionRejection,
};
use crate::catalog::RegistrySnapshot;
use crate::document::{
    DefinitionRef, Edge, Extensions, GraphDocument, NodeInstance, NodeKind, OutputSource,
    PackageLockEntry, PortDescriptor, PortDirection, PortKind, PortRef, ProviderBinding,
    SignatureSnapshot,
};
pub use crate::edit::PlanApplyError;
use crate::edit::{Committed, DocCommand, Preconditions};
use crate::error::{IdError, TxError};
use crate::id::{EdgeId, NodeId, PortKey, TypeExpr};
use crate::index::DocIndex;
use crate::inference::component::declared_port;
use crate::inference::{ComponentInference, InferenceIssue, infer_components, plan_edit_indexed};
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
    request: ConnectionRequest,
    new_id: impl FnMut() -> Uuid,
) -> Result<ConnectionOutcome, ConnectionError> {
    let mut scratch = document.clone();
    let mut index = DocIndex::build(&scratch);
    plan_connection_indexed(&mut scratch, &mut index, registry, request, new_id)
}

/// Plans using a retained index and journaled trial edits instead of document copies.
/// Public read-only planning remains available through plan_connection. This method
/// restores document content/revision/identity and the index before returning.
/// Stale indexes rebuild on entry; an unwind restores the document and invalidates
/// the index so its next indexed operation rebuilds it.
pub fn plan_connection_indexed(
    document: &mut GraphDocument,
    index: &mut DocIndex,
    registry: &RegistrySnapshot,
    request: ConnectionRequest,
    mut new_id: impl FnMut() -> Uuid,
) -> Result<ConnectionOutcome, ConnectionError> {
    if !index.is_current(document) {
        *index = DocIndex::build(document);
    }
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
    endpoint_type(document, registry, &from, PortDirection::Output, &[])?;
    endpoint_type(document, registry, &to, PortDirection::Input, &[])?;

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
    // is removed only inside the journaled trial: the accepted edit preserves it, shadowed.
    let (from_type, to_type) = crate::inference::persist::scratch_indexed(
        document,
        index,
        edit.iter().cloned().chain([DocCommand::SetLiteral {
            port: to.clone(),
            literal: None,
        }]),
        |selection, index, _| {
            let components = infer_components(selection, index, [from.node, to.node], registry);
            Ok::<_, ConnectionError>((
                endpoint_type(
                    selection,
                    registry,
                    &from,
                    PortDirection::Output,
                    &components,
                )?,
                endpoint_type(selection, registry, &to, PortDirection::Input, &components)?,
            ))
        },
    )??;
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

    let inferred = plan_edit_indexed(document, index, edit, registry)?;
    if inferred.issues.iter().any(InferenceIssue::is_conflict) {
        let already_broken = infer_components(document, index, [from.node, to.node], registry)
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
        preconditions: inferred.preconditions,
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
    let contract = registry.contract(document, node).ok().flatten();
    let declared = declared_port(document, registry, port).ok_or_else(not_data)?;
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
    let config = ConverterConfig {
        entry: resolved.entry.clone(),
        options: resolved.options.clone(),
    }
    .to_config();
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
            id: ids::CONVERT.parse().expect("core/convert is canonical"),
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

/// Applies `plan` as one transaction: one revision bump and one undo step covering the
/// replaced wire, the converter, its edges and the inferred updates.
///
/// The document content state and the registry snapshot must still be
/// the ones the plan was made against; a stale plan is rejected without changing the
/// document, because its selection and inference results may no longer hold. A registry
/// snapshot is immutable, so a matching generation also means the selected provider is
/// still available. If a command fails, nothing changes, revision included.
pub fn apply_connection(
    document: &mut GraphDocument,
    registry: &RegistrySnapshot,
    plan: ConnectionPlan,
) -> Result<Committed, PlanApplyError> {
    plan.preconditions.check(document, registry)?;
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
            preconditions: Preconditions::capture(&document, &registry),
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
