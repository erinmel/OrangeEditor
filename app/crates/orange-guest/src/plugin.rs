//! The `orange:expansion/plugin@1.0.0` world (sdk/wit): grant-scoped host imports and
//! node evaluation with values in the `orange.value.v1` literal codec (K02b).

use orange_model::Value;
use orange_model::document::literal::Literal;
use orange_model::id::TypeExpr;
use orange_model::types::TypeRegistry;
use orange_model::value::literal_codec;
use wasmtime::component::{HasSelf, Linker};
use wasmtime::{Store, StoreLimitsBuilder};

use crate::{GuestComponent, GuestError, classify};

wasmtime::component::bindgen!({
    path: "../../../sdk/wit",
    world: "plugin",
});

use orange::expansion::{host, log};

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
    limits: wasmtime::StoreLimits,
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
    poisoned: bool,
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

impl GuestPlugin {
    /// Instantiates `component` as an `orange:expansion/plugin` with exactly `grants`.
    pub fn new(component: &GuestComponent, grants: Grants) -> Result<Self, GuestError> {
        let engine = &component.engine;
        let limits = engine.limits;
        let mut store = Store::new(
            &engine.engine,
            PluginState {
                limits: StoreLimitsBuilder::new()
                    .memory_size(limits.memory_bytes)
                    .instances(limits.instances)
                    .trap_on_grow_failure(false)
                    .build(),
                log: Vec::new(),
            },
        );
        store.limiter(|state| &mut state.limits);
        store.epoch_deadline_trap();
        store.set_epoch_deadline(1);
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
            poisoned: false,
        })
    }

    /// Evaluates `definition` with typed inputs and checks each output against
    /// `outputs`. Values cross as `orange.value.v1` literals resolved through `types`.
    /// The outer error is a host/guest fault; the inner one a guest-reported node error.
    pub fn evaluate(
        &mut self,
        types: &TypeRegistry,
        definition: &str,
        inputs: &[(TypeExpr, Value)],
        outputs: &[TypeExpr],
    ) -> Result<Result<Vec<Value>, GuestNodeError>, GuestError> {
        if self.poisoned {
            return Err(GuestError::Poisoned);
        }
        let mut encoded = Vec::with_capacity(inputs.len());
        for (ty, value) in inputs {
            let literal =
                literal_codec::encode(ty, value, types).map_err(|_| GuestError::InputContract)?;
            encoded.push(serde_json::to_string(&literal).map_err(|_| GuestError::InputContract)?);
        }
        self.store
            .set_fuel(self.fuel)
            .map_err(|_| GuestError::Engine)?;
        self.store.set_epoch_deadline(1);
        let result = self
            .bindings
            .orange_expansion_nodes()
            .call_evaluate(&mut self.store, definition, &encoded)
            .map_err(|error| {
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
                for (text, ty) in values.iter().zip(outputs) {
                    let literal: Literal =
                        serde_json::from_str(text).map_err(|_| GuestError::OutputContract)?;
                    if &literal.ty != ty {
                        return Err(GuestError::OutputContract);
                    }
                    decoded.push(
                        literal_codec::decode(&literal, types)
                            .map_err(|_| GuestError::OutputContract)?,
                    );
                }
                Ok(Ok(decoded))
            }
        }
    }

    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    /// Takes the retained log entries (granted `log` only).
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.store.data_mut().log)
    }
}
