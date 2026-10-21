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
    #[error("`core/handle@1` requires a registered handle-kind type argument")]
    InvalidHandleKind,
    #[error("map key type `{ty}` must implement language `hash` and `eq` and cannot be a float")]
    InvalidMapKey { ty: String },
    #[error("type `{ty}` has an unsupported recursive `{constraint}` derivation")]
    RecursiveDerivation { ty: String, constraint: String },
    #[error("structural derivation exceeds the nominal-schema depth limit of {max}")]
    DerivationTooDeep { max: usize },
    #[error("this key does not belong to the registry being queried")]
    ForeignKey,
    #[error("registry contains more types than can be represented by a type key")]
    TooManyTypes,
}

/// Failure to register a package type or validate its schema before publication.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TypeRegistrationError {
    #[error(transparent)]
    InvalidTypeExpr(#[from] crate::error::IdError),
    #[error("expected a canonical named type, found `{ty}`")]
    ExpectedNamedType { ty: String },
    #[error("package type `{ty}` cannot be registered in the reserved core namespace")]
    ReservedCoreNamespace { ty: String },
    #[error("package type `{ty}` cannot declare type arguments")]
    GenericNominalType { ty: String },
    #[error("type `{ty}` is outside package namespace `{package_namespace}`")]
    OwnerNamespaceMismatch {
        ty: String,
        package_namespace: String,
    },
    #[error("type `{ty}` is already registered")]
    DuplicateType { ty: String },
    #[error("type `{ty}` is not registered in this builder")]
    UnknownType { ty: String },
    #[error("type `{ty}` is not a record or enum and cannot declare a structural derive")]
    NotDerivableType { ty: String },
    #[error("structural derive `{constraint}` is not supported for `{ty}`")]
    UnsupportedDerivation { ty: String, constraint: String },
    #[error("structural derive `{constraint}` is invalid for `{ty}` or one of its members")]
    InvalidDerivedConstraint { ty: String, constraint: String },
    #[error("structural derive `{constraint}` for `{ty}` has a recursive proof cycle")]
    RecursiveDerivation { ty: String, constraint: String },
    #[error("structural derivation exceeds the nominal-schema depth limit of {max}")]
    DerivationTooDeep { max: usize },
    #[error("invalid package constraint identity `{namespace}/{name}@{major}`")]
    InvalidConstraintId {
        namespace: String,
        name: String,
        major: u32,
    },
    #[error("package `{authority}` does not own constraint `{constraint}`")]
    ConstraintAuthorityMismatch {
        authority: String,
        constraint: String,
    },
    #[error("package `{authority}` does not match implementation provider `{provider}`")]
    ImplementationAuthorityMismatch { authority: String, provider: String },
    #[error(
        "constraint implementation reference contains an invalid provider or implementation ID"
    )]
    InvalidImplementationRef,
    #[error(
        "constraint implementation artifact digest `{digest}` is not a lowercase SHA-256 digest"
    )]
    InvalidImplementationDigest { digest: String },
    #[error("type `{ty}` is core-owned and its built-in constraints cannot be overridden")]
    CoreTypeImplementationDenied { ty: String },
    #[error(
        "package `{authority}` cannot implement a core constraint for `{ty}`, owned by `{owner}`"
    )]
    CoreConstraintOwnerMismatch {
        ty: String,
        owner: String,
        authority: String,
    },
    #[error("type `{ty}` already has an implementation or derive for `{constraint}`")]
    DuplicateConstraintImplementation { ty: String, constraint: String },
    #[error("type `{ty}` already has an implementation for numeric operation `{operation}`")]
    DuplicateNumericOperationImplementation { ty: String, operation: String },
    #[error("constraint implementation target `{ty}` must be a registered concrete named type")]
    InvalidImplementationTarget { ty: String },
    #[error("constraint implementation target `{ty}` is invalid: {error}")]
    InvalidImplementationTargetType {
        ty: String,
        #[source]
        error: TypeResolveError,
    },
    #[error("resolved type `{ty}` has invalid language constraints: {error}")]
    InvalidResolvedType {
        ty: String,
        #[source]
        error: TypeResolveError,
    },
    #[error("schema for `{owner}` references an invalid type: {error}")]
    InvalidSchemaType {
        owner: String,
        #[source]
        error: TypeResolveError,
    },
    #[error("schema for `{owner}` has a blank member ID")]
    BlankMemberId { owner: String },
    #[error("schema for `{owner}` repeats member ID `{member_id}`")]
    DuplicateMemberId { owner: String, member_id: String },
}

pub(super) fn validate_member_ids<'a>(
    owner: &str,
    ids: impl IntoIterator<Item = &'a str>,
) -> Result<(), TypeRegistrationError> {
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if id.trim().is_empty() {
            return Err(TypeRegistrationError::BlankMemberId {
                owner: owner.to_owned(),
            });
        }
        if !seen.insert(id) {
            return Err(TypeRegistrationError::DuplicateMemberId {
                owner: owner.to_owned(),
                member_id: id.to_owned(),
            });
        }
    }
    Ok(())
}
