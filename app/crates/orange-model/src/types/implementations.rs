//! Coherence-checked package constraint and operation implementation metadata.

use std::collections::HashMap;

use crate::document::ProviderBinding;
use crate::id::TypeExpr;
use crate::type_term::TypeTerm;

use super::TypeOwner;
use super::constraints::{Constraint, NumericOperation};
use super::errors::TypeRegistrationError;
use super::registry::{TypeRegistry, TypeRegistryBuilder};

/// Stable identity for a language or package-owned constraint.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConstraintId {
    /// A constraint defined by the language contract.
    Core(Constraint),
    /// A package-owned constraint with its own major contract version.
    Package(PackageConstraintId),
}

/// Versioned identity for a package-defined constraint.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageConstraintId {
    namespace: String,
    name: String,
    major: u32,
}

impl PackageConstraintId {
    /// Creates a validated package constraint identity.
    pub fn new(
        namespace: impl Into<String>,
        name: impl Into<String>,
        major: u32,
    ) -> Result<Self, TypeRegistrationError> {
        let namespace = namespace.into();
        let name = name.into();
        if major == 0 || namespace == "core" {
            return Err(TypeRegistrationError::InvalidConstraintId {
                namespace,
                name,
                major,
            });
        }
        let expression = format!("{namespace}/{name}@{major}");
        let parsed = expression
            .parse::<TypeExpr>()
            .ok()
            .and_then(|expression| TypeTerm::parse(&expression).ok());
        let Some(TypeTerm::Named(named)) = parsed else {
            return Err(TypeRegistrationError::InvalidConstraintId {
                namespace,
                name,
                major,
            });
        };
        if named.namespace != namespace
            || named.name != name
            || named.major != major
            || !named.args.is_empty()
        {
            return Err(TypeRegistrationError::InvalidConstraintId {
                namespace,
                name,
                major,
            });
        }
        Ok(Self {
            namespace,
            name,
            major,
        })
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn major(&self) -> u32 {
        self.major
    }
}

impl std::fmt::Display for PackageConstraintId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}@{}", self.namespace, self.name, self.major)
    }
}

/// Exact package implementation identity recorded as proof metadata.
///
/// This names a declared implementation; it does not load or call package code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintImplementationRef {
    pub provider: ProviderBinding,
    pub content_digest: String,
    pub implementation_id: String,
}

impl ConstraintImplementationRef {
    fn validate(&self, authority: &str) -> Result<(), TypeRegistrationError> {
        if self.provider.package != authority {
            return Err(TypeRegistrationError::ImplementationAuthorityMismatch {
                authority: authority.to_owned(),
                provider: self.provider.package.clone(),
            });
        }
        if self.provider.package.trim().is_empty()
            || self.provider.package.trim() != self.provider.package
            || self.provider.version.trim().is_empty()
            || self.provider.version.trim() != self.provider.version
            || self.provider.id.trim().is_empty()
            || self.provider.id.trim() != self.provider.id
            || self.implementation_id.trim().is_empty()
            || self.implementation_id.trim() != self.implementation_id
            || self.implementation_id.chars().any(char::is_whitespace)
        {
            return Err(TypeRegistrationError::InvalidImplementationRef);
        }
        let Some(digest) = self.content_digest.strip_prefix("sha256:") else {
            return Err(TypeRegistrationError::InvalidImplementationDigest {
                digest: self.content_digest.clone(),
            });
        };
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(TypeRegistrationError::InvalidImplementationDigest {
                digest: self.content_digest.clone(),
            });
        }
        Ok(())
    }
}

impl TypeRegistryBuilder {
    /// Registers one implementation of a core or package constraint for an exact term.
    ///
    /// `authority` is supplied by the trusted package host. A core constraint can only be
    /// implemented by the package that owns the target nominal type. A package constraint
    /// can be implemented only by its defining package, even for a foreign target type.
    /// Core-owned types and core implementations are immutable.
    pub fn register_constraint_implementation(
        &mut self,
        authority: &str,
        target: &TypeTerm,
        constraint: ConstraintId,
        implementation: ConstraintImplementationRef,
    ) -> Result<(), TypeRegistrationError> {
        self.register_implementation(authority, target, constraint, implementation)
    }

    /// Registers exact evidence for one numeric operation on an exact term.
    ///
    /// A `num` constraint alone does not prove a particular operation. Numeric operation
    /// metadata must identify its own implementation reference.
    pub fn register_numeric_operation_implementation(
        &mut self,
        authority: &str,
        target: &TypeTerm,
        operation: NumericOperation,
        implementation: ConstraintImplementationRef,
    ) -> Result<(), TypeRegistrationError> {
        let owner = self.validate_implementation_target(authority, target, &implementation)?;
        let TypeOwner::Package(namespace) = owner else {
            return Err(TypeRegistrationError::CoreTypeImplementationDenied {
                ty: target.to_string(),
            });
        };
        if namespace != authority {
            return Err(TypeRegistrationError::CoreConstraintOwnerMismatch {
                ty: target.to_string(),
                owner: namespace,
                authority: authority.to_owned(),
            });
        }
        let key = (target.clone(), operation);
        if self.numeric_implementations.contains_key(&key) {
            return Err(
                TypeRegistrationError::DuplicateNumericOperationImplementation {
                    ty: target.to_string(),
                    operation: format!("{operation:?}").to_ascii_lowercase(),
                },
            );
        }
        self.numeric_implementations.insert(key, implementation);
        Ok(())
    }

    fn register_implementation(
        &mut self,
        authority: &str,
        target: &TypeTerm,
        constraint: ConstraintId,
        implementation: ConstraintImplementationRef,
    ) -> Result<(), TypeRegistrationError> {
        let owner = self.validate_implementation_target(authority, target, &implementation)?;
        match &constraint {
            ConstraintId::Core(_) => {
                let TypeOwner::Package(namespace) = owner else {
                    return Err(TypeRegistrationError::CoreTypeImplementationDenied {
                        ty: target.to_string(),
                    });
                };
                if namespace != authority {
                    return Err(TypeRegistrationError::CoreConstraintOwnerMismatch {
                        ty: target.to_string(),
                        owner: namespace,
                        authority: authority.to_owned(),
                    });
                }
            }
            ConstraintId::Package(id) if id.namespace() != authority => {
                return Err(TypeRegistrationError::ConstraintAuthorityMismatch {
                    authority: authority.to_owned(),
                    constraint: id.to_string(),
                });
            }
            ConstraintId::Package(_) => {}
        }
        let key = (target.clone(), constraint.clone());
        if self.implementations.contains_key(&key) {
            return Err(TypeRegistrationError::DuplicateConstraintImplementation {
                ty: target.to_string(),
                constraint: constraint.to_string(),
            });
        }
        if let ConstraintId::Core(core_constraint) = constraint {
            let TypeTerm::Named(named) = target else {
                unreachable!("a validated concrete target is named")
            };
            if self.definitions[&super::identity::NamedTypeId::from_term(named)]
                .derives
                .contains(&core_constraint)
            {
                return Err(TypeRegistrationError::DuplicateConstraintImplementation {
                    ty: target.to_string(),
                    constraint: core_constraint.id_name().to_owned(),
                });
            }
        }
        self.implementations.insert(key, implementation);
        Ok(())
    }

    fn validate_implementation_target(
        &self,
        authority: &str,
        target: &TypeTerm,
        implementation: &ConstraintImplementationRef,
    ) -> Result<TypeOwner, TypeRegistrationError> {
        implementation.validate(authority)?;
        let TypeTerm::Named(named) = target else {
            return Err(TypeRegistrationError::InvalidImplementationTarget {
                ty: target.to_string(),
            });
        };
        crate::types::registry::validate_type(&self.definitions, target, 0).map_err(|error| {
            TypeRegistrationError::InvalidImplementationTargetType {
                ty: target.to_string(),
                error,
            }
        })?;
        let id = super::identity::NamedTypeId::from_term(named);
        self.definitions
            .get(&id)
            .map(|definition| definition.owner.clone())
            .ok_or_else(|| TypeRegistrationError::InvalidImplementationTarget {
                ty: target.to_string(),
            })
    }
}

impl TypeRegistry {
    pub(super) fn explicit_constraint_implementation(
        &self,
        target: &TypeTerm,
        constraint: &ConstraintId,
    ) -> Option<&ConstraintImplementationRef> {
        self.implementations
            .get(&(target.clone(), constraint.clone()))
    }

    pub(super) fn explicit_numeric_implementation(
        &self,
        target: &TypeTerm,
        operation: NumericOperation,
    ) -> Option<&ConstraintImplementationRef> {
        self.numeric_implementations
            .get(&(target.clone(), operation))
    }
}

impl std::fmt::Display for ConstraintId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Core(constraint) => write!(f, "core/{}@1", constraint.id_name()),
            Self::Package(id) => id.fmt(f),
        }
    }
}

pub(super) type ConstraintImplementations =
    HashMap<(TypeTerm, ConstraintId), ConstraintImplementationRef>;
pub(super) type NumericImplementations =
    HashMap<(TypeTerm, NumericOperation), ConstraintImplementationRef>;
