//! Sources a flow's run arms with arguments, and the live values their providers
//! publish (11 §10.5). Everything here runs on the flow's own thread.
use crate::{FlowEvent, FlowId, flow::Sink};
use orange_model::Value;
use orange_model::id::{NodeId, PortKey};
use orange_runtime::{HostInput, InboxError, Run, RunInbox};
use orange_sdk::{Arguments, EmitError, Emitter, LiveSource, Publisher, ServiceSource, Session};
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::Thread;
use std::time::{Duration, Instant};

/// What a source's provider delivers: firings of an entry, or values of a live node.
pub(crate) enum Provider {
    Entry(Arc<dyn ServiceSource>),
    Live(Arc<dyn LiveSource>),
}

/// A package entry with data inputs, or a live node, ready to be armed.
pub(crate) struct Armable {
    pub node: NodeId,
    pub provider: Provider,
    pub session: Option<Arc<dyn Session>>,
    pub settings: serde_json::Map<String, Json>,
    /// The names of its data inputs, in the order the run hands their values.
    pub ports: Vec<String>,
    pub outputs: Vec<PortKey>,
    pub routes: BTreeSet<PortKey>,
}

/// The latest value a live provider published, until the flow passes it to its run.
#[derive(Default)]
struct Slot {
    latest: Mutex<Option<Option<Value>>>,
    fresh: AtomicBool,
}

struct OutputSlot {
    pending: Arc<Slot>,
    published: Option<Instant>,
}

/// Least time between two values of one live node reaching the run. A faster provider
/// is read as its latest value, so its rate cannot flood the run.
const LIVE_INTERVAL: Duration = Duration::from_millis(10);

struct ArmedProvider {
    active: Arc<AtomicBool>,
    _guard: Box<dyn Send>,
}
impl Drop for ArmedProvider {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

/// The armed sources of one flow and the live values waiting for its run.
#[derive(Default)]
pub(crate) struct Armed {
    armables: BTreeMap<NodeId, Armable>,
    guards: BTreeMap<NodeId, ArmedProvider>,
    slots: BTreeMap<NodeId, BTreeMap<PortKey, OutputSlot>>,
}
impl std::fmt::Debug for Armed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Armed")
            .field("sources", &self.armables.len())
            .field("armed", &self.guards.len())
            .finish()
    }
}
impl Armed {
    pub(crate) fn new(armables: Vec<Armable>) -> Self {
        Self {
            armables: armables.into_iter().map(|a| (a.node, a)).collect(),
            ..Self::default()
        }
    }

    /// Stops every provider and takes the sources of an edited graph instead.
    pub(crate) fn replace(&mut self, armables: Vec<Armable>) {
        *self = Self::new(armables);
    }

    /// Starts the live nodes that take no arguments. The others, and entries with data
    /// inputs, wait for the arguments their run evaluates. Returns the nodes whose
    /// provider refused to start, with why.
    pub(crate) fn start(
        &mut self,
        inbox: &RunInbox,
        flow: &Thread,
        events: (&Sink, FlowId),
    ) -> Vec<(NodeId, String)> {
        let idle: Vec<NodeId> = self
            .armables
            .values()
            .filter(|armable| armable.ports.is_empty())
            .map(|armable| armable.node)
            .collect();
        idle.into_iter()
            .filter_map(|node| {
                self.arm(node, &[], inbox, flow, events)
                    .err()
                    .map(|e| (node, e))
            })
            .collect()
    }

    /// Starts `node`'s provider with `values` for its data inputs, after stopping the
    /// one started with the previous values.
    pub(crate) fn arm(
        &mut self,
        node: NodeId,
        values: &[Value],
        inbox: &RunInbox,
        flow: &Thread,
        events: (&Sink, FlowId),
    ) -> Result<(), String> {
        let Some(armable) = self.armables.get(&node) else {
            return Ok(());
        };
        drop(self.guards.remove(&node));
        self.slots.remove(&node);
        let arguments = Arguments::new(
            armable
                .ports
                .iter()
                .cloned()
                .zip(values.iter().cloned())
                .collect(),
        );
        let session = armable.session.clone();
        let active = Arc::new(AtomicBool::new(true));
        let slots: BTreeMap<_, _> = armable
            .outputs
            .iter()
            .map(|port| (port.clone(), Arc::new(Slot::default())))
            .collect();
        self.slots.insert(
            node,
            slots
                .iter()
                .map(|(port, pending)| {
                    (
                        port.clone(),
                        OutputSlot {
                            pending: Arc::clone(pending),
                            published: None,
                        },
                    )
                })
                .collect(),
        );
        let started = match &armable.provider {
            Provider::Entry(source) => {
                let (route_inbox, route_thread, publish_thread) =
                    (inbox.clone(), flow.clone(), flow.clone());
                let (route_active, publish_active) = (Arc::clone(&active), Arc::clone(&active));
                let routes = armable.routes.clone();
                let (inbox, thread) = (inbox.clone(), flow.clone());
                let (send_active, fault_active) = (Arc::clone(&active), Arc::clone(&active));
                let (sink, id) = (events.0.clone(), events.1);
                let emitter = Emitter::with_faults(
                    move |payload| {
                        if !send_active.load(Ordering::Acquire) {
                            return Err(EmitError::Closed);
                        }
                        let input = HostInput::Trigger {
                            node,
                            payload,
                            wall_ns: crate::wall_ns(),
                        };
                        inbox.send(input).map_err(|error| match error {
                            InboxError::Full(_) => EmitError::Busy,
                            InboxError::Closed(_) => EmitError::Closed,
                        })?;
                        thread.unpark();
                        Ok(())
                    },
                    move |fault| {
                        if !fault_active.load(Ordering::Acquire) {
                            return Err(EmitError::Closed);
                        }
                        sink.send(id, FlowEvent::SourceFault { node, fault });
                        Ok(())
                    },
                )
                .with_outputs(
                    move |port| {
                        if !route_active.load(Ordering::Acquire) || !routes.contains(port) {
                            return Err(EmitError::Closed);
                        }
                        route_inbox
                            .send(HostInput::TriggerOn {
                                node,
                                port: port.clone(),
                                wall_ns: crate::wall_ns(),
                                active: Some(Arc::clone(&route_active)),
                            })
                            .map_err(|error| match error {
                                InboxError::Full(_) => EmitError::Busy,
                                InboxError::Closed(_) => EmitError::Closed,
                            })?;
                        route_thread.unpark();
                        Ok(())
                    },
                    move |port, value| {
                        let slot = slots.get(port).ok_or(EmitError::Closed)?;
                        slot.publish(&publish_active, &publish_thread, value)
                    },
                );
                source.start_with_arguments(session, &armable.settings, &arguments, emitter)
            }
            Provider::Live(source) => {
                let slot = slots.into_values().next().ok_or("missing live output")?;
                let thread = flow.clone();
                let publishing = Arc::clone(&active);
                let publisher =
                    Publisher::new(move |value| slot.publish(&publishing, &thread, Some(value)));
                source.start(session, &armable.settings, &arguments, publisher)
            }
        };
        match started {
            Ok(guard) => {
                self.guards.insert(
                    node,
                    ArmedProvider {
                        active,
                        _guard: guard,
                    },
                );
                Ok(())
            }
            Err(reason) => {
                active.store(false, Ordering::Release);
                self.slots.remove(&node);
                Err(reason)
            }
        }
    }

    /// Passes each live port's latest value to `run`, at most one per
    /// [`LIVE_INTERVAL`]. Returns how long until a value held back may go.
    pub(crate) fn flush(&mut self, run: &mut Run) -> Option<Duration> {
        let now = Instant::now();
        let mut wait: Option<Duration> = None;
        for (node, slots) in &mut self.slots {
            for (port, output) in slots {
                let slot = &output.pending;
                if !slot.fresh.load(Ordering::Acquire) {
                    continue;
                }
                if let Some(since) = output.published.map(|at| now - at)
                    && since < LIVE_INTERVAL
                {
                    let left = LIVE_INTERVAL - since;
                    wait = Some(wait.map_or(left, |w| w.min(left)));
                    continue;
                }
                slot.fresh.store(false, Ordering::Release);
                let value = slot
                    .latest
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                if let Some(value) = value {
                    // A value of the wrong type or past the run's limits is not taken; the
                    // provider's next one may be.
                    let _ = run.publish_live_port(*node, port, value);
                    output.published = Some(now);
                }
            }
        }
        wait
    }

    /// Stops every provider.
    pub(crate) fn stop(&mut self) {
        self.guards.clear();
        self.slots.clear();
    }
}

impl Slot {
    fn publish(
        &self,
        active: &AtomicBool,
        flow: &Thread,
        value: Option<Value>,
    ) -> Result<(), EmitError> {
        if !active.load(Ordering::Acquire) {
            return Err(EmitError::Closed);
        }
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = Some(value);
        self.fresh.store(true, Ordering::Release);
        flow.unpark();
        Ok(())
    }
}
