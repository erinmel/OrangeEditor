//! Registry construction, type validation and the literal-schema adapter.

use std::collections::HashMap;
use std::sync::Arc;

use crate::id::TypeExpr;
use crate::type_term::{MAX_NESTING, NamedTerm, TypeTerm};
use crate::value::literal_codec::{EnumSchema, RecordSchema, Schemas};

use super::errors::{TypeRegistrationError, TypeResolveError, validate_member_ids};
use super::identity::{NamedTypeId, RegistryIdentity, TypeKey, TypeKind, TypeOwner};

#[derive(Debug, Clone)]
enum TypeData {
    Core,
    Record(RecordSchema),
    Enum(EnumSchema),
    HandleKind,
}

impl TypeData {
    fn kind(&self) -> TypeKind {
        match self {
            Self::Core => TypeKind::Core,
            Self::Record(_) => TypeKind::Record,
            Self::Enum(_) => TypeKind::Enum,
            Self::HandleKind => TypeKind::HandleKind,
        }
    }
}

#[derive(Debug, Clone)]
struct RegisteredType {
    owner: TypeOwner,
    arity: usize,
    data: TypeData,
}

/// Builder for one type snapshot. Package metadata and resolved keys are assembled
/// before publication; the resulting registry is immutable.
#[derive(Debug)]
pub struct TypeRegistryBuilder {
    owner: Arc<RegistryIdentity>,
    definitions: HashMap<NamedTypeId, RegisteredType>,
    interned: HashMap<TypeTerm, TypeKey>,
    expressions: Vec<TypeTerm>,
}

impl Default for TypeRegistryBuilder {
    fn default() -> Self {
        Self::core()
    }
}

impl TypeRegistryBuilder {
    /// Creates a builder preloaded with v1 core types and constructors (11 §5.2).
    pub fn core() -> Self {
        let mut definitions = HashMap::new();
        for (name, arity) in core_types() {
            definitions.insert(
                NamedTypeId {
                    namespace: "core".to_owned(),
                    name: (*name).to_owned(),
                    major: 1,
                },
                RegisteredType {
                    owner: TypeOwner::Core,
                    arity: *arity,
                    data: TypeData::Core,
                },
            );
        }
        Self {
            owner: Arc::new(RegistryIdentity),
            definitions,
            interned: HashMap::new(),
            expressions: Vec::new(),
        }
    }

    /// Registers a nominal package record using the codec's existing schema type.
    pub fn register_record(
        &mut self,
        package_namespace: &str,
        ty: &TypeExpr,
        schema: RecordSchema,
    ) -> Result<(), TypeRegistrationError> {
        self.register(package_namespace, ty, TypeData::Record(schema))
    }

    /// Registers a nominal package enum using the codec's existing schema type.
    pub fn register_enum(
        &mut self,
        package_namespace: &str,
        ty: &TypeExpr,
        schema: EnumSchema,
    ) -> Result<(), TypeRegistrationError> {
        self.register(package_namespace, ty, TypeData::Enum(schema))
    }

    /// Registers a nominal opaque handle-kind type. It has no literal schema or codec.
    pub fn register_handle_kind(
        &mut self,
        package_namespace: &str,
        ty: &TypeExpr,
    ) -> Result<(), TypeRegistrationError> {
        self.register(package_namespace, ty, TypeData::HandleKind)
    }

    /// Validates and interns a concrete expression before publishing the registry.
    pub fn resolve(&mut self, term: &TypeTerm) -> Result<TypeKey, TypeResolveError> {
        validate_type(&self.definitions, term, 0)?;
        if let Some(key) = self.interned.get(term) {
            return Ok(key.clone());
        }
        let slot =
            u32::try_from(self.expressions.len()).map_err(|_| TypeResolveError::TooManyTypes)?;
        let key = TypeKey {
            owner: Arc::clone(&self.owner),
            slot,
        };
        self.expressions.push(term.clone());
        self.interned.insert(term.clone(), key.clone());
        Ok(key)
    }

    /// Publishes immutable registration metadata after checking schema references.
    pub fn build(self) -> Result<TypeRegistry, TypeRegistrationError> {
        let registry = TypeRegistry {
            owner: self.owner,
            definitions: self.definitions,
            expressions: self.expressions,
        };
        for (id, definition) in &registry.definitions {
            // Validate referenced type expressions without expanding other schema bodies.
            // Recursive nominal schemas are therefore bounded by the codec's normal
            // MAX_NESTING check when a literal is decoded or encoded.
            match &definition.data {
                TypeData::Record(schema) => {
                    validate_member_ids(
                        &id.to_string(),
                        schema.fields.iter().map(|field| field.field_id.as_str()),
                    )?;
                    for field in &schema.fields {
                        validate_type(&registry.definitions, &field.ty, 0).map_err(|error| {
                            TypeRegistrationError::InvalidSchemaType {
                                owner: id.to_string(),
                                error,
                            }
                        })?;
                    }
                }
                TypeData::Enum(schema) => {
                    validate_member_ids(
                        &id.to_string(),
                        schema
                            .variants
                            .iter()
                            .map(|variant| variant.variant_id.as_str()),
                    )?;
                    for variant in &schema.variants {
                        if let Some(payload) = &variant.payload {
                            validate_type(&registry.definitions, payload, 0).map_err(|error| {
                                TypeRegistrationError::InvalidSchemaType {
                                    owner: id.to_string(),
                                    error,
                                }
                            })?;
                        }
                    }
                }
                TypeData::Core | TypeData::HandleKind => {}
            }
        }
        Ok(registry)
    }

    fn register(
        &mut self,
        package_namespace: &str,
        ty: &TypeExpr,
        data: TypeData,
    ) -> Result<(), TypeRegistrationError> {
        let term = TypeTerm::parse(ty).map_err(TypeRegistrationError::InvalidTypeExpr)?;
        let TypeTerm::Named(named) = term else {
            return Err(TypeRegistrationError::ExpectedNamedType { ty: ty.to_string() });
        };
        let id = NamedTypeId::from_term(&named);
        if id.namespace == "core" || package_namespace == "core" {
            return Err(TypeRegistrationError::ReservedCoreNamespace { ty: id.to_string() });
        }
        if !named.args.is_empty() {
            return Err(TypeRegistrationError::GenericNominalType { ty: id.to_string() });
        }
        if package_namespace != named.namespace {
            return Err(TypeRegistrationError::OwnerNamespaceMismatch {
                ty: id.to_string(),
                package_namespace: package_namespace.to_owned(),
            });
        }
        if self.definitions.contains_key(&id) {
            return Err(TypeRegistrationError::DuplicateType { ty: id.to_string() });
        }
        self.definitions.insert(
            id,
            RegisteredType {
                owner: TypeOwner::Package(package_namespace.to_owned()),
                arity: 0,
                data,
            },
        );
        Ok(())
    }
}

/// A registry with immutable type metadata and a frozen expression interner.
///
/// The registry is deliberately not `Clone`: cloned registries with divergent interners
/// must not share key identity. Builders are the only way to add package metadata; after
/// [`TypeRegistryBuilder::build`], definitions, ownership, schemas, and kinds are read-only.
#[derive(Debug)]
pub struct TypeRegistry {
    owner: Arc<RegistryIdentity>,
    definitions: HashMap<NamedTypeId, RegisteredType>,
    expressions: Vec<TypeTerm>,
}

impl Default for TypeRegistry {
    fn default() -> Self {
        Self::core()
    }
}

impl TypeRegistry {
    /// Creates a registry containing only v1 core types and constructors.
    pub fn core() -> Self {
        TypeRegistryBuilder::core()
            .build()
            .expect("the built-in core type table has valid schemas")
    }

    /// Validates a concrete type expression without interning or mutating the registry.
    pub fn validate(&self, term: &TypeTerm) -> Result<(), TypeResolveError> {
        validate_type(&self.definitions, term, 0)
    }

    /// Returns the structural expression for a key, rejecting keys from other registries.
    pub fn expression(&self, key: &TypeKey) -> Result<&TypeTerm, TypeResolveError> {
        if !Arc::ptr_eq(&self.owner, &key.owner) {
            return Err(TypeResolveError::ForeignKey);
        }
        self.expressions
            .get(key.slot as usize)
            .ok_or(TypeResolveError::ForeignKey)
    }

    /// Returns the immutable package or language owner of a resolved type expression.
    pub fn owner(&self, key: &TypeKey) -> Result<&TypeOwner, TypeResolveError> {
        let term = self.expression(key)?;
        self.owner_of(term)
    }

    /// Returns the registered kind of the expression's outer named constructor.
    pub fn kind(&self, key: &TypeKey) -> Result<TypeKind, TypeResolveError> {
        let term = self.expression(key)?;
        self.kind_of(term)
    }

    /// Returns the immutable owner of a validated term's outer named constructor.
    ///
    /// Unlike [`Self::owner`], this accepts structural terms that were never interned,
    /// such as a converter endpoint inspected after registry publication.
    pub fn owner_of(&self, term: &TypeTerm) -> Result<&TypeOwner, TypeResolveError> {
        self.validate(term)?;
        let named = match term {
            TypeTerm::Named(named) => named,
            TypeTerm::Param(name) => {
                return Err(TypeResolveError::UnresolvedParameter { name: name.clone() });
            }
        };
        self.definition(named)
            .map(|definition| &definition.owner)
            .ok_or_else(|| unknown_type(named))
    }

    /// Returns the kind of a validated term's outer named constructor without interning it.
    pub fn kind_of(&self, term: &TypeTerm) -> Result<TypeKind, TypeResolveError> {
        self.validate(term)?;
        let named = match term {
            TypeTerm::Named(named) => named,
            TypeTerm::Param(name) => {
                return Err(TypeResolveError::UnresolvedParameter { name: name.clone() });
            }
        };
        self.definition(named)
            .map(|definition| definition.data.kind())
            .ok_or_else(|| unknown_type(named))
    }

    fn definition(&self, named: &NamedTerm) -> Option<&RegisteredType> {
        self.definitions.get(&NamedTypeId::from_term(named))
    }
}

fn validate_type(
    definitions: &HashMap<NamedTypeId, RegisteredType>,
    term: &TypeTerm,
    depth: usize,
) -> Result<(), TypeResolveError> {
    if depth > MAX_NESTING {
        return Err(TypeResolveError::TooDeep { max: MAX_NESTING });
    }
    match term {
        TypeTerm::Param(name) => Err(TypeResolveError::UnresolvedParameter { name: name.clone() }),
        TypeTerm::Named(named) => {
            let definition = definitions
                .get(&NamedTypeId::from_term(named))
                .ok_or_else(|| unknown_type(named))?;
            if named.args.len() != definition.arity {
                return Err(TypeResolveError::WrongArity {
                    namespace: named.namespace.clone(),
                    name: named.name.clone(),
                    major: named.major,
                    expected: definition.arity,
                    found: named.args.len(),
                });
            }
            for arg in &named.args {
                validate_type(definitions, arg, depth + 1)?;
            }
            if named.namespace == "core" && named.name == "handle" {
                let [argument] = named.args.as_slice() else {
                    unreachable!("core/handle arity was validated above")
                };
                let TypeTerm::Named(handle_kind) = argument else {
                    return Err(TypeResolveError::InvalidHandleKind);
                };
                if !matches!(
                    definitions
                        .get(&NamedTypeId::from_term(handle_kind))
                        .map(|definition| &definition.data),
                    Some(TypeData::HandleKind)
                ) {
                    return Err(TypeResolveError::InvalidHandleKind);
                }
            }
            Ok(())
        }
    }
}

impl Schemas for TypeRegistry {
    fn record(&self, ty: &NamedTerm) -> Option<&RecordSchema> {
        if !ty.args.is_empty() {
            return None;
        }
        match &self.definition(ty)?.data {
            TypeData::Record(schema) => Some(schema),
            TypeData::Core | TypeData::Enum(_) | TypeData::HandleKind => None,
        }
    }

    fn enumeration(&self, ty: &NamedTerm) -> Option<&EnumSchema> {
        if !ty.args.is_empty() {
            return None;
        }
        match &self.definition(ty)?.data {
            TypeData::Enum(schema) => Some(schema),
            TypeData::Core | TypeData::Record(_) | TypeData::HandleKind => None,
        }
    }
}

fn core_types() -> &'static [(&'static str, usize)] {
    &[
        ("bool", 0),
        ("i32", 0),
        ("i64", 0),
        ("u32", 0),
        ("u64", 0),
        ("f32", 0),
        ("f64", 0),
        ("string", 0),
        ("bytes", 0),
        ("duration", 0),
        ("timestamp", 0),
        ("list", 1),
        ("map", 2),
        ("option", 1),
        ("result", 1),
        ("error", 0),
        ("json", 0),
        ("stream", 1),
        ("secret-ref", 0),
        ("handle", 1),
    ]
}

fn unknown_type(named: &NamedTerm) -> TypeResolveError {
    TypeResolveError::UnknownType {
        namespace: named.namespace.clone(),
        name: named.name.clone(),
        major: named.major,
    }
}
