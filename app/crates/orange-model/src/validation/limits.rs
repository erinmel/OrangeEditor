//! Host-configurable admission limits and cooperative cancellation for combined analysis.

use crate::catalog::intrinsic_id as ids;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use crate::catalog::{Ports, RegistrySnapshot};
use crate::document::literal::{Literal, LiteralPayload};
use crate::document::{ConfigValue, GraphDocument, PortDescriptor};
use crate::id::NodeId;

use super::{DiagnosticCode, LeafEffects};

/// Admission quotas, checked before building indices or resolved projections.
/// These bound input size, not elapsed time, allocations or algorithm instruction counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisLimits {
    /// Aggregate graph items and inspected port, type-argument and leaf-effect metadata.
    pub max_items: usize,
    /// Aggregate JSON values in configurations, schemas, defaults and extension metadata.
    pub max_json_values: usize,
    /// Aggregate UTF-8 bytes of inspected semantic metadata, including JSON object keys.
    pub max_metadata_bytes: usize,
    /// Square of the aggregate item count: a deliberately conservative admission estimate
    /// for passes that revisit pairs of graph items. This is not a CPU-work guarantee.
    pub max_pair_estimate: u64,
}

impl Default for AnalysisLimits {
    fn default() -> Self {
        Self {
            max_items: 250_000,
            max_json_values: 1_000_000,
            max_metadata_bytes: 64 * 1024 * 1024,
            max_pair_estimate: 25_000_000_000,
        }
    }
}

/// Controls for one combined analysis invocation. Defaults also apply to legacy entry points.
/// Cancellation is polled during admission, between passes and between function bodies;
/// it does not interrupt an individual pass. Hosts needing isolation must run a worker.
#[derive(Debug, Clone, Copy, Default)]
pub struct AnalysisOptions<'a> {
    pub limits: AnalysisLimits,
    pub cancelled: Option<&'a AtomicBool>,
}

impl AnalysisOptions<'_> {
    pub(super) fn checkpoint(&self) -> Result<(), DiagnosticCode> {
        if self
            .cancelled
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            Err(DiagnosticCode::AnalysisCancelled)
        } else {
            Ok(())
        }
    }

    pub(super) fn admit(
        &self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        effects: &BTreeMap<NodeId, LeafEffects>,
    ) -> Result<Admitted, DiagnosticCode> {
        self.checkpoint()?;
        let mut budget = Budget {
            options: self,
            items: 0,
            values: 0,
            bytes: 0,
        };
        budget.items(document.package_lock().len())?;
        for lock in document.package_lock() {
            budget.text(&lock.id)?;
            budget.text(&lock.version)?;
            budget.text(&lock.content_digest)?;
        }
        budget.items(document.exports().len())?;
        for export in document.exports() {
            budget.text(&export.id)?;
            budget.text(export.port.port().as_str())?;
        }
        for (_, body) in document.bodies_with_refs() {
            budget.items(body.nodes.len())?;
            budget.items(body.edges.len())?;
            for edge in body.edges.values() {
                budget.text(edge.from.port.port().as_str())?;
                budget.text(edge.to.port.port().as_str())?;
            }
            for node in body.nodes.values() {
                budget.text(node.definition.id.as_str())?;
                if let Some(provider) = &node.provider {
                    budget.text(&provider.package)?;
                    budget.text(&provider.version)?;
                    budget.text(&provider.id)?;
                }
                budget.object(&node.config)?;
                budget.object(&node.extensions)?;
                budget.items(node.type_args.len())?;
                budget.items(node.type_pins.len())?;
                budget.items(node.variadic.len())?;
                for (name, ty) in node.type_args.iter().chain(&node.type_pins) {
                    budget.text(name)?;
                    budget.text(ty.as_str())?;
                }
                for literal in node.inputs.values() {
                    budget.literal(literal)?;
                }
                for state in node.variadic.values() {
                    budget.items(state.order.len())?;
                }
                if let Some(key) = &node.signature {
                    budget.text(key.as_str())?;
                }
                if let Some(signature) = node
                    .signature
                    .as_ref()
                    .and_then(|key| document.signatures().get(key))
                {
                    budget.ports(&signature.ports)?;
                    budget.object(&signature.extensions)?;
                }
                // Borrow definitions: projecting contextual contracts here would clone
                // the very metadata that admission is meant to inspect first.
                if let Some(definition) = registry.definition(&node.definition) {
                    budget.parameters(&definition.type_params)?;
                    budget.items(definition.references.len())?;
                    for reference in &definition.references {
                        budget.text(&reference.pointer)?;
                    }
                    if let Some(specializations) = &definition.specializations {
                        budget.items(specializations.len())?;
                        for specialization in specializations {
                            budget.items(specialization.len())?;
                            for ty in specialization.values() {
                                budget.text(ty.as_str())?;
                            }
                        }
                    }
                    if let Ports::Declared(ports) = &definition.ports {
                        budget.ports(ports)?;
                    }
                    if let Some(schema) = &definition.config_schema {
                        budget.json(schema)?;
                    }
                }
                if node.definition.id.as_str() == ids::CALL
                    && let Some(function) = node
                        .config
                        .get("function")
                        .and_then(Value::as_str)
                        .and_then(|id| id.parse::<crate::id::FunctionId>().ok())
                        .and_then(|id| document.functions().get(&id))
                {
                    budget.ports(&function.inputs)?;
                    budget.ports(&function.outputs)?;
                }
            }
        }
        budget.items(document.variables().len())?;
        for variable in document.variables().values() {
            budget.text(variable.ty.as_str())?;
            if let Some(literal) = &variable.init {
                budget.literal(literal)?;
            }
        }
        budget.items(document.events().len())?;
        for event in document.events().values() {
            budget.text(event.payload.as_str())?;
        }
        budget.items(document.functions().len())?;
        for function in document.functions().values() {
            budget.ports(&function.inputs)?;
            budget.ports(&function.outputs)?;
            budget.parameters(&function.type_params)?;
        }
        for effect in effects.values() {
            budget.items(1)?;
            if let LeafEffects::Known(writes) = effect {
                budget.items(writes.len())?;
            }
        }
        self.checkpoint()?;
        Ok(Admitted {
            items: budget.items,
            values: budget.values,
            bytes: budget.bytes,
        })
    }

    /// Whether totals fit every quota. Charges only grow, so admission of a document
    /// succeeds exactly when its final totals fit.
    pub(super) fn fits(&self, totals: Admitted) -> bool {
        let limits = &self.limits;
        totals.items <= limits.max_items
            && totals.values <= limits.max_json_values
            && totals.bytes <= limits.max_metadata_bytes
            && u64::try_from(totals.items)
                .ok()
                .and_then(|items| items.checked_mul(items))
                .is_some_and(|pairs| pairs <= limits.max_pair_estimate)
    }
}

/// What admission charged a document: items, JSON values and metadata bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Admitted {
    pub items: usize,
    pub values: usize,
    pub bytes: usize,
}

impl Admitted {
    /// What admission charges for one node's literals, type arguments and type pins.
    pub(super) fn of_node_fields(
        inputs: &BTreeMap<crate::id::PortKey, Literal>,
        type_args: &BTreeMap<String, crate::id::TypeExpr>,
        type_pins: &BTreeMap<String, crate::id::TypeExpr>,
    ) -> Self {
        let unlimited = AnalysisOptions {
            limits: AnalysisLimits {
                max_items: usize::MAX,
                max_json_values: usize::MAX,
                max_metadata_bytes: usize::MAX,
                max_pair_estimate: u64::MAX,
            },
            cancelled: None,
        };
        let mut budget = Budget {
            options: &unlimited,
            items: 0,
            values: 0,
            bytes: 0,
        };
        // Only an addition past `usize::MAX` could fail; saturate like the totals.
        let charged = (|| {
            budget.items(type_args.len())?;
            budget.items(type_pins.len())?;
            for (name, ty) in type_args.iter().chain(type_pins) {
                budget.text(name)?;
                budget.text(ty.as_str())?;
            }
            for literal in inputs.values() {
                budget.literal(literal)?;
            }
            Ok::<_, DiagnosticCode>(())
        })();
        if charged.is_err() {
            return Self {
                items: usize::MAX,
                values: usize::MAX,
                bytes: usize::MAX,
            };
        }
        Self {
            items: budget.items,
            values: budget.values,
            bytes: budget.bytes,
        }
    }

    /// `self` plus `other`, or `None` on overflow.
    pub(super) fn add(self, other: Self) -> Option<Self> {
        self.replace(Self::default(), other)
    }

    /// What admission charges for one edge.
    pub(super) fn of_edge(edge: &crate::document::Edge) -> Self {
        Self {
            items: 1,
            values: 0,
            bytes: edge.from.port.port().as_str().len() + edge.to.port.port().as_str().len(),
        }
    }

    /// `self` with `removed` taken out and `added` put in, or `None` on overflow.
    pub(super) fn replace(self, removed: Self, added: Self) -> Option<Self> {
        let swap = |total: usize, old: usize, new: usize| total.checked_sub(old)?.checked_add(new);
        Some(Self {
            items: swap(self.items, removed.items, added.items)?,
            values: swap(self.values, removed.values, added.values)?,
            bytes: swap(self.bytes, removed.bytes, added.bytes)?,
        })
    }
}

struct Budget<'a, 'b> {
    options: &'a AnalysisOptions<'b>,
    items: usize,
    values: usize,
    bytes: usize,
}

fn charge(total: &mut usize, count: usize, limit: usize) -> Result<(), DiagnosticCode> {
    *total = total
        .checked_add(count)
        .filter(|value| *value <= limit)
        .ok_or(DiagnosticCode::AnalysisLimit)?;
    Ok(())
}

impl Budget<'_, '_> {
    fn parameters(&mut self, params: &[crate::document::TypeParam]) -> Result<(), DiagnosticCode> {
        self.items(params.len())?;
        for param in params {
            self.text(&param.name)?;
            self.items(param.constraints.len())?;
            for constraint in &param.constraints {
                self.text(constraint)?;
            }
            self.object(&param.extensions)?;
        }
        Ok(())
    }

    fn items(&mut self, count: usize) -> Result<(), DiagnosticCode> {
        self.options.checkpoint()?;
        charge(&mut self.items, count, self.options.limits.max_items)?;
        let items = u64::try_from(self.items).map_err(|_| DiagnosticCode::AnalysisLimit)?;
        if items
            .checked_mul(items)
            .is_none_or(|pairs| pairs > self.options.limits.max_pair_estimate)
        {
            return Err(DiagnosticCode::AnalysisLimit);
        }
        Ok(())
    }

    fn text(&mut self, text: &str) -> Result<(), DiagnosticCode> {
        self.options.checkpoint()?;
        charge(
            &mut self.bytes,
            text.len(),
            self.options.limits.max_metadata_bytes,
        )
    }

    fn object(&mut self, object: &ConfigValue) -> Result<(), DiagnosticCode> {
        charge(&mut self.values, 1, self.options.limits.max_json_values)?;
        for (key, value) in object {
            self.text(key)?;
            self.json(value)?;
        }
        Ok(())
    }

    fn json(&mut self, value: &Value) -> Result<(), DiagnosticCode> {
        // Charge children before extending the stack, so a wide value cannot allocate
        // an unbounded work list before its quota is checked. No payload is cloned.
        charge(&mut self.values, 1, self.options.limits.max_json_values)?;
        let mut stack = vec![value];
        while let Some(value) = stack.pop() {
            self.options.checkpoint()?;
            match value {
                Value::String(text) => self.text(text)?,
                Value::Array(values) => {
                    charge(
                        &mut self.values,
                        values.len(),
                        self.options.limits.max_json_values,
                    )?;
                    stack.extend(values);
                }
                Value::Object(values) => {
                    charge(
                        &mut self.values,
                        values.len(),
                        self.options.limits.max_json_values,
                    )?;
                    for (key, value) in values {
                        self.text(key)?;
                        stack.push(value);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn literal(&mut self, literal: &Literal) -> Result<(), DiagnosticCode> {
        self.items(1)?;
        self.text(literal.ty.as_str())?;
        match &literal.payload {
            LiteralPayload::Value(value) => self.json(value)?,
            LiteralPayload::Bits(text) => self.text(text)?,
        }
        if let Some(display) = &literal.display {
            self.text(display)?;
        }
        Ok(())
    }

    fn ports(&mut self, ports: &[PortDescriptor]) -> Result<(), DiagnosticCode> {
        self.items(ports.len())?;
        for port in ports {
            self.text(port.port_id.as_str())?;
            if let Some(ty) = &port.ty {
                self.text(ty.as_str())?;
            }
            if let Some(default) = &port.default {
                self.literal(default)?;
            }
            if let Some(outcomes) = &port.available_on {
                self.items(outcomes.len())?;
                for outcome in outcomes {
                    self.text(outcome.as_str())?;
                }
            }
            self.object(&port.extensions)?;
        }
        Ok(())
    }
}
