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
