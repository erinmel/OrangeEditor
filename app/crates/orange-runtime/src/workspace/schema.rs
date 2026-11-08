use super::*;
use orange_model::value::literal_codec;
type Prepared = (
    BTreeMap<VariableId, Variable>,
    BTreeMap<VariableId, Arc<Cell>>,
    usize,
    usize,
);
pub(super) fn prepare(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    limits: WorkspaceLimits,
    budget: &Arc<Budget>,
) -> Result<Prepared, WorkspaceError> {
    let count = document
        .variables()
        .values()
        .filter(|d| d.scope == VarScope::Workspace)
        .take(limits.max_variables.saturating_add(1))
        .count();
    if count > limits.max_variables {
        return Err(WorkspaceError::Limit);
    }
    let mut variables = BTreeMap::new();
    let mut values = BTreeMap::new();
    let mut value_bytes = 0usize;
    let mut schema_bytes = std::mem::size_of::<Schema>();
    if schema_bytes > limits.max_schema_bytes {
        return Err(WorkspaceError::Limit);
    }
    let mut types = BTreeMap::<TypeExpr, ValueShape>::new();
    for declaration in document
        .variables()
        .values()
        .filter(|d| d.scope == VarScope::Workspace)
    {
        let shape = if let Some(shape) = types.get(&declaration.ty) {
            shape.clone()
        } else {
            let shape = ValueShape::of_in(&declaration.ty, registry.types().as_ref())
                .ok_or(WorkspaceError::Contract)?;
            schema_bytes = schema_bytes
                .checked_add(shape_bytes(&shape, limits.max_schema_bytes)?)
                .ok_or(WorkspaceError::Limit)?;
            types.insert(declaration.ty.clone(), shape.clone());
            shape
        };
        schema_bytes = schema_bytes
            .checked_add(std::mem::size_of::<Variable>() + std::mem::size_of::<VariableId>())
            .and_then(|n| n.checked_add(declaration.ty.as_str().len()))
            .filter(|n| *n <= limits.max_schema_bytes)
            .ok_or(WorkspaceError::Limit)?;
        let value = match &declaration.init {
            None => None,
            Some(literal) if literal.ty == declaration.ty => Some(
                literal_codec::decode_with_budget(
                    literal,
                    registry.types().as_ref(),
                    limits.max_value_bytes,
                )
                .map_err(|error| match error {
                    orange_model::LiteralError::TooLarge { .. } => WorkspaceError::Limit,
                    _ => WorkspaceError::Contract,
                })?,
            ),
            Some(_) => return Err(WorkspaceError::Contract),
        };
        let bytes = match &value {
            Some(v) => charge(&shape, v, limits.max_value_bytes)?,
            None => 16,
        };
        value_bytes = value_bytes
            .checked_add(bytes)
            .filter(|n| *n <= limits.max_state_bytes)
            .ok_or(WorkspaceError::Limit)?;
        let value_hold = hold(
            budget,
            bytes
                .checked_add(std::mem::size_of::<Cell>())
                .ok_or(WorkspaceError::Limit)?,
        )?;
        values.insert(
            declaration.id,
            Arc::new(Cell {
                value,
                bytes,
                _hold: value_hold,
            }),
        );
        variables.insert(
            declaration.id,
            Variable {
                ty: declaration.ty.clone(),
                shape,
            },
        );
    }
    Ok((variables, values, value_bytes, schema_bytes))
}
pub(super) fn shape_bytes(root: &ValueShape, limit: usize) -> Result<usize, WorkspaceError> {
    let mut work = vec![root];
    let mut seen = BTreeSet::new();
    let mut bytes = 0usize;
    while let Some(shape) = work.pop() {
        bytes = bytes
            .checked_add(std::mem::size_of::<ValueShape>())
            .filter(|n| *n <= limit)
            .ok_or(WorkspaceError::Limit)?;
        match shape {
            ValueShape::Handle(_) | ValueShape::Stream(_) => return Err(WorkspaceError::Contract),
            ValueShape::List(inner) | ValueShape::Option(inner) | ValueShape::Result(inner) => {
                work.push(inner)
            }
            ValueShape::Map(key, value) => work.extend([key.as_ref(), value.as_ref()]),
            ValueShape::Record(schema) => {
                if !seen.insert(schema.ty().clone()) {
                    continue;
                }
                bytes = bytes
                    .checked_add(schema.ty().as_str().len())
                    .filter(|n| *n <= limit)
                    .ok_or(WorkspaceError::Limit)?;
                for index in 0..schema.len() {
                    bytes = bytes
                        .checked_add(
                            schema
                                .field_id(index)
                                .ok_or(WorkspaceError::Contract)?
                                .len(),
                        )
                        .filter(|n| *n <= limit)
                        .ok_or(WorkspaceError::Limit)?;
                    work.push(schema.field(index).ok_or(WorkspaceError::Contract)?.0);
                }
            }
            ValueShape::Enum(schema) => {
                if !seen.insert(schema.ty().clone()) {
                    continue;
                }
                bytes = bytes
                    .checked_add(schema.ty().as_str().len())
                    .filter(|n| *n <= limit)
                    .ok_or(WorkspaceError::Limit)?;
                for index in 0..schema.len() {
                    let tag = u32::try_from(index).map_err(|_| WorkspaceError::Limit)?;
                    bytes = bytes
                        .checked_add(
                            schema
                                .variant_id(tag)
                                .ok_or(WorkspaceError::Contract)?
                                .len(),
                        )
                        .filter(|n| *n <= limit)
                        .ok_or(WorkspaceError::Limit)?;
                    if let Some(shape) = schema.payload(tag).ok_or(WorkspaceError::Contract)? {
                        work.push(shape);
                    }
                }
            }
            ValueShape::Bool
            | ValueShape::I32
            | ValueShape::I64
            | ValueShape::U32
            | ValueShape::U64
            | ValueShape::F32
            | ValueShape::F64
            | ValueShape::Json
            | ValueShape::String
            | ValueShape::Duration
            | ValueShape::Timestamp
            | ValueShape::Bytes
            | ValueShape::Error => {}
            _ => return Err(WorkspaceError::Contract),
        }
    }
    Ok(bytes)
}
