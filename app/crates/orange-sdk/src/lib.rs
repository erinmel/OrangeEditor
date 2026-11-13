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
        }
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
