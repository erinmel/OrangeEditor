//! Read-only core constraint proofs and declared package structural derives.

use std::collections::HashSet;

use thiserror::Error;

use crate::id::TypeExpr;
use crate::type_term::{MAX_NESTING, NamedTerm, TypeTerm};

use super::errors::{TypeRegistrationError, TypeResolveError};
use super::identity::NamedTypeId;
use super::implementations::{ConstraintId, ConstraintImplementationRef, PackageConstraintId};
use super::registry::{RegisteredType, TypeData, TypeRegistry, TypeRegistryBuilder};

#[cfg(test)]
#[path = "constraints_tests.rs"]
mod tests;

/// A v1 language constraint from 11 §5.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Constraint {
    Eq,
    Ord,
    Hash,
    Num,
    Int,
    Float,
    Display,
    Serialize,
    Default,
}

impl Constraint {
    const ALL: [Self; 9] = [
        Self::Eq,
        Self::Ord,
        Self::Hash,
        Self::Num,
        Self::Int,
        Self::Float,
        Self::Display,
        Self::Serialize,
        Self::Default,
    ];

    /// Parses a constraint name as written in a definition type parameter (11 §5.4).
    pub fn from_id_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|constraint| constraint.id_name() == name)
    }

    pub(crate) fn id_name(self) -> &'static str {
        match self {
            Self::Eq => "eq",
            Self::Ord => "ord",
            Self::Hash => "hash",
            Self::Num => "num",
            Self::Int => "int",
            Self::Float => "float",
            Self::Display => "display",
            Self::Serialize => "serialize",
            Self::Default => "default",
        }
    }
}

/// A bounded set of arithmetic operations used for operation-specific type proofs.
///
/// This describes support only; it does not select or execute an arithmetic kernel.
/// The current proof surface covers basic add/subtract/multiply/divide. It does not define
/// whether other math operations (such as remainder, power, or transcendental functions)
/// are part of the language; future nodes must request their exact capability explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumericOperation {
    Add,
    Subtract,
    Multiply,
    Divide,
}

/// A capability requested for a concrete type term.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CapabilityRequest {
    Constraint(Constraint),
    /// A package-owned constraint, requested by its stable versioned identity.
    PackageConstraint(PackageConstraintId),
    NumericOperation(NumericOperation),
}

/// Evidence kind for a capability proof.
/// A `serialize` witness describes declared type capability only: it does not serialize a
/// value or prove that value-sensitive `core/error` details have been redacted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityEvidence {
    CoreRule,
    StructuralDerive,
    /// A stable package binding and implementation reference; no package code was called.
    PackageImplementation(ConstraintImplementationRef),
}

/// Read-only evidence that a term satisfies a requested constraint or operation.
///
/// A `Default` witness for `list`, `map`, or `option` concerns their empty/none value and
/// does not require the contained type to be defaultable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityWitness {
    pub term: TypeTerm,
    pub request: CapabilityRequest,
    pub evidence: CapabilityEvidence,
}

/// A failure to evaluate a type capability.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CapabilityError {
    #[error(transparent)]
    Type(#[from] TypeResolveError),
    #[error("deriving `{constraint:?}` for recursive type `{ty}` has no bounded proof")]
    RecursiveDerivation { ty: String, constraint: Constraint },
    #[error("structural derivation exceeds the nominal-schema depth limit of {max}")]
    DerivationTooDeep { max: usize },
}

impl TypeRegistryBuilder {
    /// Declares a package-owned structural derive for a record or enum.
    ///
    /// This host implementation supports only `eq`, `hash`, `display`, and `serialize`.
    /// Other constraint forms require an explicit package implementation in the later
    /// coherence layer. Derives are checked against every field or payload at query time.
    pub fn derive_constraint(
        &mut self,
        package_namespace: &str,
        ty: &TypeExpr,
        constraint: Constraint,
    ) -> Result<(), TypeRegistrationError> {
        if !matches!(
            constraint,
            Constraint::Eq | Constraint::Hash | Constraint::Display | Constraint::Serialize
        ) {
            return Err(TypeRegistrationError::UnsupportedDerivation {
                ty: ty.to_string(),
                constraint: constraint.id_name().to_owned(),
            });
        }
        let term = TypeTerm::parse(ty).map_err(TypeRegistrationError::InvalidTypeExpr)?;
        let TypeTerm::Named(named) = term else {
            return Err(TypeRegistrationError::ExpectedNamedType { ty: ty.to_string() });
        };
        if !named.args.is_empty() {
            return Err(TypeRegistrationError::GenericNominalType { ty: ty.to_string() });
        }
        let id = NamedTypeId::from_term(&named);
        if id.namespace == "core" || package_namespace == "core" {
            return Err(TypeRegistrationError::ReservedCoreNamespace { ty: id.to_string() });
        }
        if package_namespace != id.namespace {
            return Err(TypeRegistrationError::OwnerNamespaceMismatch {
                ty: id.to_string(),
                package_namespace: package_namespace.to_owned(),
            });
        }
        let Some(definition) = self.definitions.get(&id) else {
            return Err(TypeRegistrationError::UnknownType { ty: id.to_string() });
        };
        if !matches!(definition.data, TypeData::Record(_) | TypeData::Enum(_)) {
            return Err(TypeRegistrationError::NotDerivableType { ty: id.to_string() });
        }
        let target = TypeTerm::Named(named.clone());
        if self
            .implementations
            .contains_key(&(target, ConstraintId::Core(constraint)))
        {
            return Err(TypeRegistrationError::DuplicateConstraintImplementation {
                ty: id.to_string(),
                constraint: constraint.id_name().to_owned(),
            });
        }
        let Some(definition) = self.definitions.get_mut(&id) else {
            unreachable!("definition was checked above")
        };
        definition.derives.insert(constraint);
        Ok(())
    }
}

impl TypeRegistry {
    /// Proves a capability for a validated concrete term without interning it or executing
    /// any package code. `None` means the type does not satisfy the request.
    pub fn supports_term(
        &self,
        term: &TypeTerm,
        request: CapabilityRequest,
    ) -> Result<Option<CapabilityWitness>, CapabilityError> {
        self.validate(term)?;
        let mut visiting = HashSet::new();
        let evidence = match &request {
            CapabilityRequest::Constraint(constraint) => {
                constraint_evidence(self, term, *constraint, &mut visiting, 0)?
            }
            CapabilityRequest::PackageConstraint(constraint) => self
                .explicit_constraint_implementation(
                    term,
                    &ConstraintId::Package(constraint.clone()),
                )
                .cloned()
                .map(CapabilityEvidence::PackageImplementation),
            CapabilityRequest::NumericOperation(operation) => {
                numeric_operation_evidence(self, term, *operation)
            }
        };
        Ok(evidence.map(|evidence| CapabilityWitness {
            term: term.clone(),
            request,
            evidence,
        }))
    }

    pub(super) fn validate_map_constraints(&self, term: &TypeTerm) -> Result<(), TypeResolveError> {
        validate_map_constraints(self, term, 0)
    }

    pub(super) fn validate_registered_derives(&self) -> Result<(), TypeRegistrationError> {
        for (id, definition) in &self.definitions {
            for constraint in &definition.derives {
                let term = TypeTerm::Named(NamedTerm {
                    namespace: id.namespace.clone(),
                    name: id.name.clone(),
                    major: id.major,
                    args: Vec::new(),
                });
                let mut visiting = HashSet::new();
                match constraint_evidence(self, &term, *constraint, &mut visiting, 0) {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        return Err(TypeRegistrationError::InvalidDerivedConstraint {
                            ty: id.to_string(),
                            constraint: constraint.id_name().to_owned(),
                        });
                    }
                    Err(CapabilityError::RecursiveDerivation { ty, constraint }) => {
                        return Err(TypeRegistrationError::RecursiveDerivation {
                            ty,
                            constraint: constraint.id_name().to_owned(),
                        });
                    }
                    Err(CapabilityError::DerivationTooDeep { max }) => {
                        return Err(TypeRegistrationError::DerivationTooDeep { max });
                    }
                    Err(CapabilityError::Type(_)) => {
                        return Err(TypeRegistrationError::InvalidDerivedConstraint {
                            ty: id.to_string(),
                            constraint: constraint.id_name().to_owned(),
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

fn validate_map_constraints(
    registry: &TypeRegistry,
    term: &TypeTerm,
    depth: usize,
) -> Result<(), TypeResolveError> {
    if depth > MAX_NESTING {
        return Err(TypeResolveError::TooDeep { max: MAX_NESTING });
    }
    let TypeTerm::Named(named) = term else {
        return Ok(());
    };
    if named.namespace == "core" && named.name == "map" {
        let [key, _] = named.args.as_slice() else {
            unreachable!("the map arity was structurally validated")
        };
        if key.core("f32").is_some() || key.core("f64").is_some() {
            return Err(TypeResolveError::InvalidMapKey {
                ty: key.to_string(),
            });
        }
        for constraint in [Constraint::Hash, Constraint::Eq] {
            let mut visiting = HashSet::new();
            match constraint_evidence(registry, key, constraint, &mut visiting, 0) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    return Err(TypeResolveError::InvalidMapKey {
                        ty: key.to_string(),
                    });
                }
                Err(CapabilityError::RecursiveDerivation { ty, constraint }) => {
                    return Err(TypeResolveError::RecursiveDerivation {
                        ty,
                        constraint: constraint.id_name().to_owned(),
                    });
                }
                Err(CapabilityError::DerivationTooDeep { max }) => {
                    return Err(TypeResolveError::DerivationTooDeep { max });
                }
                Err(CapabilityError::Type(error)) => return Err(error),
            }
        }
    }
    for arg in &named.args {
        validate_map_constraints(registry, arg, depth + 1)?;
    }
    Ok(())
}

fn constraint_evidence(
    registry: &TypeRegistry,
    term: &TypeTerm,
    constraint: Constraint,
    visiting: &mut HashSet<(NamedTypeId, Constraint)>,
    depth: usize,
) -> Result<Option<CapabilityEvidence>, CapabilityError> {
    if depth > MAX_NESTING {
        return Err(CapabilityError::DerivationTooDeep { max: MAX_NESTING });
    }
    let TypeTerm::Named(named) = term else {
        return Ok(None);
    };
    let id = NamedTypeId::from_term(named);
    let definition = registry.definitions.get(&id).expect("term was validated");
    if let Some(implementation) =
        registry.explicit_constraint_implementation(term, &ConstraintId::Core(constraint))
    {
        return Ok(Some(CapabilityEvidence::PackageImplementation(
            implementation.clone(),
        )));
    }
    match &definition.data {
        TypeData::Core => core_constraint(registry, named, constraint, visiting, depth),
        TypeData::Record(schema) => derive_members(
            registry,
            named,
            definition,
            constraint,
            visiting,
            schema.fields.iter().map(|field| &field.ty),
            depth,
        ),
        TypeData::Enum(schema) => derive_members(
            registry,
            named,
            definition,
            constraint,
            visiting,
            schema
                .variants
                .iter()
                .filter_map(|variant| variant.payload.as_ref()),
            depth,
        ),
        TypeData::HandleKind => Ok(None),
    }
}

fn derive_members<'a>(
    registry: &TypeRegistry,
    named: &NamedTerm,
    definition: &RegisteredType,
    constraint: Constraint,
    visiting: &mut HashSet<(NamedTypeId, Constraint)>,
    members: impl Iterator<Item = &'a TypeTerm>,
    depth: usize,
) -> Result<Option<CapabilityEvidence>, CapabilityError> {
    if !definition.derives.contains(&constraint) {
        return Ok(None);
    }
    let id = NamedTypeId::from_term(named);
    if !visiting.insert((id.clone(), constraint)) {
        return Err(CapabilityError::RecursiveDerivation {
            ty: id.to_string(),
            constraint,
        });
    }
    let result = members
        .map(|member| constraint_evidence(registry, member, constraint, visiting, depth + 1))
        .try_fold(true, |all, member| {
            member.map(|member| all && member.is_some())
        });
    visiting.remove(&(id, constraint));
    match result? {
        true => Ok(Some(CapabilityEvidence::StructuralDerive)),
        false => Ok(None),
    }
}

fn core_constraint(
    registry: &TypeRegistry,
    named: &NamedTerm,
    constraint: Constraint,
    visiting: &mut HashSet<(NamedTypeId, Constraint)>,
    depth: usize,
) -> Result<Option<CapabilityEvidence>, CapabilityError> {
    let name = named.name.as_str();
    let supports = match constraint {
        Constraint::Eq => match name {
            "list" | "option" | "result" => all_args(registry, named, constraint, visiting, depth)?,
            "map" => all_args(registry, named, constraint, visiting, depth)?,
            "f32" | "f64" | "stream" | "handle" => false,
            _ => true,
        },
        Constraint::Ord => matches!(
            name,
            "i32" | "i64" | "u32" | "u64" | "bool" | "string" | "duration" | "timestamp"
        ),
        Constraint::Hash => matches!(
            name,
            "bool" | "i32" | "i64" | "u32" | "u64" | "string" | "bytes"
        ),
        Constraint::Num => is_numeric(name),
        Constraint::Int => matches!(name, "i32" | "i64" | "u32" | "u64"),
        Constraint::Float => matches!(name, "f32" | "f64"),
        Constraint::Display => core_display(registry, named, visiting, depth)?,
        Constraint::Serialize => core_serialize(registry, named, visiting, depth)?,
        Constraint::Default => matches!(
            name,
            "bool"
                | "i32"
                | "i64"
                | "u32"
                | "u64"
                | "f32"
                | "f64"
                | "duration"
                | "timestamp"
                | "string"
                | "bytes"
                | "list"
                | "map"
                | "option"
        ),
    };
    Ok(supports.then_some(CapabilityEvidence::CoreRule))
}

fn all_args(
    registry: &TypeRegistry,
    named: &NamedTerm,
    constraint: Constraint,
    visiting: &mut HashSet<(NamedTypeId, Constraint)>,
    depth: usize,
) -> Result<bool, CapabilityError> {
    for arg in &named.args {
        if constraint_evidence(registry, arg, constraint, visiting, depth + 1)?.is_none() {
            return Ok(false);
        }
    }
    Ok(true)
}

fn core_display(
    registry: &TypeRegistry,
    named: &NamedTerm,
    visiting: &mut HashSet<(NamedTypeId, Constraint)>,
    depth: usize,
) -> Result<bool, CapabilityError> {
    match named.name.as_str() {
        "list" | "option" | "map" => {
            all_args(registry, named, Constraint::Display, visiting, depth)
        }
        "bool" | "i32" | "i64" | "u32" | "u64" | "f32" | "f64" | "string" | "bytes"
        | "duration" | "timestamp" | "json" | "secret-ref" => Ok(true),
        // Result, error, stream and handle have no literal codec form.
        _ => Ok(false),
    }
}

fn core_serialize(
    registry: &TypeRegistry,
    named: &NamedTerm,
    visiting: &mut HashSet<(NamedTypeId, Constraint)>,
    depth: usize,
) -> Result<bool, CapabilityError> {
    match named.name.as_str() {
        "list" | "option" | "map" => {
            all_args(registry, named, Constraint::Serialize, visiting, depth)
        }
        "result" => all_args(registry, named, Constraint::Serialize, visiting, depth),
        // The type-level table includes error; the eventual value serializer still must
        // redact/validate sensitive details before emitting a value. This query cannot prove
        // that value-level condition.
        "error" => Ok(true),
        "stream" | "handle" => Ok(false),
        _ => Ok(true),
    }
}

fn numeric_operation_evidence(
    registry: &TypeRegistry,
    term: &TypeTerm,
    operation: NumericOperation,
) -> Option<CapabilityEvidence> {
    if let Some(implementation) = registry.explicit_numeric_implementation(term, operation) {
        return Some(CapabilityEvidence::PackageImplementation(
            implementation.clone(),
        ));
    }
    let TypeTerm::Named(named) = term else {
        return None;
    };
    if named.namespace != "core" {
        return None;
    }
    let supported = match named.name.as_str() {
        "i32" | "i64" | "u32" | "u64" | "f32" | "f64" => true,
        "duration" => matches!(
            operation,
            NumericOperation::Add | NumericOperation::Subtract
        ),
        _ => false,
    };
    supported.then_some(CapabilityEvidence::CoreRule)
}

fn is_numeric(name: &str) -> bool {
    matches!(
        name,
        "i32" | "i64" | "u32" | "u64" | "f32" | "f64" | "duration"
    )
}
