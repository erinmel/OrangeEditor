//! The `orange:expansion/plugin@1.0.0` world (sdk/wit): grant-scoped host imports and
//! node evaluation. Values cross as `orange.value.v1` literals, or in a binary case
//! for bytes and dense numeric lists (K02b).

use crate::limits::InstanceLimits;
use orange_model::Value;
use orange_model::document::literal::Literal;
use orange_model::id::TypeExpr;
use orange_model::types::TypeRegistry;
use orange_model::value::literal_codec;
use wasmtime::Store;
use wasmtime::component::{HasSelf, Linker};

use crate::{GuestCancellation, GuestComponent, GuestError, GuestTrap, classify};

wasmtime::component::bindgen!({
    path: "../../../sdk/wit",
    world: "plugin",
});

use orange::expansion::types::Value as Wire;
use orange::expansion::{host, log};
use orange_model::value::ListRepr;
use std::sync::Arc;

/// The host API version offered through `host.api-version` (04 SDK surface).
pub const API_VERSION: (u16, u16) = (1, 0);
/// Largest number of retained log entries per instance; later entries are dropped.
pub const MAX_LOG_ENTRIES: usize = 64;
/// Longest retained log entry in bytes; longer entries are cut at a character boundary.
pub const MAX_LOG_BYTES: usize = 1024;

/// Host capabilities granted to one instance. `host` is always linked; every other
/// import is linked only when granted, so an ungranted import fails instantiation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Grants {
    pub log: bool,
}

struct PluginState {
    limits: InstanceLimits,
    log: Vec<String>,
}

impl host::Host for PluginState {
    fn api_version(&mut self) -> (u16, u16) {
        API_VERSION
    }
}

impl log::Host for PluginState {
    fn write(&mut self, message: String) {
        if self.log.len() >= MAX_LOG_ENTRIES {
            return;
        }
        let mut end = message.len().min(MAX_LOG_BYTES);
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        self.log.push(message[..end].to_owned());
    }
}

/// A node failure reported by the guest, with checked namespaced identifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestNodeError {
    pub code: String,
    pub message_key: String,
}

/// One instantiated plugin with its own store, limits and grants.
pub struct GuestPlugin {
    store: Store<PluginState>,
    bindings: Plugin,
    fuel: u64,
    output_bytes: usize,
    poisoned: bool,
    cancellation: GuestCancellation,
}

impl std::fmt::Debug for GuestPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestPlugin")
            .field("poisoned", &self.poisoned)
            .finish_non_exhaustive()
    }
}

fn namespaced(text: &str) -> bool {
    text.split_once('/').is_some_and(|(namespace, name)| {
        !namespace.is_empty()
            && !name.is_empty()
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'/'))
    })
}

#[derive(Clone, Copy)]
enum Export {
    Evaluate,
    Execute,
}

const BYTES: &str = "core/bytes@1";

/// The binary wire case of a dense numeric list, for exactly its own list type.
macro_rules! dense_lists {
    ($($repr:ident, $wire:ident, $ty:literal;)*) => {
        fn dense_to_wire(ty: &str, list: &ListRepr) -> Option<Wire> {
            match (ty, list) {
                $(($ty, ListRepr::$repr(items)) => Some(Wire::$wire(items.to_vec())),)*
                _ => None,
            }
        }
        fn dense_from_wire(wire: Wire, ty: &str) -> Result<Value, GuestError> {
            match (wire, ty) {
                $((Wire::$wire(items), $ty) => {
                    Ok(Value::List(Arc::new(ListRepr::$repr(items.into_boxed_slice()))))
                })*
                _ => Err(GuestError::OutputContract),
            }
        }
    };
}
dense_lists! {
    I32, I32List, "core/list@1<core/i32@1>";
    I64, I64List, "core/list@1<core/i64@1>";
    U32, U32List, "core/list@1<core/u32@1>";
    U64, U64List, "core/list@1<core/u64@1>";
    F32, F32List, "core/list@1<core/f32@1>";
    F64, F64List, "core/list@1<core/f64@1>";
}

/// Bytes and dense numeric lists cross without text conversion; everything else, and
/// a list the host does not hold densely, crosses as a literal.
fn to_wire(ty: &TypeExpr, value: &Value, types: &TypeRegistry) -> Result<Wire, GuestError> {
    let binary = match value {
        Value::Bytes(bytes) if ty.as_str() == BYTES => Some(Wire::Bytes(bytes.to_vec())),
        Value::List(list) => dense_to_wire(ty.as_str(), list),
        _ => None,
    };
    if let Some(wire) = binary {
        return Ok(wire);
    }
    let literal = literal_codec::encode(ty, value, types).map_err(|_| GuestError::InputContract)?;
    serde_json::to_string(&literal)
        .map(Wire::Literal)
        .map_err(|_| GuestError::InputContract)
}

/// A guest value for an output of type `ty`. The caller still checks its shape and
/// charges it; a binary case is accepted only for exactly its own type.
fn from_wire(
    wire: Wire,
    ty: &TypeExpr,
    types: &TypeRegistry,
    budget: usize,
) -> Result<Value, GuestError> {
    match wire {
        Wire::Literal(text) => {
            let literal: Literal =
                serde_json::from_str(&text).map_err(|_| GuestError::OutputContract)?;
            if &literal.ty != ty {
                return Err(GuestError::OutputContract);
            }
            literal_codec::decode_with_budget(&literal, types, budget)
                .map_err(|_| GuestError::OutputContract)
        }
        Wire::Bytes(bytes) if ty.as_str() == BYTES => Ok(Value::Bytes(Arc::new(bytes))),
        Wire::Bytes(_) => Err(GuestError::OutputContract),
        dense => dense_from_wire(dense, ty.as_str()),
    }
}

impl GuestPlugin {
    /// Instantiates `component` as an `orange:expansion/plugin` with exactly `grants`.
    pub fn new(component: &GuestComponent, grants: Grants) -> Result<Self, GuestError> {
        let engine = &component.engine;
        let limits = engine.limits;
        let mut store = Store::new(
            &engine.engine,
            PluginState {
                limits: InstanceLimits::new(limits),
                log: Vec::new(),
            },
        );
        store.limiter(|state| &mut state.limits);
        store.set_hostcall_fuel(limits.transfer_bytes);
        let cancellation = GuestCancellation::install(engine, &mut store)?;
        store
            .set_fuel(limits.fuel_per_call)
            .map_err(|_| GuestError::Engine)?;
        let mut linker = Linker::<PluginState>::new(&engine.engine);
        host::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .map_err(|_| GuestError::Engine)?;
        if grants.log {
            log::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
                .map_err(|_| GuestError::Engine)?;
        }
        let bindings =
            Plugin::instantiate(&mut store, &component.component, &linker).map_err(|error| {
                match error.downcast_ref::<wasmtime::Trap>() {
                    Some(_) => classify(&error),
                    None => GuestError::Link,
                }
            })?;
        Ok(Self {
            store,
            bindings,
            fuel: limits.fuel_per_call,
            output_bytes: limits.transfer_bytes,
            poisoned: false,
            cancellation,
        })
    }

    /// Evaluates a pure node: `definition` with typed inputs, each output checked
    /// against `outputs`. The outer error is a host/guest fault; the inner one a
    /// guest-reported node error.
    pub fn evaluate(
        &mut self,
        types: &TypeRegistry,
        definition: &str,
        inputs: &[(TypeExpr, Value)],
        outputs: &[TypeExpr],
    ) -> Result<Result<Vec<Value>, GuestNodeError>, GuestError> {
        self.call(Export::Evaluate, types, definition, inputs, outputs)
    }

    /// Runs an action or async node once. Same value and error contract as
    /// `evaluate`, but the guest may keep state between calls on this instance.
    pub fn execute(
        &mut self,
        types: &TypeRegistry,
        definition: &str,
        inputs: &[(TypeExpr, Value)],
        outputs: &[TypeExpr],
    ) -> Result<Result<Vec<Value>, GuestNodeError>, GuestError> {
        self.call(Export::Execute, types, definition, inputs, outputs)
    }

    fn call(
        &mut self,
        export: Export,
        types: &TypeRegistry,
        definition: &str,
        inputs: &[(TypeExpr, Value)],
        outputs: &[TypeExpr],
    ) -> Result<Result<Vec<Value>, GuestNodeError>, GuestError> {
        if self.poisoned {
            return Err(GuestError::Poisoned);
        }
        if self.cancellation.is_cancelled() {
            self.poisoned = true;
            return Err(GuestError::Trap(GuestTrap::Interrupted));
        }
        let mut encoded = Vec::with_capacity(inputs.len());
        for (ty, value) in inputs {
            encoded.push(to_wire(ty, value, types)?);
        }
        self.store
            .set_fuel(self.fuel)
            .map_err(|_| GuestError::Engine)?;
        if let Err(error) = self.cancellation.arm(&mut self.store) {
            self.poisoned = true;
            return Err(error);
        }
        let called = match export {
            Export::Evaluate => self.bindings.orange_expansion_nodes().call_evaluate(
                &mut self.store,
                definition,
                &encoded,
            ),
            Export::Execute => self.bindings.orange_expansion_effects().call_execute(
                &mut self.store,
                definition,
                &encoded,
            ),
        };
        let result = called.map_err(|error| {
            self.poisoned = true;
            classify(&error)
        })?;
        match result {
            Err(error) => {
                if !namespaced(&error.code) || !namespaced(&error.message_key) {
                    return Err(GuestError::OutputContract);
                }
                Ok(Err(GuestNodeError {
                    code: error.code,
                    message_key: error.message_key,
                }))
            }
            Ok(values) => {
                if values.len() != outputs.len() {
                    return Err(GuestError::OutputContract);
                }
                let mut decoded = Vec::with_capacity(values.len());
                let mut remaining = self.output_bytes;
                for (wire, ty) in values.into_iter().zip(outputs) {
                    let value = from_wire(wire, ty, types, remaining)?;
                    let charge = orange_model::value::shape::ValueShape::of_in(ty, types)
                        .ok_or(GuestError::OutputContract)?
                        .checked_charge(&value, remaining)
                        .map_err(|_| GuestError::OutputContract)?;
                    remaining -= charge;
                    decoded.push(value);
                }
                Ok(Ok(decoded))
            }
        }
    }

    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }
    /// Clone a cancellation handle scoped to this plugin instance's lifetime.
    pub fn cancellation(&self) -> GuestCancellation {
        self.cancellation.clone()
    }

    /// Takes the retained log entries (granted `log` only).
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.store.data_mut().log)
    }
}
