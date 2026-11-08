//! Versioned length-framed semantic identity, independent of display/layout.
use super::*;
use orange_model::catalog::ProviderKey;
use sha2::{Digest, Sha256};
struct Seal {
    hash: Sha256,
    bytes: usize,
    limit: usize,
}
impl Seal {
    fn field(&mut self, value: &[u8]) -> Result<(), CheckpointError> {
        self.bytes = self
            .bytes
            .checked_add(value.len())
            .and_then(|n| n.checked_add(8))
            .filter(|n| *n <= self.limit)
            .ok_or(CheckpointError::Limit)?;
        self.hash.update((value.len() as u64).to_le_bytes());
        self.hash.update(value);
        Ok(())
    }
    fn text(&mut self, s: &str) -> Result<(), CheckpointError> {
        self.field(s.as_bytes())
    }
    fn number(&mut self, n: u64) -> Result<(), CheckpointError> {
        self.field(&n.to_le_bytes())
    }
    fn shape(&mut self, s: &ValueShape) -> Result<(), CheckpointError> {
        let tag = match s {
            ValueShape::Bool => "bool",
            ValueShape::I32 => "i32",
            ValueShape::I64 => "i64",
            ValueShape::U32 => "u32",
            ValueShape::U64 => "u64",
            ValueShape::F32 => "f32",
            ValueShape::F64 => "f64",
            ValueShape::Json => "json",
            ValueShape::String => "string",
            ValueShape::Duration => "duration",
            ValueShape::Timestamp => "timestamp",
            ValueShape::Bytes => "bytes",
            ValueShape::List(_) => "list",
            ValueShape::Map(_, _) => "map",
            ValueShape::Option(_) => "option",
            ValueShape::Record(_) => "record",
            ValueShape::Enum(_) => "enum",
            _ => return Err(CheckpointError::UnsupportedValue),
        };
        self.text(tag)?;
        match s {
            ValueShape::List(s) | ValueShape::Option(s) => self.shape(s)?,
            ValueShape::Map(k, v) => {
                self.shape(k)?;
                self.shape(v)?;
            }
            ValueShape::Record(r) => {
                self.text(r.ty().as_str())?;
                self.number(r.len() as u64)?;
                self.field(&[u8::from(r.derives_eq()), u8::from(r.derives_key())])?;
                for i in 0..r.len() {
                    self.text(r.field_id(i).ok_or(CheckpointError::Incompatible)?)?;
                    let (shape, optional) = r.field(i).ok_or(CheckpointError::Incompatible)?;
                    self.field(&[u8::from(optional)])?;
                    self.shape(shape)?;
                }
            }
            ValueShape::Enum(e) => {
                self.text(e.ty().as_str())?;
                self.number(e.len() as u64)?;
                self.field(&[u8::from(e.derives_eq()), u8::from(e.derives_key())])?;
                for i in 0..e.len() {
                    let i = u32::try_from(i).map_err(|_| CheckpointError::Limit)?;
                    self.text(e.variant_id(i).ok_or(CheckpointError::Incompatible)?)?;
                    match e.payload(i).ok_or(CheckpointError::Incompatible)? {
                        Some(shape) => {
                            self.number(1)?;
                            self.shape(shape)?;
                        }
                        None => self.number(0)?,
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
pub(super) fn digest(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    scope: CheckpointScope,
    ids: &BTreeSet<VariableId>,
    limits: CheckpointLimits,
) -> Result<String, CheckpointError> {
    let mut seal = Seal {
        hash: Sha256::new(),
        bytes: 0,
        limit: limits.max_bytes,
    };
    seal.text("orange.checkpoint.contract@1")?;
    seal.text(literal_codec::CODEC_ID)?;
    seal.text(&document.graph_id().to_string())?;
    seal.text(match scope {
        CheckpointScope::Run => "run",
        CheckpointScope::Workspace => "workspace",
    })?;
    seal.number(ids.len() as u64)?;
    for id in ids {
        let v = document
            .variables()
            .get(id)
            .filter(|v| v.scope == scope.variable_scope() && v.persistent)
            .ok_or(CheckpointError::Selection)?;
        seal.text(&id.to_string())?;
        seal.text(v.ty.as_str())?;
        seal.shape(&shape(&v.ty, registry)?)?;
    }
    let mut items = ids.len().saturating_add(
        document
            .package_lock()
            .len()
            .saturating_add(document.type_lock().len()),
    );
    if items > limits.max_identity_items {
        return Err(CheckpointError::Limit);
    }
    let mut packages: Vec<_> = document.package_lock().iter().collect();
    packages.sort_by_key(|p| (&p.id, &p.version, &p.content_digest));
    seal.number(packages.len() as u64)?;
    for p in packages {
        seal.text(&p.id)?;
        seal.text(&p.version)?;
        seal.text(&p.content_digest)?;
    }
    let mut types: Vec<_> = document.type_lock().iter().collect();
    types.sort_by_key(|t| (&t.ty, &t.codec));
    seal.number(types.len() as u64)?;
    for t in types {
        let term = orange_model::type_term::TypeTerm::parse(&t.ty)
            .map_err(|_| CheckpointError::Incompatible)?;
        if registry.types().literal_codec_identity(&term) != Some(t.codec.as_str()) {
            return Err(CheckpointError::Incompatible);
        }
        seal.text(t.ty.as_str())?;
        seal.text(&t.codec)?;
    }
    let mut providers = BTreeSet::new();
    let mut provider_bytes = 0usize;
    let nodes = document.root().nodes.values().chain(
        document
            .functions()
            .values()
            .flat_map(|f| f.body.nodes.values()),
    );
    for node in nodes {
        items = items.checked_add(1).ok_or(CheckpointError::Limit)?;
        if items > limits.max_identity_items {
            return Err(CheckpointError::Limit);
        }
        if node.definition.id.is_core() {
            continue;
        }
        let binding = node
            .provider
            .as_ref()
            .ok_or(CheckpointError::Incompatible)?;
        // Bound provider text before constructing the registry lookup's owned key.
        if [&binding.package, &binding.version, &binding.id]
            .iter()
            .any(|s| s.len() > limits.max_bytes)
        {
            return Err(CheckpointError::Limit);
        }
        let key = ProviderKey::of(document, binding).ok_or(CheckpointError::Incompatible)?;
        for text in [
            node.definition.id.as_str(),
            &key.package,
            &key.version,
            &key.content_digest,
            &key.id,
        ] {
            provider_bytes = provider_bytes
                .checked_add(text.len())
                .and_then(|n| n.checked_add(8))
                .filter(|n| *n <= limits.max_bytes)
                .ok_or(CheckpointError::Limit)?;
        }
        if !registry.provides(&node.definition, node.config_version, &key) {
            return Err(CheckpointError::Incompatible);
        }
        providers.insert((
            node.definition.id.as_str().to_owned(),
            node.definition.version,
            node.config_version,
            key.package,
            key.version,
            key.content_digest,
            key.id,
        ));
    }
    seal.number(providers.len() as u64)?;
    for (definition, version, config, package, pkg_version, digest, id) in providers {
        seal.text(&definition)?;
        seal.number(version.into())?;
        seal.number(config.into())?;
        seal.text(&package)?;
        seal.text(&pkg_version)?;
        seal.text(&digest)?;
        seal.text(&id)?;
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in seal.hash.finalize() {
        out.push(char::from(HEX[(byte >> 4) as usize]));
        out.push(char::from(HEX[(byte & 15) as usize]));
    }
    Ok(out)
}
