//! The life of a flow: preparing a document, starting it on its own thread, editing it
//! while it runs, controlling and observing it, and closing it.
use super::*;
use orange_runtime::RunInbox;
use std::sync::atomic::AtomicU8;

const STAGED: u8 = 0;
const ACTIVE: u8 = 1;
const CLOSED: u8 = 2;

pub(crate) struct ActiveFeed {
    active: Arc<AtomicU8>,
    _guard: Box<dyn Send>,
}

impl ActiveFeed {
    pub(crate) fn activate(&self) {
        self.active.store(ACTIVE, Ordering::Release);
    }
}

impl Drop for ActiveFeed {
    fn drop(&mut self) {
        self.active.store(CLOSED, Ordering::Release);
    }
}

impl Host {
    /// Validates and compiles a saved document and resolves everything it needs from
    /// the host, without starting anything. `previous` are the sessions of the flow
    /// being edited, reused where a connection's declaration is unchanged.
    fn prepare(
        &mut self,
        document: GraphDocument,
        semantic: String,
        previous: Option<&Opened>,
    ) -> Result<Prepared, HostError> {
        let document = self.link_document(document)?;
        let registry = self.registry_for(&document);
        let analysis = analyzed(&document, &registry);
        if !analysis.is_ready_for_lowering(&document, &registry) {
            return Err(HostError::Invalid(analysis.report().diagnostics.clone()));
        }
        let mut resolver = WithGuests::new(self.native.clone());
        for guest in &self.guests {
            resolver.add(guest.clone());
        }
        let plan = Arc::new(
            effects::guard(|| {
                compile_control_with_effects(
                    &document,
                    Arc::clone(&registry),
                    &analysis,
                    &resolver,
                    &resolver,
                    CompileOptions::default(),
                )
            })
            .map_err(|_| RunError::Interrupted)??,
        );
        let opened = self.sessions(&document, &registry, previous)?;
        // The session of a node that uses a connection: the one it names.
        let session_of = |node: &orange_model::document::NodeInstance,
                          refused: &dyn Fn(&str) -> HostError|
         -> Result<Option<Arc<dyn Session>>, HostError> {
            let id: ConnectionId = node
                .config
                .get("connection")
                .and_then(|value| value.as_str())
                .and_then(|text| text.parse().ok())
                .ok_or_else(|| refused("the node names no connection"))?;
            let (_, session) = opened.get(&id).ok_or_else(|| {
                refused("the node names a connection this document does not declare")
            })?;
            Ok(Some(Arc::clone(session)))
        };
        // The data inputs a source is armed with, by name, in the run's order.
        let arguments = |node: NodeId| -> Option<Vec<String>> {
            let sources = plan.sources();
            let index = sources.partition_point(|source| source.node() < node);
            sources
                .get(index)
                .filter(|source| source.node() == node)
                .map(|source| source.ports().iter().map(ToString::to_string).collect())
        };
        let mut armables = Vec::new();
        // Every package entry needs a source before anything starts.
        let mut feeds = Vec::new();
        let mut manual_entries = Vec::new();
        let mut package_order = Vec::new();
        let mut package_entries = BTreeMap::<_, BTreeSet<_>>::new();
        for entry in plan.manuals() {
            let node = document
                .node(entry.node())
                .ok_or_else(|| HostError::Source(entry.node(), "node not found".into()))?;
            if node.definition.id.is_core() {
                manual_entries.push((entry.node(), entry.payload_type().is_some()));
                continue;
            }
            package_entries
                .entry(node.id)
                .or_insert_with(|| {
                    package_order.push(node.id);
                    Default::default()
                })
                .insert(entry.port().clone());
        }
        for source in plan.sources() {
            if source.kind() == orange_compiler::SourceKind::Entry {
                package_entries.entry(source.node()).or_insert_with(|| {
                    package_order.push(source.node());
                    Default::default()
                });
            }
        }
        for id in package_order {
            let routes = package_entries.remove(&id).unwrap_or_default();
            let node = document
                .node(id)
                .ok_or_else(|| HostError::Source(id, "node not found".into()))?;
            let refused = |reason: &str| HostError::Source(node.id, reason.to_owned());
            let provider = node
                .provider
                .as_ref()
                .and_then(|binding| ProviderKey::of(&document, binding))
                .ok_or_else(|| refused("the node has no exact provider"))?;
            let (source, uses_connection) = self
                .native
                .1
                .source(&provider, &node.definition)
                .or_else(|| self.guest_trigger(&provider, &node.definition, &registry))
                .ok_or_else(|| refused("no service provides this entry"))?;
            let session = if uses_connection {
                session_of(node, &refused)?
            } else {
                None
            };
            // An entry with data inputs starts once its run has their values.
            let outputs: Vec<_> = plan.live_outputs(node.id).cloned().collect();
            let ports = arguments(node.id);
            if ports.is_some()
                || !outputs.is_empty()
                || routes.iter().any(|port| port.port().as_str() != "then")
            {
                armables.push(armed::Armable {
                    node: node.id,
                    definition: node.definition.clone(),
                    provider: armed::Provider::Entry(source),
                    session,
                    settings: node.config.clone(),
                    ports: ports.unwrap_or_default(),
                    outputs,
                    routes,
                });
            } else {
                feeds.push((
                    node.id,
                    node.definition.clone(),
                    source,
                    session,
                    node.config.clone(),
                ));
            }
        }
        // Every live node needs the provider that publishes its value.
        for live in plan.lives() {
            if plan.live_index(live.node()).is_none() {
                continue;
            }
            let node = document
                .node(live.node())
                .ok_or_else(|| HostError::Source(live.node(), "node not found".into()))?;
            let refused = |reason: &str| HostError::Source(node.id, reason.to_owned());
            let provider = node
                .provider
                .as_ref()
                .and_then(|binding| ProviderKey::of(&document, binding))
                .ok_or_else(|| refused("the node has no exact provider"))?;
            let (source, uses_connection) = self
                .native
                .1
                .live(&provider, &node.definition)
                .ok_or_else(|| refused("no service provides this live value"))?;
            let session = if uses_connection {
                session_of(node, &refused)?
            } else {
                None
            };
            armables.push(armed::Armable {
                node: node.id,
                definition: node.definition.clone(),
                provider: armed::Provider::Live(source),
                session,
                settings: node.config.clone(),
                ports: arguments(node.id).unwrap_or_default(),
                outputs: vec![live.port().clone()],
                routes: Default::default(),
            });
        }
        Ok(Prepared {
            semantic,
            document,
            registry,
            plan,
            opened,
            feeds,
            armables,
            manual_entries,
        })
    }

    /// Starts the source of every package entry of a flow. On a failure the sources
    /// already started are stopped again.
    pub(crate) fn stage_feeds(
        sink: &Sink,
        id: FlowId,
        shared: &Arc<flow::Shared>,
        inbox: &RunInbox,
        thread: &thread::Thread,
        feeds: Vec<Feed>,
        gate: &services::Gate,
    ) -> Result<Vec<ActiveFeed>, HostError> {
        let mut staged = Vec::with_capacity(feeds.len());
        for (node, definition, source, session, settings) in feeds {
            if !gate.allows(
                node,
                &definition,
                ServiceOperation::Entry,
                services::declared_connection(&settings),
                &[],
            ) {
                return Err(HostError::Source(
                    node,
                    "the host denied the service entry".into(),
                ));
            }
            let inbox = inbox.clone();
            let thread = thread.clone();
            let (sink, shared) = (sink.clone(), Arc::clone(shared));
            let active = Arc::new(AtomicU8::new(STAGED));
            let (can_emit, can_fault) = (Arc::clone(&active), Arc::clone(&active));
            let emitter = Emitter::with_faults(
                move |payload| {
                    match can_emit.load(Ordering::Acquire) {
                        STAGED | ACTIVE => {}
                        _ => return Err(EmitError::Closed),
                    }
                    let input = HostInput::Trigger {
                        node,
                        payload,
                        wall_ns: wall_ns(),
                    };
                    inbox.send(input).map_err(|error| match error {
                        InboxError::Full(_) => EmitError::Busy,
                        InboxError::Closed(_) => EmitError::Closed,
                    })?;
                    thread.unpark();
                    Ok(())
                },
                move |fault| {
                    if can_fault.load(Ordering::Acquire) != ACTIVE
                        || shared.closing.load(Ordering::Acquire)
                    {
                        return Err(EmitError::Closed);
                    }
                    // Keep diagnostics ahead of the final Finished observation, even
                    // when the flow stops on its own while this callback is pending.
                    let state = shared.state.try_lock().map_err(|_| EmitError::Busy)?;
                    if matches!(
                        *state,
                        RunState::Stopped | RunState::Faulted | RunState::Disposed
                    ) {
                        return Err(EmitError::Closed);
                    }
                    sink.send(id, FlowEvent::SourceFault { node, fault });
                    Ok(())
                },
            );
            let started = effects::guard(|| source.start_with(session, &settings, emitter))
                .map_err(|fault| fault.message.to_string())
                .and_then(|result| result);
            match started {
                Ok(guard) => staged.push(ActiveFeed {
                    active,
                    _guard: guard,
                }),
                Err(reason) => {
                    active.store(CLOSED, Ordering::Release);
                    return Err(HostError::Source(node, reason));
                }
            }
        }
        Ok(staged)
    }

    /// Validates, compiles and starts a document as a new flow on its own thread.
    pub fn open(&mut self, json: &str) -> Result<FlowId, HostError> {
        let document = Self::decode_document(json)?;
        let prepared = self.prepare(document, semantic(json)?, None)?;
        let mut keep = self.keep(&prepared.document, &prepared.registry);
        if keep.is_some()
            && self.flows.values().any(|flow| {
                flow.info.graph == prepared.document.graph_id()
                    && flow
                        .thread
                        .as_ref()
                        .is_some_and(|thread| !thread.is_finished())
            })
        {
            return Err(HostError::State(
                "a run already owns this graph's persistent state".into(),
            ));
        }
        // Stored values that no longer fit the document are left out; a file that
        // cannot be read refuses the flow rather than silently starting it empty.
        let mut values = BTreeMap::new();
        if let Some(keep) = &mut keep {
            keep.acquire(None)?;
            let stored = state::load(&keep.directory, &keep.name).map_err(HostError::State)?;
            let restored = state::restore(&stored, &keep.variables, keep.registry.types());
            values.extend(restored.into_iter().map(|(id, value)| (id, Some(value))));
        }
        let run = if flow::uses_space(&prepared.plan) {
            let snapshot = self.space.snapshot().ok_or_else(|| {
                HostError::Workspace(
                    "the document uses shared variables and none are declared".into(),
                )
            })?;
            Run::start_in_workspace_with_variables(
                Arc::clone(&prepared.plan),
                &prepared.document,
                &prepared.registry,
                0,
                self.options.run.clone(),
                &snapshot,
                &values,
            )?
        } else {
            Run::start_with_variables(
                Arc::clone(&prepared.plan),
                &prepared.document,
                &prepared.registry,
                0,
                self.options.run.clone(),
                &values,
            )?
        };

        let now = u64::try_from(self.clock.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let (mut driver, inbox) = RunDriver::new(run, self.options.inbox_capacity.max(1), now);
        let sources = flow::event_sources(&mut driver, &prepared.plan);
        let info = FlowInfo {
            graph: prepared.document.graph_id(),
            manual_entries: prepared.manual_entries,
            events: sources.keys().copied().collect(),
        };

        let id = FlowId(self.next_flow);
        self.next_flow += 1;
        let shared = Arc::new(Shared {
            state: Mutex::new(driver.run().state()),
            queries: Mutex::new(Some(VecDeque::new())),
            closing: AtomicBool::new(false),
            observed: AtomicBool::new(false),
            swap: Mutex::new(None),
        });
        let gate = services::Gate {
            graph: prepared.document.graph_id(),
            policy: self.options.connection_authorizer.clone(),
        };
        let context = Context {
            keep,
            id,
            shared: Arc::clone(&shared),
            sink: self.sink.clone(),
            pool: self.pool.scoped(),
            inbox: inbox.clone(),
            plan: prepared.plan,
            sessions: sessions_of(&prepared.opened),
            space: Arc::clone(&self.space),
            clock: self.clock,
            turn_inputs: self.options.turn_inputs.max(1),
            turn_budget: self.options.turn_budget.max(1),
            armed: armed::Armed::new(prepared.armables, gate.clone()),
            gate: gate.clone(),
        };
        let (launch, ready) = channel();
        let thread = thread::Builder::new()
            .name(format!("orange-flow-{id}"))
            .spawn(move || {
                if ready.recv().is_ok() {
                    flow::drive(driver, context);
                }
            })
            .map_err(|error| HostError::Start(error.to_string()))?;
        let feeds = Self::stage_feeds(
            &self.sink,
            id,
            &shared,
            &inbox,
            thread.thread(),
            prepared.feeds,
            &gate,
        )
        .and_then(|feeds| {
            for feed in &feeds {
                feed.activate();
            }
            effects::guard(|| {
                for (_, session) in prepared.opened.values() {
                    session.start();
                }
            })
            .map_err(|fault| HostError::Start(fault.message.to_string()))?;
            Ok(feeds)
        });
        let feeds = match feeds {
            Ok(feeds) => feeds,
            Err(error) => {
                // The actor has not pumped On Start or any queued input.
                drop(launch);
                let _ = thread.join();
                return Err(error);
            }
        };
        launch.send(()).map_err(|_| HostError::FlowEnded)?;
        let _ = self
            .space
            .flows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, thread.thread().clone());
        let flow = Flow {
            info,
            inbox,
            sources,
            shared,
            feeds,
            thread: Some(thread),
            opened: prepared.opened,
            semantic: prepared.semantic,
        };
        let _ = self.flows.insert(id, flow);
        Ok(id)
    }

    /// Replaces a running flow's graph with an edited version of the same document.
    ///
    /// The edit is validated and compiled first; if that fails the flow is untouched and
    /// keeps running. Otherwise, between two turns of the flow, its control flow
    /// restarts on the new graph: work in progress is cancelled, On Start runs again and
    /// On Stop does not. Run variables keep their values where a variable with the same
    /// ID and type still exists; other variables, and the private state of nodes such
    /// as gates and counters, start fresh. A paused flow stays paused. Connections whose
    /// declaration is unchanged keep their session. A change to layout alone does
    /// nothing.
    pub fn update(&mut self, flow: FlowId, json: &str) -> Result<Updated, HostError> {
        let current = self.flows.get(&flow).ok_or(HostError::UnknownFlow)?;
        let document = Self::decode_document(json)?;
        let semantic = semantic(json)?;
        if semantic == current.semantic {
            return Ok(Updated::default());
        }
        let previous = current.opened.clone();
        let graph = current.info.graph;
        if document.graph_id() != graph {
            return Err(HostError::Document(
                "the edit belongs to another graph".into(),
            ));
        }
        let next_keeps_state =
            self.options.state_dir.is_some() && !state::kept(&document, VarScope::Run).is_empty();
        if next_keeps_state
            && self.flows.iter().any(|(id, other)| {
                *id != flow
                    && other.info.graph == graph
                    && other
                        .thread
                        .as_ref()
                        .is_some_and(|thread| !thread.is_finished())
            })
        {
            return Err(HostError::State(
                "another run owns this graph's persistent state".into(),
            ));
        }
        let prepared = self.prepare(document, semantic, Some(&previous))?;
        let keep = self.keep(&prepared.document, &prepared.registry);
        let (reply, answer) = channel();
        let swap = flow::Swap {
            keep,
            plan: prepared.plan,
            document: prepared.document,
            registry: prepared.registry,
            options: self.options.run.clone(),
            inbox_capacity: self.options.inbox_capacity.max(1),
            sessions: sessions_of(&prepared.opened),
            armables: prepared.armables,
            feeds: prepared.feeds,
            reply,
        };
        let current = self.flows.get_mut(&flow).ok_or(HostError::UnknownFlow)?;
        *current
            .shared
            .swap
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(swap);
        current.wake();
        let waiting_since = Instant::now();
        let swapped = loop {
            match answer.recv_timeout(std::time::Duration::from_millis(20)) {
                Ok(outcome) => break outcome?,
                Err(RecvTimeoutError::Disconnected) => return Err(HostError::FlowEnded),
                Err(RecvTimeoutError::Timeout) => {
                    // A flow that ended without taking the edit will never answer.
                    if current.thread.as_ref().is_none_or(JoinHandle::is_finished) {
                        return Err(HostError::FlowEnded);
                    }
                    if waiting_since.elapsed() >= std::time::Duration::from_secs(2)
                        && current
                            .shared
                            .swap
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .take()
                            .is_some()
                    {
                        return Err(HostError::Busy);
                    }
                }
            }
        };
        // Sources of the old graph fed its mailbox; replace them with the new graph's.
        current.feeds.clear();
        current.inbox = swapped.inbox;
        current.info.manual_entries = prepared.manual_entries;
        current.info.events = swapped.sources.keys().copied().collect();
        current.sources = swapped.sources;
        current.opened = prepared.opened;
        current.semantic = prepared.semantic;
        current.feeds = swapped.feeds;
        Ok(Updated {
            changed: true,
            kept_variables: swapped.kept,
            reset_variables: swapped.reset,
        })
    }

    /// Flows that have not been closed, including ones whose run has ended.
    pub fn flows(&self) -> impl Iterator<Item = FlowId> + '_ {
        self.flows.keys().copied()
    }

    pub fn info(&self, flow: FlowId) -> Option<&FlowInfo> {
        self.flows.get(&flow).map(|flow| &flow.info)
    }

    /// The state the flow's thread last observed.
    pub fn state(&self, flow: FlowId) -> Option<RunState> {
        let flow = self.flows.get(&flow)?;
        Some(
            *flow
                .shared
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    fn send(&self, flow: FlowId, input: HostInput) -> Result<(), HostError> {
        let flow = self.flows.get(&flow).ok_or(HostError::UnknownFlow)?;
        flow.inbox.send(input).map_err(|error| match error {
            InboxError::Full(_) => HostError::Busy,
            InboxError::Closed(_) => HostError::FlowEnded,
        })?;
        flow.wake();
        Ok(())
    }

    /// Holds the flow: no graph work runs and its clock stops. Input is still taken
    /// and handled by each entry's `while_paused` policy.
    pub fn pause(&self, flow: FlowId) -> Result<(), HostError> {
        self.send(flow, HostInput::Pause)
    }
    pub fn resume(&self, flow: FlowId) -> Result<(), HostError> {
        self.send(flow, HostInput::Resume)
    }
    /// Asks the flow to stop. Its `Finished` event follows once cleanup is done.
    pub fn stop(&self, flow: FlowId, mode: StopMode) -> Result<(), HostError> {
        self.send(flow, HostInput::Stop(mode))
    }
    /// Fires a root manual entry.
    pub fn trigger(
        &self,
        flow: FlowId,
        node: NodeId,
        payload: Option<Value>,
    ) -> Result<(), HostError> {
        self.send(
            flow,
            HostInput::Trigger {
                node,
                payload,
                wall_ns: wall_ns(),
            },
        )
    }
    /// Raises a declared event in the flow, as an external source would.
    pub fn emit(&self, flow: FlowId, event: EventDeclId, payload: Value) -> Result<(), HostError> {
        let source = *self
            .flows
            .get(&flow)
            .ok_or(HostError::UnknownFlow)?
            .sources
            .get(&event)
            .ok_or(HostError::UnknownEvent)?;
        self.send(
            flow,
            HostInput::Event {
                source,
                payload,
                wall_ns: wall_ns(),
            },
        )
    }

    /// Every node a document can use, ordered by ID and version: the core language, the
    /// bundled and built-in service packages and every installed package. This is the
    /// data of a node palette; a node's ports are in its definition.
    pub fn nodes(&self) -> impl Iterator<Item = NodeInfo<'_>> {
        let registry = self.store.current();
        registry.definitions().map(move |definition| NodeInfo {
            definition,
            presentation: self.presentations.get(&definition.definition),
            provider: registry.new_binding_default(&definition.definition),
        })
    }

    /// Turns reporting of the flow's values on or off. While on, the flow sends
    /// [`FlowEvent::Variables`] and [`FlowEvent::Ports`] after each turn in which a
    /// variable or an output port differs from what was last reported, starting with
    /// every variable. It costs one comparison per variable and per port per turn, so a
    /// front end observes the flows it is showing.
    pub fn observe(&self, flow: FlowId, on: bool) -> Result<(), HostError> {
        let flow = self.flows.get(&flow).ok_or(HostError::UnknownFlow)?;
        flow.shared.observed.store(on, Ordering::Relaxed);
        flow.wake();
        Ok(())
    }

    /// Reads from the flow's run on its own thread, between two turns, and returns the
    /// result. The run is never locked or paused for it. Keep `read` short: the flow
    /// does nothing else meanwhile.
    pub fn inspect<T: Send + 'static>(
        &self,
        flow: FlowId,
        read: impl FnOnce(&Run) -> T + Send + 'static,
    ) -> Result<T, HostError> {
        let flow = self.flows.get(&flow).ok_or(HostError::UnknownFlow)?;
        let (answer, result) = channel();
        let query: Query = Box::new(move |run| {
            let _ = answer.send(effects::guard(|| read(&*run)));
            false
        });
        flow.enqueue_query(query, self.options.inbox_capacity)?;
        result
            .recv()
            .map_err(|_| HostError::FlowEnded)?
            .map_err(|_| HostError::Run(RunError::Interrupted))
    }

    /// Reads one variable at an actor turn boundary.
    pub fn variable_value(
        &self,
        flow: FlowId,
        variable: VariableId,
    ) -> Result<Option<Value>, HostError> {
        self.inspect(flow, move |run| run.variable(variable).cloned())
    }

    /// Stops the flow at once if it is still running, waits for its thread and forgets
    /// it. Its remaining events stay readable.
    pub fn close(&mut self, flow: FlowId) -> Result<(), HostError> {
        let _ = self
            .space
            .flows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&flow);
        let mut flow = self.flows.remove(&flow).ok_or(HostError::UnknownFlow)?;
        flow.feeds.clear();
        flow.shared.closing.store(true, Ordering::Relaxed);
        flow.wake();
        if let Some(thread) = flow.thread.take() {
            let _ = thread.join();
        }
        Ok(())
    }
}
