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
    path: "../../../../sdk/wit",
    world: "plugin",
});

use crate::access::{GuestAccess, HttpRequest};
use orange::expansion::types::Value as Wire;
use orange::expansion::{files, host, http, log, settings, websocket};
use orange_model::value::ListRepr;
use std::sync::Arc;

/// The host API version offered through `host.api-version` (04 SDK surface).
pub const API_VERSION: (u16, u16) = (1, 1);
/// Largest number of retained log entries per instance; later entries are dropped.
pub const MAX_LOG_ENTRIES: usize = 64;
/// Longest retained log entry in bytes; longer entries are cut at a character boundary.
pub const MAX_LOG_BYTES: usize = 1024;

/// Host capabilities granted to one instance. `host` is always linked; every other
/// import is linked only when granted, so an ungranted import fails instantiation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Grants {
    pub log: bool,
    /// Read and write files in the package's own data directory.
    pub files: bool,
    /// Send HTTPS requests through the application's network client.
    pub http: bool,
    /// Send HTTP or HTTPS requests to this computer and the local network
    /// ([`is_local`](crate::access::is_local)) through the application's network client.
    pub http_local: bool,
    /// Heavy compute: each call gets [`HEAVY_COMPUTE_FACTOR`] times the engine's fuel
    /// and transfer limits. Memory limits stay those of the engine.
    pub compute: bool,
    /// Send messages on the WebSocket connection the node's settings name, through the
    /// host's own session (1.1).
    pub websocket: bool,
}

/// How many times the per-call fuel and transfer limits a `compute` grant gives.
pub const HEAVY_COMPUTE_FACTOR: u32 = 16;

impl Grants {
    /// Whether the `http` import is linked: either network grant.
    pub const fn links_http(self) -> bool {
        self.http || self.http_local
    }

    /// Each grant with its manifest `permissions` name, in a fixed order.
    pub const fn named(self) -> [(&'static str, bool); 6] {
        [
            ("log", self.log),
            ("files", self.files),
            ("http", self.http),
            ("http-local", self.http_local),
            ("compute", self.compute),
            ("websocket", self.websocket),
        ]
    }

    /// Gives the grant with this manifest name; `false` when no grant has it.
    pub fn grant(&mut self, name: &str) -> bool {
        let field = match name {
            "log" => &mut self.log,
            "files" => &mut self.files,
            "http" => &mut self.http,
            "http-local" => &mut self.http_local,
            "compute" => &mut self.compute,
            "websocket" => &mut self.websocket,
            _ => return false,
        };
        *field = true;
        true
    }

    /// Every grant that is in both.
    #[must_use]
    pub const fn within(self, allowed: Self) -> Self {
        Self {
            log: self.log && allowed.log,
            files: self.files && allowed.files,
            http: self.http && allowed.http,
            http_local: self.http_local && allowed.http_local,
            compute: self.compute && allowed.compute,
            websocket: self.websocket && allowed.websocket,
        }
    }
}

struct PluginState {
    limits: InstanceLimits,
    log: Vec<String>,
    access: GuestAccess,
    grants: Grants,
    /// The JSON text of the node's settings, for the `settings` import.
    settings: String,
    /// The host session of the connection the node names, during one effect call.
    session: Option<Arc<dyn orange_sdk::Session>>,
    effects_allowed: bool,
    /// Largest content one import call hands to the guest.
    transfer_bytes: usize,
}

impl files::Host for PluginState {
    fn read(&mut self, path: String) -> Result<Vec<u8>, String> {
        self.require_effect()?;
        self.access
            .read(&path, self.transfer_bytes)
            .map_err(str::to_owned)
    }
    fn write(&mut self, path: String, data: Vec<u8>) -> Result<(), String> {
        self.require_effect()?;
        self.access.write(&path, &data).map_err(str::to_owned)
    }
    fn list(&mut self, path: String) -> Result<Vec<String>, String> {
        self.require_effect()?;
        self.access.list(&path).map_err(str::to_owned)
    }
    fn remove(&mut self, path: String) -> Result<(), String> {
        self.require_effect()?;
        self.access.remove(&path).map_err(str::to_owned)
    }
}

impl http::Host for PluginState {
    fn send(&mut self, request: http::Request) -> Result<http::Response, String> {
        self.require_effect()?;
        let request = HttpRequest {
            method: request.method,
            url: request.url,
            headers: request.headers,
            body: request.body,
            max_response_bytes: self.transfer_bytes,
        };
        self.access
            .send(&request, self.grants.http, self.grants.http_local)
            .map(|response| http::Response {
                status: response.status,
                headers: response.headers,
                body: response.body,
            })
            .map_err(str::to_owned)
    }
}

impl settings::Host for PluginState {
    fn get(&mut self) -> String {
        self.settings.clone()
    }
}

impl websocket::Host for PluginState {
    fn send_text(&mut self, text: String) -> Result<(), String> {
        self.send(orange_sdk::Message::Text(text.into()))
    }
    fn send_binary(&mut self, data: Vec<u8>) -> Result<(), String> {
        self.send(orange_sdk::Message::Binary(data.into()))
    }
}

impl PluginState {
    /// Queues a message on the node's connection, through the host's session.
    fn send(&mut self, message: orange_sdk::Message) -> Result<(), String> {
        self.require_effect()?;
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| crate::access::UNAVAILABLE.to_owned())?;
        session.send_message(message).map_err(|error| {
            match error {
                orange_sdk::MessageError::Full => crate::access::QUOTA,
                orange_sdk::MessageError::TooLarge => crate::access::TOO_LARGE,
                _ => crate::access::UNAVAILABLE,
            }
            .to_owned()
        })
    }

    fn require_effect(&self) -> Result<(), String> {
        if self.effects_allowed {
            Ok(())
        } else {
            Err(crate::access::NOT_ALLOWED.to_owned())
        }
    }
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

/// The optional export of a package that contributes trigger nodes (1.1).
pub const EVENTS_INTERFACE: &str = "orange:expansion/events@1.1.0";

/// `events.receive`: definition, text or binary message, and the trigger's outputs.
type ReceiveFunc = wasmtime::component::TypedFunc<
    (String, Option<String>, Option<Vec<u8>>),
    (Result<Option<Vec<Wire>>, orange::expansion::types::NodeError>,),
>;

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
    /// The instance, for the optional `events` export the world does not require.
    instance: wasmtime::component::Instance,
    fuel: u64,
    transfer_bytes: usize,
    input_bytes: usize,
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
    /// Instantiates `component` as an `orange:expansion/plugin` with exactly `grants`
    /// and nothing to reach: a granted `files` or `http` import answers `unavailable`.
    pub fn new(component: &GuestComponent, grants: Grants) -> Result<Self, GuestError> {
        Self::with_access(component, grants, GuestAccess::default())
    }

    /// Like [`new`](Self::new), with the data directory and network client its granted
    /// `files` and `http` imports use.
    pub fn with_access(
        component: &GuestComponent,
        grants: Grants,
        access: GuestAccess,
    ) -> Result<Self, GuestError> {
        let engine = &component.engine;
        let mut limits = engine.limits;
        if grants.compute {
            let factor = HEAVY_COMPUTE_FACTOR as usize;
            limits.fuel_per_call = limits
                .fuel_per_call
                .saturating_mul(u64::from(HEAVY_COMPUTE_FACTOR));
            limits.transfer_bytes = limits.transfer_bytes.saturating_mul(factor);
            limits.input_bytes = limits.input_bytes.saturating_mul(factor);
        }
        let mut store = Store::new(
            &engine.engine,
            PluginState {
                limits: InstanceLimits::new(limits, &engine.aggregate)?,
                log: Vec::new(),
                access,
                grants,
                settings: "{}".to_owned(),
                session: None,
                effects_allowed: false,
                transfer_bytes: limits.transfer_bytes,
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
        settings::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .map_err(|_| GuestError::Engine)?;
        if grants.websocket {
            websocket::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
                .map_err(|_| GuestError::Engine)?;
        }
        if grants.log {
            log::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
                .map_err(|_| GuestError::Engine)?;
        }
        if grants.files {
            files::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
                .map_err(|_| GuestError::Engine)?;
        }
        if grants.links_http() {
            http::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
                .map_err(|_| GuestError::Engine)?;
        }
        let link = |error: wasmtime::Error| match error.downcast_ref::<wasmtime::Trap>() {
            Some(_) => classify(&error),
            None => GuestError::Link,
        };
        let instance = linker
            .instantiate(&mut store, &component.component)
            .map_err(link)?;
        let bindings = Plugin::new(&mut store, &instance).map_err(link)?;
        Ok(Self {
            store,
            bindings,
            instance,
            fuel: limits.fuel_per_call,
            transfer_bytes: limits.transfer_bytes,
            input_bytes: limits.input_bytes,
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
    /// Gives the instance its node's settings, as the JSON text of its saved `config`.
    pub fn set_settings(&mut self, settings: String) {
        self.store.data_mut().settings = settings;
    }

    /// The host session the next effect calls may send on; `None` takes it away.
    pub fn set_session(&mut self, session: Option<Arc<dyn orange_sdk::Session>>) {
        self.store.data_mut().session = session;
    }

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
        let mut encoded = Vec::new();
        let mut remaining = self.input_bytes;
        for (ty, value) in inputs {
            let charge = orange_model::value::shape::ValueShape::of_in(ty, types)
                .ok_or(GuestError::InputContract)?
                .checked_charge(value, remaining)
                .map_err(|_| GuestError::InputContract)?;
            let wire = to_wire(ty, value, types)?;
            let charge = match &wire {
                Wire::Literal(text) => charge.max(text.len()),
                _ => charge,
            };
            remaining = remaining
                .checked_sub(charge)
                .ok_or(GuestError::InputContract)?;
            encoded.push(wire);
        }
        self.store
            .set_fuel(self.fuel)
            .map_err(|_| GuestError::Engine)?;
        if let Err(error) = self.cancellation.arm(&mut self.store) {
            self.poisoned = true;
            return Err(error);
        }
        self.store.data_mut().effects_allowed = matches!(export, Export::Execute);
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
        self.store.data_mut().effects_allowed = false;
        let result = called.map_err(|error| {
            self.poisoned = true;
            classify(&error)
        })?;
        match result {
            Err(error) => node_error(error).map(Err),
            Ok(values) => self.decode(values, types, outputs).map(Ok),
        }
    }

    /// Decodes a call's outputs, each checked against `outputs` and all of them together
    /// within the instance's transfer limit.
    fn decode(
        &self,
        values: Vec<Wire>,
        types: &TypeRegistry,
        outputs: &[TypeExpr],
    ) -> Result<Vec<Value>, GuestError> {
        if values.len() != outputs.len() {
            return Err(GuestError::OutputContract);
        }
        let mut decoded = Vec::with_capacity(values.len());
        let mut remaining = self.transfer_bytes;
        for (wire, ty) in values.into_iter().zip(outputs) {
            let value = from_wire(wire, ty, types, remaining)?;
            let charge = orange_model::value::shape::ValueShape::of_in(ty, types)
                .ok_or(GuestError::OutputContract)?
                .checked_charge(&value, remaining)
                .map_err(|_| GuestError::OutputContract)?;
            remaining -= charge;
            decoded.push(value);
        }
        Ok(decoded)
    }

    /// The `receive` function of the optional `events` export, when the component has it.
    fn receive_func(&mut self) -> Option<ReceiveFunc> {
        let interface = self
            .instance
            .get_export_index(&mut self.store, None, EVENTS_INTERFACE)?;
        let func = self
            .instance
            .get_export_index(&mut self.store, Some(&interface), "receive")?;
        self.instance.get_typed_func(&mut self.store, func).ok()
    }

    /// Whether the component exports `events`, so its trigger nodes can start.
    pub fn receives_events(&mut self) -> bool {
        self.receive_func().is_some()
    }

    /// Hands one incoming message to trigger `definition` and returns its outputs, each
    /// checked against `outputs`: `None` when the guest lets the message pass. The call
    /// has the instance's fuel and transfer limits and no effects: a message over the
    /// input limit is refused before the guest sees it.
    pub fn receive(
        &mut self,
        types: &TypeRegistry,
        definition: &str,
        message: &orange_sdk::Message,
        outputs: &[TypeExpr],
    ) -> Result<Result<Option<Vec<Value>>, GuestNodeError>, GuestError> {
        if self.poisoned {
            return Err(GuestError::Poisoned);
        }
        if self.cancellation.is_cancelled() {
            self.poisoned = true;
            return Err(GuestError::Trap(GuestTrap::Interrupted));
        }
        let (text, binary) = match message {
            orange_sdk::Message::Text(text) => (Some(text.to_string()), None),
            orange_sdk::Message::Binary(data) => (None, Some(data.to_vec())),
        };
        let size = text.as_ref().map_or(0, String::len) + binary.as_ref().map_or(0, Vec::len);
        if size.saturating_add(definition.len()) > self.input_bytes {
            return Err(GuestError::InputContract);
        }
        let func = self.receive_func().ok_or(GuestError::Export)?;
        self.store
            .set_fuel(self.fuel)
            .map_err(|_| GuestError::Engine)?;
        if let Err(error) = self.cancellation.arm(&mut self.store) {
            self.poisoned = true;
            return Err(error);
        }
        self.store.data_mut().effects_allowed = false;
        let (result,) = func
            .call(&mut self.store, (definition.to_owned(), text, binary))
            .map_err(|error| {
                self.poisoned = true;
                classify(&error)
            })?;
        match result {
            Err(error) => node_error(error).map(Err),
            Ok(None) => Ok(Ok(None)),
            Ok(Some(values)) => self.decode(values, types, outputs).map(|v| Ok(Some(v))),
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

/// A node error the guest returned, when its code and message key are namespaced.
fn node_error(error: orange::expansion::types::NodeError) -> Result<GuestNodeError, GuestError> {
    if !namespaced(&error.code) || !namespaced(&error.message_key) {
        return Err(GuestError::OutputContract);
    }
    Ok(GuestNodeError {
        code: error.code,
        message_key: error.message_key,
    })
}
