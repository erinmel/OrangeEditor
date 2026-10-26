//! Type-only projection of graph-local function interfaces at `core/call` sites.
//!
//! This projection is for authoring inference. It deliberately carries no execution kind,
//! suspension, or effect facts and is not an accepted node contract or signature snapshot.

use std::collections::BTreeSet;

use serde_json::Value;
use thiserror::Error;

use crate::document::{
    BodyRef, FunctionDecl, GraphDocument, NodeInstance, PortDescriptor, PortDirection, PortKind,
    TypeParam,
};
use crate::edit::DeclId;
use crate::id::FunctionId;
use crate::type_term::TypeTerm;

use super::language::IMPLICIT_PORTS;
use super::{ReferenceError, ReferenceKind, RegistrySnapshot};

/// The graph-local type interface visible at one call site.
///
/// This is only a type-inference projection. In particular, it is not a [`super::NodeContract`]
/// and does not establish that the target body conforms or that the call can execute.
#[derive(Debug, Clone, PartialEq)]
pub struct CallInterface {
    /// The exact graph-local target selected by the call's typed config reference.
    pub function: FunctionId,
    /// The target's universal type parameters, instantiated independently by each call node.
    pub type_params: Vec<TypeParam>,
    /// Declared data inputs followed by declared data outputs, with provenance removed.
    pub ports: Vec<PortDescriptor>,
}

/// Why a call's type-only interface cannot be projected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CallInterfaceError {
    /// The node is not the exact supported host-owned `core/call` identity/configuration.
    #[error("call node identity or configuration is unsupported")]
    Identity,
    /// The call has no well-formed canonical function target reference.
    #[error("call target reference is missing or malformed")]
    TargetMissing,
    /// The referenced function is not declared in this graph.
    #[error("call target is not declared in this graph")]
    TargetUnknown,
    /// The caller or target body declares a non-control execution domain.
    #[error("call typing requires the supported control domain")]
    DomainUnsupported,
    /// The declared function interface is not a valid type-only data interface.
    #[error("function interface is malformed for call typing")]
    InvalidInterface,
}

/// Project the declared data interface for an exact graph-local `core/call`.
///
/// The function kind and body are intentionally not inspected. Those facts are derived only
/// after body and call-summary validation. No document state is changed.
/// `node` must be the matching node resident in `document`, because its containing body's
/// supported domain is part of the typing precondition. Callers constructing a proposed edit
/// should apply it to a scratch document first; the projector does not infer a body from a
/// detached node value.
pub fn call_interface(
    document: &GraphDocument,
    node: &NodeInstance,
    registry: &RegistrySnapshot,
) -> Result<Option<CallInterface>, CallInterfaceError> {
    if node.definition.id.as_str() != "core/call" {
        return Ok(None);
    }
    if node.definition.version != 1
        || node.config_version != 1
        || node.provider.is_some()
        || document.node(node.id) != Some(node)
    {
        return Err(CallInterfaceError::Identity);
    }

    let caller = document
        .locate_node(node.id)
        .ok_or(CallInterfaceError::Identity)?;
    if !body_domain_supported(document, caller) {
        return Err(CallInterfaceError::DomainUnsupported);
    }

    let target = match registry.reference(node, ReferenceKind::Function) {
        Ok(DeclId::Function(id)) => id,
        Ok(_) | Err(ReferenceError::Missing | ReferenceError::Malformed) => {
            return Err(CallInterfaceError::TargetMissing);
        }
    };
    let function = document
        .functions()
        .get(&target)
        .ok_or(CallInterfaceError::TargetUnknown)?;
    if !domain_value_supported(function.extensions.get("domain")) {
        return Err(CallInterfaceError::DomainUnsupported);
    }

    let ports = project_interface(function)?;
    Ok(Some(CallInterface {
        function: target,
        type_params: function.type_params.clone(),
        ports,
    }))
}

fn body_domain_supported(document: &GraphDocument, body: BodyRef) -> bool {
    match body {
        BodyRef::Root => domain_value_supported(document.extensions().get("domain")),
        BodyRef::Function(id) => document
            .functions()
            .get(&id)
            .is_some_and(|function| domain_value_supported(function.extensions.get("domain"))),
    }
}

fn domain_value_supported(domain: Option<&Value>) -> bool {
    domain.is_none_or(|value| value.as_str() == Some("control"))
}

pub(crate) fn project_interface(
    function: &FunctionDecl,
) -> Result<Vec<PortDescriptor>, CallInterfaceError> {
    let mut params = BTreeSet::<&str>::new();
    for param in &function.type_params {
        if !params.insert(param.name.as_str()) {
            return Err(CallInterfaceError::InvalidInterface);
        }
    }

    let mut ids = BTreeSet::new();
    let mut ports = Vec::with_capacity(function.inputs.len() + function.outputs.len());
    for (expected_direction, declared) in [
        (PortDirection::Input, function.inputs.as_slice()),
        (PortDirection::Output, function.outputs.as_slice()),
    ] {
        for port in declared {
            if port.direction != expected_direction
                || port.kind != PortKind::Data
                || port.ty.is_none()
                || IMPLICIT_PORTS.contains(&port.port_id.as_str())
                || !ids.insert(port.port_id.clone())
            {
                return Err(CallInterfaceError::InvalidInterface);
            }
            if expected_direction == PortDirection::Output
                && (port.default.is_some()
                    || port.optional.unwrap_or(false)
                    || port.lazy.unwrap_or(false))
            {
                return Err(CallInterfaceError::InvalidInterface);
            }
            if expected_direction == PortDirection::Input
                && (port.source.is_some() || port.available_on.is_some())
            {
                return Err(CallInterfaceError::InvalidInterface);
            }
            let term = TypeTerm::parse(
                port.ty
                    .as_ref()
                    .ok_or(CallInterfaceError::InvalidInterface)?,
            )
            .map_err(|_| CallInterfaceError::InvalidInterface)?;
            if !params_in_term_are_declared(&term, &params) {
                return Err(CallInterfaceError::InvalidInterface);
            }

            let mut projected = port.clone();
            projected.source = None;
            projected.available_on = None;
            if projected.optional == Some(false) {
                projected.optional = None;
            }
            if projected.lazy == Some(false) {
                projected.lazy = None;
            }
            ports.push(projected);
        }
    }
    Ok(ports)
}

fn params_in_term_are_declared(term: &TypeTerm, declared: &BTreeSet<&str>) -> bool {
    match term {
        TypeTerm::Param(name) => declared.contains(name.as_str()),
        TypeTerm::Named(named) => named
            .args
            .iter()
            .all(|arg| params_in_term_are_declared(arg, declared)),
    }
}

/// Whether a definition ID names a function-structure marker (`core/function-entry`,
/// `core/function-outputs` or `core/return`).
pub(crate) fn is_function_marker(id: &str) -> bool {
    matches!(
        id,
        "core/function-entry" | "core/function-outputs" | "core/return"
    )
}

/// The function type parameters that are rigid at `node`: its owning function's declared
/// parameters, or none for a root-body node. Inside a body they are opaque constants, equal
/// only to themselves; each call site of the function instantiates them afresh (11 section 14).
pub(crate) fn rigid_params(document: &GraphDocument, node: crate::id::NodeId) -> &[TypeParam] {
    body_rigid_params(document, document.locate_node(node))
}

/// [`rigid_params`] for every node of `body`; a missing body has none.
pub(crate) fn body_rigid_params(document: &GraphDocument, body: Option<BodyRef>) -> &[TypeParam] {
    match body {
        Some(BodyRef::Function(id)) => document
            .functions()
            .get(&id)
            .map_or(&[], |function| function.type_params.as_slice()),
        _ => &[],
    }
}

/// Ports of a function-structure marker, derived from the owning function's interface.
///
/// The entry exposes the declared inputs as data outputs; return and function-outputs accept
/// the declared outputs as required data inputs (`return` also has the exec `in`). The
/// declaration is the only type authority, so a marker is never independently generic: the
/// function's own type parameters stay in the port types as rigid `$T` constants shared by
/// every marker, and only a call site instantiates them.
pub(crate) fn marker_contract(
    document: &GraphDocument,
    node: &crate::document::NodeInstance,
) -> Result<super::NodeContract, CallInterfaceError> {
    if !is_function_marker(node.definition.id.as_str())
        || node.definition.version != 1
        || node.config_version != 1
        || node.provider.is_some()
    {
        return Err(CallInterfaceError::Identity);
    }
    let Some(BodyRef::Function(function)) = document.locate_node(node.id) else {
        return Err(CallInterfaceError::Identity);
    };
    let declaration = document
        .functions()
        .get(&function)
        .ok_or(CallInterfaceError::Identity)?;
    let interface =
        project_interface(declaration).map_err(|_| CallInterfaceError::InvalidInterface)?;
    let entry = node.definition.id.as_str() == "core/function-entry";
    let mut ports = Vec::new();
    for port in interface {
        let ty = port.ty.expect("projected interface ports are typed");
        match (entry, port.direction) {
            (true, crate::document::PortDirection::Input) => {
                ports.push(PortDescriptor::data_out(
                    port.port_id.as_str(),
                    ty,
                    crate::document::OutputSource::Computed,
                ));
            }
            (false, crate::document::PortDirection::Output) => {
                ports.push(PortDescriptor::data_in(port.port_id.as_str(), ty));
            }
            _ => {}
        }
    }
    if node.definition.id.as_str() == "core/return" {
        ports.push(PortDescriptor::exec_in("in"));
    }
    Ok(super::NodeContract {
        kind: if entry {
            crate::document::NodeKind::Entry
        } else {
            crate::document::NodeKind::Pure
        },
        suspends: None,
        type_params: Vec::new(),
        ports,
    })
}
