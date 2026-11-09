//! Host-side execution of installed Wasm components (ADR-03, K02).
//!
//! A [`GuestEngine`] owns one Wasmtime engine with fuel and epoch interruption enabled.
//! Each [`GuestInstance`] has its own store, memory limit and an empty linker: a
//! component that imports anything the host has not granted fails to instantiate. A
//! trap poisons only its own instance; the engine and other instances keep running.
//!
//! This crate is a host boundary. Graph-core crates never depend on it, and no
//! Wasmtime type is part of the graph model.

mod nodes;
mod plugin;

use std::sync::Arc;

pub use nodes::{
    GuestLoadError, GuestPackage, MAX_FAULT_TEXT_BYTES, PLUGIN_WORLD_PREFIX, WithGuests,
};
pub use plugin::{
    API_VERSION, Grants, GuestNodeError, GuestPlugin, MAX_LOG_BYTES, MAX_LOG_ENTRIES,
};

use thiserror::Error;
use wasmtime::component::{Component, ComponentNamedList, Instance, Lift, Linker, Lower};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder, Trap};

/// Per-instance resource limits (04 proposals: 64 MiB linear memory, finite fuel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestLimits {
    /// Total linear-memory bytes one instance may grow to.
    pub memory_bytes: usize,
    /// Fuel granted to each call; it is reset, not accumulated, before every call.
    pub fuel_per_call: u64,
    /// Core instances one component instantiation may create.
    pub instances: usize,
    /// Native stack bytes guest code may use before trapping.
    pub max_wasm_stack: usize,
}

impl Default for GuestLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 64 * 1024 * 1024,
            fuel_per_call: 10_000_000,
            instances: 64,
            max_wasm_stack: 512 * 1024,
        }
    }
}

/// Why a guest stopped. Static, safe classifications; no guest text is retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestTrap {
    Unreachable,
    OutOfFuel,
    Interrupted,
    StackOverflow,
    MemoryOutOfBounds,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum GuestError {
    #[error("the guest engine configuration is invalid")]
    Engine,
    #[error("the component is not a valid component for this engine")]
    Compile,
    #[error("the component imports something the host has not granted")]
    Link,
    #[error("the component has no export with that name and type")]
    Export,
    #[error("the guest trapped: {0:?}")]
    Trap(GuestTrap),
    #[error("the guest instance trapped earlier and can no longer be called")]
    Poisoned,
    #[error("a host value does not have its declared type")]
    InputContract,
    #[error("the guest returned values or an error outside the interface contract")]
    OutputContract,
}

impl GuestError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::Engine => "core/guest-engine",
            Self::Compile => "core/guest-compile",
            Self::Link => "core/guest-link",
            Self::Export => "core/guest-export",
            Self::Trap(_) => "core/guest-trap",
            Self::Poisoned => "core/guest-poisoned",
            Self::InputContract => "core/guest-input-contract",
            Self::OutputContract => "core/guest-output-contract",
        }
    }
}

pub(crate) fn classify(error: &wasmtime::Error) -> GuestError {
    GuestError::Trap(match error.downcast_ref::<Trap>() {
        Some(Trap::UnreachableCodeReached) => GuestTrap::Unreachable,
        Some(Trap::OutOfFuel) => GuestTrap::OutOfFuel,
        Some(Trap::Interrupt) => GuestTrap::Interrupted,
        Some(Trap::StackOverflow) => GuestTrap::StackOverflow,
        Some(Trap::MemoryOutOfBounds) => GuestTrap::MemoryOutOfBounds,
        _ => GuestTrap::Other,
    })
}

/// One compilation engine shared by every instance it creates.
#[derive(Clone)]
pub struct GuestEngine {
    engine: Engine,
    limits: GuestLimits,
}

impl std::fmt::Debug for GuestEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestEngine")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl GuestEngine {
    pub fn new(limits: GuestLimits) -> Result<Self, GuestError> {
        let mut config = Config::new();
        config
            .consume_fuel(true)
            .epoch_interruption(true)
            .max_wasm_stack(limits.max_wasm_stack)
            .wasm_component_model(true);
        let engine = Engine::new(&config).map_err(|_| GuestError::Engine)?;
        Ok(Self { engine, limits })
    }

    pub fn limits(&self) -> GuestLimits {
        self.limits
    }

    /// Validates and compiles a component binary (or, when the host enables Wasmtime's
    /// `wat` feature, component text). Never loads precompiled native code.
    pub fn compile(&self, bytes: &[u8]) -> Result<GuestComponent, GuestError> {
        let component = Component::new(&self.engine, bytes).map_err(|_| GuestError::Compile)?;
        Ok(GuestComponent {
            engine: self.clone(),
            component: Arc::new(component),
        })
    }

    /// Makes every running call on this engine trap with [`GuestTrap::Interrupted`] at its
    /// next epoch check. Safe to call from any thread; this is the host's cancellation.
    pub fn interrupt(&self) {
        self.engine.increment_epoch();
    }
}

/// A compiled component, shareable across instances.
#[derive(Clone)]
pub struct GuestComponent {
    engine: GuestEngine,
    component: Arc<Component>,
}

impl std::fmt::Debug for GuestComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestComponent").finish_non_exhaustive()
    }
}

struct HostState {
    limits: StoreLimits,
}

/// One isolated instance with its own store and limits.
pub struct GuestInstance {
    store: Store<HostState>,
    instance: Instance,
    fuel: u64,
    poisoned: bool,
}

impl std::fmt::Debug for GuestInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestInstance")
            .field("poisoned", &self.poisoned)
            .finish_non_exhaustive()
    }
}

impl GuestInstance {
    /// Instantiates with no granted imports. Instantiation itself runs under the same
    /// fuel and epoch limits as a call.
    pub fn new(component: &GuestComponent) -> Result<Self, GuestError> {
        let limits = component.engine.limits;
        let mut store = Store::new(
            &component.engine.engine,
            HostState {
                limits: StoreLimitsBuilder::new()
                    .memory_size(limits.memory_bytes)
                    .instances(limits.instances)
                    .trap_on_grow_failure(false)
                    .build(),
            },
        );
        store.limiter(|state| &mut state.limits);
        store.epoch_deadline_trap();
        store.set_epoch_deadline(1);
        store
            .set_fuel(limits.fuel_per_call)
            .map_err(|_| GuestError::Engine)?;
        let linker = Linker::<HostState>::new(&component.engine.engine);
        let instance = linker
            .instantiate(&mut store, &component.component)
            .map_err(|error| match error.downcast_ref::<Trap>() {
                Some(_) => classify(&error),
                None => GuestError::Link,
            })?;
        Ok(Self {
            store,
            instance,
            fuel: limits.fuel_per_call,
            poisoned: false,
        })
    }

    /// Calls an exported function with Wasmtime-typed parameters and results. Fuel is
    /// reset first and the epoch deadline renewed, so each call has the same budget.
    pub fn call<Params, Results>(
        &mut self,
        export: &str,
        params: Params,
    ) -> Result<Results, GuestError>
    where
        Params: ComponentNamedList + Lower,
        Results: ComponentNamedList + Lift,
    {
        if self.poisoned {
            return Err(GuestError::Poisoned);
        }
        let func = self
            .instance
            .get_typed_func::<Params, Results>(&mut self.store, export)
            .map_err(|_| GuestError::Export)?;
        self.store
            .set_fuel(self.fuel)
            .map_err(|_| GuestError::Engine)?;
        self.store.set_epoch_deadline(1);
        func.call(&mut self.store, params).map_err(|error| {
            self.poisoned = true;
            classify(&error)
        })
    }

    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    /// Fuel left after the last call.
    pub fn remaining_fuel(&self) -> u64 {
        self.store.get_fuel().unwrap_or(0)
    }
}
