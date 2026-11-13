//! The OrangeEditor plugin SDK: what a Rust crate implements to add nodes, triggers and
//! connection kinds to a host application.
//!
//! A plugin builds a [`ServicePackage`]: definition files for its nodes, a
//! [`ServiceNode`] or [`ServiceSource`] behind each, and a [`ConnectionAdapter`] per
//! connection kind. The application passes the package to its host. This crate has no
//! package store, compiler front end or WebAssembly engine, so a plugin links only the
//! graph model, the value types and the run's effect lease.
//!
//! This is the trusted tier for work that needs the operating system: sockets, files,
//! devices. Nodes are asynchronous: a run issues a request and a worker thread performs
//! it, so no plugin code runs on a flow's own thread.
//!
//! A connection is declared in a document or shared through the host. An adapter turns
//! its settings into a [`Session`]; nodes that declare they use a connection receive the
//! session the node's `connection` config field names.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

pub use orange_kernel::{FaultText, KernelFault};
pub use orange_model::Value;
pub use orange_model::catalog::{DefinitionError, NodeDefinition, Presentation, TextRef};
pub use orange_model::document::DefinitionRef;
pub use orange_model::value::{ListRepr, RecordRepr};
pub use orange_runtime::EffectLease;
use serde_json::{Map, Value as Json};
use std::any::Any;
use std::sync::Arc;

/// Prefix of a credential reference in connection settings: `credential://<name>`.
pub const SECRET_PREFIX: &str = "credential://";

/// Whether `name` can name a secret: 1 to 128 bytes of `a-z`, `0-9`, `.`, `_`, `-`
/// and `/`, starting with a letter or digit.
pub fn is_secret_name(name: &str) -> bool {
    (1..=128).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-' | b'/')
        })
}

/// Why a secret was not read or written. No variant carries secret text.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SecretError {
    /// No secret is stored under the name.
    NotFound,
    /// The name is not one [`is_secret_name`] accepts.
    InvalidName,
    /// The secret is empty or too long for the store.
    InvalidSecret,
    /// The store is locked, missing or failed; the text says which, without details.
    Unavailable(&'static str),
}
impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("no credential is stored under that name"),
            Self::InvalidName => f.write_str("the credential name is not valid"),
            Self::InvalidSecret => f.write_str("the credential is empty or too long"),
            Self::Unavailable(why) => write!(f, "the credential store is not available: {why}"),
        }
    }
}
impl std::error::Error for SecretError {}

/// Where an application keeps credentials. The host reads from it when a connection's
/// settings name a secret as `credential://<name>`; documents, packages and logs hold
/// only that reference.
pub trait SecretStore: std::fmt::Debug + Send + Sync + 'static {
    /// The secret stored under `name`.
    fn get(&self, name: &str) -> Result<String, SecretError>;
    /// Stores a new value for a secret a connection was opened with, because the
    /// service replaced it (a rotated refresh token). A store that cannot be written
    /// refuses, which is the default; the connection then works until the application
    /// stops and needs a new sign-in afterwards.
    fn replace(&self, name: &str, secret: &str) -> Result<(), SecretError> {
        let _ = (name, secret);
        Err(SecretError::Unavailable("it cannot be written"))
    }
}

/// How a session gives back a secret its service replaced. The host makes one for each
/// connection it opens; the adapter names the setting the secret came in (a JSON
/// pointer into the settings, such as `/oauth/refresh_token`) and the host stores the
/// new value under the credential that setting referred to. A setting that held a
/// plain value, not a reference, is not stored anywhere.
#[derive(Clone, Default)]
pub struct Renew(Option<Arc<Keep>>);
type Keep = dyn Fn(&str, &str) + Send + Sync;
impl Renew {
    /// A renewal that stores nothing.
    pub fn none() -> Self {
        Self(None)
    }
    pub fn new(keep: impl Fn(&str, &str) + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(keep)))
    }
    /// Reports that the secret given in `setting` is now `secret`.
    pub fn keep(&self, setting: &str, secret: &str) {
        if let Some(keep) = &self.0 {
            keep(setting, secret);
        }
    }
}
impl std::fmt::Debug for Renew {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "Renew(host)"
        } else {
            "Renew(none)"
        })
    }
}

/// Provider ID of every node a service package implements.
pub const SERVICE_PROVIDER: &str = "native";

/// A live connection. The adapter that opened it defines what it holds; its nodes get
/// it back through [`Session::as_any`]. Dropping the last reference closes it.
pub trait Session: std::fmt::Debug + Send + Sync + 'static {
    fn as_any(&self) -> &dyn Any;
    /// Called each time a flow that uses the session has started all of its triggers,
    /// so more than once for a session several flows share. A session that receives
    /// should not begin before the first call: what arrives earlier has no trigger to
    /// reach and would be lost. The default does nothing.
    fn start(&self) {}
}

/// Opens sessions of one adapter kind from declared settings.
///
/// `open` runs on the thread that opens a document and must not block on the network:
/// an adapter that needs a handshake performs it on first use, from a worker.
pub trait ConnectionAdapter: std::fmt::Debug + Send + Sync + 'static {
    fn open(&self, settings: &Map<String, Json>) -> Result<Arc<dyn Session>, String>;
    /// [`open`](Self::open) for an adapter whose service can replace a secret it was
    /// given: it keeps `renew` and reports each replacement. The host calls this one;
    /// the default ignores `renew`.
    fn open_renewing(
        &self,
        settings: &Map<String, Json>,
        renew: Renew,
    ) -> Result<Arc<dyn Session>, String> {
        let _ = renew;
        self.open(settings)
    }
}

/// One execution of a service node, on a worker thread.
pub struct Call<'a> {
    /// The node's data inputs in declared order.
    pub inputs: &'a [Value],
    /// The session of the connection the node names, when its definition uses one.
    pub session: Option<&'a dyn Session>,
    /// Cancelled when the run no longer wants the result; long work should poll it.
    pub lease: &'a EffectLease,
}
impl std::fmt::Debug for Call<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Call")
            .field("inputs", &self.inputs.len())
            .field("session", &self.session.is_some())
            .finish_non_exhaustive()
    }
}

/// The implementation of one asynchronous service node. It returns the node's data
/// outputs in declared order, or a fault that takes the node's `failed` route.
pub trait ServiceNode: Send + Sync + 'static {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault>;
    /// For a node made with [`ServicePackage::stream`]: opens its stream. An ordinary
    /// node has none.
    fn items(&self, call: &Call<'_>) -> Option<Result<Box<dyn Items>, KernelFault>> {
        let _ = call;
        None
    }
}

/// Why an [`Emitter`] did not deliver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitError {
    /// The flow's mailbox is full. The source decides whether to retry or drop.
    Busy,
    /// The flow has ended; the source should stop.
    Closed,
}

/// Fires one entry node of one flow. Cloneable and usable from any thread; firing never
/// blocks. What a paused or busy flow does with a firing is the entry's own policy.
#[derive(Clone)]
pub struct Emitter(Arc<Fire>);
type Fire = dyn Fn(Option<Value>) -> Result<(), EmitError> + Send + Sync;
impl std::fmt::Debug for Emitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Emitter").finish_non_exhaustive()
    }
}
impl Emitter {
    /// Wraps the host's way of firing one entry node. Hosts call this; a plugin only
    /// receives emitters.
    pub fn new(
        fire: impl Fn(Option<Value>) -> Result<(), EmitError> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(fire))
    }
    /// Fires the entry with its payload, or `None` for an entry that declares none.
    pub fn emit(&self, payload: Option<Value>) -> Result<(), EmitError> {
        (self.0)(payload)
    }
}

/// The entry nodes listening to one source, such as a connection's incoming messages.
/// A session keeps one of these; each entry that starts on the session is added with
/// [`Listeners::listen`], and the source fires them with [`Listeners::emit`]. `T` is
/// whatever the source needs to tell its listeners apart, such as text or binary.
pub struct Listeners<T = ()> {
    entries: std::sync::Mutex<Vec<(u64, T, Emitter)>>,
    next: std::sync::atomic::AtomicU64,
}
impl<T> Default for Listeners<T> {
    fn default() -> Self {
        Self {
            entries: std::sync::Mutex::new(Vec::new()),
            next: std::sync::atomic::AtomicU64::new(0),
        }
    }
}
impl<T> std::fmt::Debug for Listeners<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listeners").finish_non_exhaustive()
    }
}
impl<T: Send + 'static> Listeners<T> {
    pub fn new() -> Self {
        Self::default()
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, Vec<(u64, T, Emitter)>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Adds a listener and returns the guard a [`ServiceSource`] hands back: dropping
    /// it removes the listener. `owner` is what holds these listeners, usually the
    /// session's shared state, and `of` finds them in it.
    pub fn listen<S: Send + Sync + 'static>(
        owner: Arc<S>,
        of: fn(&S) -> &Self,
        tag: T,
        emitter: Emitter,
    ) -> Box<dyn Send> {
        struct Guard<S, T: Send + 'static>(Arc<S>, fn(&S) -> &Listeners<T>, u64);
        impl<S, T: Send + 'static> Drop for Guard<S, T> {
            fn drop(&mut self) {
                (self.1)(&self.0).entries().retain(|(id, ..)| *id != self.2);
            }
        }
        let listeners = of(&owner);
        let id = listeners
            .next
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        listeners.entries().push((id, tag, emitter));
        Box::new(Guard(owner, of, id))
    }

    /// Fires every listener for which `payload` gives a value. A flow that has ended
    /// is forgotten; one whose mailbox is full misses this firing, as its bounded
    /// mailbox requires.
    pub fn emit(&self, payload: impl Fn(&T) -> Option<Value>) {
        self.entries().retain(|(_, tag, emitter)| {
            payload(tag).is_none_or(|value| emitter.emit(Some(value)) != Err(EmitError::Closed))
        });
    }

    /// The emitters now listening, for a source that must not lose a firing and so
    /// handles a full mailbox itself.
    pub fn emitters(&self) -> Vec<Emitter> {
        self.entries()
            .iter()
            .map(|(_, _, emitter)| emitter.clone())
            .collect()
    }
}

/// The items of a stream a node produces, pulled one at a time. `next` runs on a
/// worker thread and may block; it is called again only after the graph has taken the
/// previous item, so a slow graph slows the producer instead of filling memory. `None`
/// ends the stream; an error ends it with that fault. The value is dropped when the
/// graph closes the stream or stops. Any iterator of item results is one.
pub trait Items: Send + 'static {
    fn next(&mut self) -> Option<Result<Value, KernelFault>>;
}
impl<I> Items for I
where
    I: Iterator<Item = Result<Value, KernelFault>> + Send + 'static,
{
    fn next(&mut self) -> Option<Result<Value, KernelFault>> {
        Iterator::next(self)
    }
}

/// The implementation of a node whose one data output is a stream
/// (`core/stream@1<T>`). `open` runs on a worker thread like [`ServiceNode::run`] and
/// returns the items; the node continues as soon as the stream is open.
pub trait ServiceStream: std::fmt::Debug + Send + Sync + 'static {
    fn open(&self, call: &Call<'_>) -> Result<Box<dyn Items>, KernelFault>;
}

/// A stream node in the list of a package's nodes.
#[derive(Debug)]
struct Streaming(Arc<dyn ServiceStream>);
impl ServiceNode for Streaming {
    fn run(&self, _call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        Err(KernelFault {
            code: FaultText::Static("org.orange.sdk/Stream"),
            message_key: FaultText::Static("org.orange.sdk/stream-node"),
            message: "The node produces a stream and the application did not open it.",
        })
    }
    fn items(&self, call: &Call<'_>) -> Option<Result<Box<dyn Items>, KernelFault>> {
        Some(self.0.open(call))
    }
}

/// The implementation of one entry node: something outside the graph that fires it.
///
/// `start` is called once per entry node when its flow opens and must not block. It
/// returns a guard; dropping the guard stops the source for that node.
pub trait ServiceSource: Send + Sync + 'static {
    fn start(
        &self,
        session: Option<Arc<dyn Session>>,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String>;
}

/// Builds the definition file text of a service node, so a package states what is
/// particular to each node and nothing else. [`ServicePackage::node`] and
/// [`ServicePackage::source`] take the result of [`build`](Self::build).
///
/// Every definition made here is version 1, takes its label, description and category
/// keys from its ID, and is marked as reaching outside the graph. A node is
/// asynchronous; an entry has a `then` outcome and at most one `payload` output.
#[derive(Debug, Clone)]
pub struct Definition {
    package: String,
    json: Map<String, Json>,
    ports: Vec<Json>,
}
impl Definition {
    fn new(package: &str, name: &str, label: &str, entry: bool) -> Self {
        let id = format!("{package}/{name}");
        let mut json = Map::new();
        let mut set = |key: &str, value: Json| {
            let _ = json.insert(key.to_owned(), value);
        };
        set(
            "label",
            serde_json::json!({"key": format!("{id}-label"), "fallback": label}),
        );
        set("definition_id", Json::String(id));
        set("definition_version", 1.into());
        set("config_version", 1.into());
        set("determinism", "external".into());
        if entry {
            set("kind", "entry".into());
        } else {
            set("kind", "async".into());
            set("suspends", "always".into());
        }
        Self {
            package: package.to_owned(),
            json,
            ports: Vec::new(),
        }
    }
    /// An asynchronous node `<package>/<name>` shown as `label`.
    pub fn node(package: &str, name: &str, label: &str) -> Self {
        Self::new(package, name, label, false)
    }
    /// An entry node `<package>/<name>` shown as `label`.
    pub fn entry(package: &str, name: &str, label: &str) -> Self {
        Self::new(package, name, label, true)
    }
    /// One sentence on what the node does.
    #[must_use]
    pub fn description(mut self, text: &str) -> Self {
        let key = format!(
            "{}-description",
            self.json["definition_id"].as_str().unwrap_or_default()
        );
        let _ = self.json.insert(
            "description".to_owned(),
            serde_json::json!({"key": key, "fallback": text}),
        );
        self
    }
    /// The palette category: `slug` names it within the package and `label` shows it.
    #[must_use]
    pub fn category(mut self, slug: &str, label: &str) -> Self {
        let id = format!("{}/category-{slug}", self.package);
        let _ = self.json.insert(
            "editor".to_owned(),
            serde_json::json!({"category_id": id, "category": {"key": id, "fallback": label}}),
        );
        self
    }
    /// The node uses the connection its `connection` config field names.
    #[must_use]
    pub fn connection(mut self) -> Self {
        let _ = self.json.insert("connection".to_owned(), true.into());
        self
    }
    /// A data input of type `ty`, such as `core/string@1`. Inputs reach
    /// [`Call::inputs`] in the order they are added.
    #[must_use]
    pub fn input(mut self, name: &str, ty: &str) -> Self {
        self.ports
            .push(serde_json::json!({"port_id": name, "direction": "input",
            "kind": "data", "type": ty}));
        self
    }
    /// A data output of type `ty`, available when the node continues. Outputs are
    /// returned in the order they are added.
    #[must_use]
    pub fn output(mut self, name: &str, ty: &str) -> Self {
        self.ports
            .push(serde_json::json!({"port_id": name, "direction": "output",
            "kind": "data", "type": ty, "source": "execution", "available_on": ["then"]}));
        self
    }
    /// The `payload` output of an entry: the value each firing carries.
    #[must_use]
    pub fn payload(mut self, ty: &str) -> Self {
        self.ports.push(
            serde_json::json!({"port_id": "payload", "direction": "output",
            "kind": "data", "type": ty, "source": "payload"}),
        );
        self
    }
    /// The definition file text.
    pub fn build(mut self) -> String {
        let _ = self
            .json
            .insert("ports".to_owned(), Json::Array(self.ports));
        Json::Object(self.json).to_string()
    }
}

/// A set of native nodes and connection adapters built into the application.
pub struct ServicePackage {
    id: String,
    version: String,
    name: Option<String>,
    description: Option<String>,
    update_feed: Option<String>,
    update_key: Option<String>,
    nodes: Vec<(NodeDefinition, Arc<dyn ServiceNode>)>,
    sources: Vec<(NodeDefinition, Arc<dyn ServiceSource>)>,
    adapters: Vec<(String, Arc<dyn ConnectionAdapter>)>,
    presentations: Vec<(DefinitionRef, Presentation)>,
    types: Vec<TypeDecl>,
}

/// A type a package adds: a record with named fields or an enum with variants. Its
/// full name is `<package id>/<name>@1`; field and payload types are type expressions
/// such as `core/f64@1` or another type of the package. Values are
/// [`Value::Record`] with the fields in declared order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeDecl {
    Record {
        name: String,
        /// Each field's ID, its type and whether it may be absent.
        fields: Vec<(String, String, bool)>,
    },
    Enum {
        name: String,
        /// Each variant's ID and the type of its payload, if it carries one.
        variants: Vec<(String, Option<String>)>,
    },
}
impl std::fmt::Debug for ServicePackage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServicePackage")
            .field("id", &self.id)
            .field("version", &self.version)
            .field("nodes", &self.nodes.len())
            .field("adapters", &self.adapters.len())
            .finish()
    }
}
impl ServicePackage {
    /// `id` is the package namespace its definitions and adapter kinds live in.
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            name: None,
            description: None,
            update_feed: None,
            update_key: None,
            nodes: Vec::new(),
            sources: Vec::new(),
            adapters: Vec::new(),
            presentations: Vec::new(),
            types: Vec::new(),
        }
    }
    /// Adds a record type `<package id>/<name>@1` with required fields, each an ID and
    /// a type. The host checks the declaration when it starts and refuses to start
    /// with one that is not valid.
    pub fn record(mut self, name: &str, fields: &[(&str, &str)]) -> Self {
        self.types.push(TypeDecl::Record {
            name: name.to_owned(),
            fields: fields
                .iter()
                .map(|(id, ty)| ((*id).to_owned(), (*ty).to_owned(), false))
                .collect(),
        });
        self
    }
    /// Adds an enum type `<package id>/<name>@1`; a variant is an ID and the type of
    /// its payload, if it carries one.
    pub fn enumeration(mut self, name: &str, variants: &[(&str, Option<&str>)]) -> Self {
        self.types.push(TypeDecl::Enum {
            name: name.to_owned(),
            variants: variants
                .iter()
                .map(|(id, payload)| ((*id).to_owned(), payload.map(str::to_owned)))
                .collect(),
        });
        self
    }
    /// Adds a type declared in full, such as a record with optional fields.
    pub fn declare(mut self, declaration: TypeDecl) -> Self {
        self.types.push(declaration);
        self
    }
    /// The types this package adds.
    pub fn types(&self) -> &[TypeDecl] {
        &self.types
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn version(&self) -> &str {
        &self.version
    }
    /// Sets what a package list shows for this package: a display name and one
    /// paragraph on what it adds.
    pub fn about(mut self, name: impl Into<String>, description: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self.description = Some(description.into());
        self
    }
    /// Names where newer versions of this package are announced: the `https://` URL of
    /// an update feed and, optionally, the publisher's feed-signing key
    /// (`ed25519:<64 hex>`). An application reads the feed and offers releases built
    /// for its [`TARGET`].
    pub fn update(mut self, feed: impl Into<String>, key: Option<&str>) -> Self {
        self.update_feed = Some(feed.into());
        self.update_key = key.map(str::to_owned);
        self
    }
    /// The update feed URL and signing key, when [`update`](Self::update) set them.
    pub fn update_source(&self) -> Option<(&str, Option<&str>)> {
        Some((self.update_feed.as_deref()?, self.update_key.as_deref()))
    }
    /// The display name, when [`about`](Self::about) set one.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    /// What the package adds, when [`about`](Self::about) said so.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
    /// The asynchronous nodes, each with its definition.
    pub fn nodes(&self) -> &[(NodeDefinition, Arc<dyn ServiceNode>)] {
        &self.nodes
    }
    /// The entry nodes, each with its definition.
    pub fn sources(&self) -> &[(NodeDefinition, Arc<dyn ServiceSource>)] {
        &self.sources
    }
    /// The connection adapters by adapter kind.
    pub fn adapters(&self) -> &[(String, Arc<dyn ConnectionAdapter>)] {
        &self.adapters
    }
    /// What each definition file says about showing its node.
    pub fn presentations(&self) -> &[(DefinitionRef, Presentation)] {
        &self.presentations
    }
    /// Adds an asynchronous node from its definition file text.
    pub fn node(
        mut self,
        definition: &str,
        node: impl ServiceNode,
    ) -> Result<Self, DefinitionError> {
        let (definition, presentation) = NodeDefinition::described(definition)?;
        self.presentations
            .push((definition.definition.clone(), presentation));
        self.nodes.push((definition, Arc::new(node)));
        Ok(self)
    }
    /// Adds a node whose one data output is a stream, from its definition file text:
    /// an asynchronous node with an output of type `core/stream@1<T>`.
    pub fn stream(
        self,
        definition: &str,
        stream: impl ServiceStream,
    ) -> Result<Self, DefinitionError> {
        self.node(definition, Streaming(Arc::new(stream)))
    }
    /// Adds an entry node from its definition file text. The definition's kind is
    /// `entry`; it has a `then` outcome and at most one data output, `payload`.
    pub fn source(
        mut self,
        definition: &str,
        source: impl ServiceSource,
    ) -> Result<Self, DefinitionError> {
        let (definition, presentation) = NodeDefinition::described(definition)?;
        self.presentations
            .push((definition.definition.clone(), presentation));
        self.sources.push((definition, Arc::new(source)));
        Ok(self)
    }
    /// Adds the adapter for connections whose `adapter` is `kind`.
    pub fn adapter(mut self, kind: impl Into<String>, adapter: impl ConnectionAdapter) -> Self {
        self.adapters.push((kind.into(), Arc::new(adapter)));
        self
    }
}

/// Identifies the build a plugin library is compatible with: the SDK's interface
/// revision and version, the exact compiler and the target. A library exports the
/// value it was compiled with and a host loads it only when it equals the host's own.
///
/// Equal text is necessary, not sufficient: a plugin must also resolve the SDK's
/// dependencies to the same versions and features as the host, because their types
/// cross the boundary too. Building the plugin against the host's lockfile ensures it.
pub const ABI: &std::ffi::CStr = match std::ffi::CStr::from_bytes_with_nul(
    concat!(
        // The interface revision: raise it when a type or trait a plugin library
        // exchanges with its host changes shape.
        "orange-plugin 3",
        " sdk ",
        env!("CARGO_PKG_VERSION"),
        " rustc ",
        env!("ORANGE_SDK_RUSTC"),
        " ",
        env!("ORANGE_SDK_TARGET"),
        "\0"
    )
    .as_bytes(),
) {
    Ok(text) => text,
    Err(_) => panic!("the plugin ABI text contains a NUL byte"),
};

/// The compiler target this SDK was built for, such as `x86_64-pc-windows-msvc`: the
/// target a native plugin library release must name to be offered as an update.
pub const TARGET: &str = env!("ORANGE_SDK_TARGET");

/// Name of the symbol a plugin library exports for [`ABI`].
pub const ABI_SYMBOL: &str = "orange_plugin_abi";
/// Name of the symbol a plugin library exports for its [`ServicePackage`].
pub const PACKAGE_SYMBOL: &str = "orange_plugin_package";

/// Makes a `cdylib` crate a native plugin library: exports the function that returns
/// its [`ServicePackage`] and the [`ABI`] it was built with.
///
/// ```ignore
/// fn package() -> orange_sdk::ServicePackage { /* nodes, triggers, adapters */ }
/// orange_sdk::export_plugin!(package);
/// ```
///
/// A native plugin runs inside the application with all of its rights; nothing limits
/// what it does, and a panic in it ends the application. Use once per library.
#[macro_export]
macro_rules! export_plugin {
    ($package:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn orange_plugin_abi() -> *const ::core::ffi::c_char {
            $crate::ABI.as_ptr()
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn orange_plugin_package() -> *mut ::core::ffi::c_void {
            let package: $crate::ServicePackage = $package();
            ::std::boxed::Box::into_raw(::std::boxed::Box::new(package)).cast()
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_built_definition_is_the_file_a_package_would_write_by_hand() {
        let built = Definition::node("org.example.demo", "fetch", "Fetch")
            .description("Fetches a thing.")
            .category("demo", "Demo")
            .connection()
            .input("path", "core/string@1")
            .output("status", "core/i64@1")
            .build();
        let by_hand = serde_json::json!({
            "definition_id": "org.example.demo/fetch",
            "label": {"key": "org.example.demo/fetch-label", "fallback": "Fetch"},
            "description": {"key": "org.example.demo/fetch-description",
                "fallback": "Fetches a thing."},
            "editor": {"category_id": "org.example.demo/category-demo",
                "category": {"key": "org.example.demo/category-demo", "fallback": "Demo"}},
            "definition_version": 1, "config_version": 1,
            "kind": "async", "suspends": "always", "determinism": "external",
            "connection": true,
            "ports": [
                {"port_id": "path", "direction": "input", "kind": "data", "type": "core/string@1"},
                {"port_id": "status", "direction": "output", "kind": "data", "type": "core/i64@1",
                    "source": "execution", "available_on": ["then"]}
            ]
        });
        assert_eq!(serde_json::from_str::<Json>(&built).unwrap(), by_hand);
        // The host's own reader accepts it, with its text.
        let (definition, presentation) = NodeDefinition::described(&built).unwrap();
        assert_eq!(definition.definition.id.as_str(), "org.example.demo/fetch");
        let _ = presentation;

        let entry = Definition::entry("org.example.demo", "on-thing", "On thing")
            .payload("core/string@1")
            .build();
        let entry: Json = serde_json::from_str(&entry).unwrap();
        assert_eq!(entry["kind"], "entry");
        assert!(entry.get("suspends").is_none());
        assert_eq!(entry["ports"][0]["source"], "payload");
        assert!(NodeDefinition::described(&entry.to_string()).is_ok());
    }
}
