//! The variable side of a host: persistent variables, editing a flow's variables from
//! outside its graph, and the variables flows share.
use super::*;

impl Host {
    /// The persistent run variables of `document` and where this host stores them.
    pub(super) fn keep(
        &self,
        document: &GraphDocument,
        registry: &Arc<RegistrySnapshot>,
    ) -> Option<Keep> {
        let variables = state::kept(document, VarScope::Run);
        let directory = self.options.state_dir.clone()?;
        (!variables.is_empty()).then(|| Keep {
            directory,
            name: document.graph_id().to_string(),
            variables,
            registry: Arc::clone(registry),
        })
    }

    /// Sets a run variable of a flow from outside its graph, as an editor does when
    /// the user changes a value. The value must have the variable's declared type.
    /// The graph sees it at its next read, a trigger on the variable fires as for a
    /// write the graph made, and a persistent variable is stored. It waits for the
    /// flow's thread to apply the write, between two turns.
    pub fn set_variable(
        &self,
        flow: FlowId,
        id: VariableId,
        value: Value,
    ) -> Result<(), HostError> {
        let flow = self.flows.get(&flow).ok_or(HostError::UnknownFlow)?;
        let (answer, result) = channel();
        let query: Query = Box::new(move |run| {
            let _ = answer.send(run.set_variable(id, value));
        });
        flow.shared
            .queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
            .ok_or(HostError::FlowEnded)?
            .push_back(query);
        flow.wake();
        result
            .recv()
            .map_err(|_| HostError::FlowEnded)?
            .map_err(|_| {
                HostError::Variable(
                    "the graph does not use such a run variable, the value is not of its type, or the flow has ended"
                        .into(),
                )
            })
    }

    /// The value a document literal denotes, such as one a person typed or a stored
    /// variable holds: what [`set_variable`](Self::set_variable) takes.
    pub fn value_of(&self, literal: &Literal) -> Result<Value, HostError> {
        orange_model::value::literal_codec::decode(literal, self.store.current().types().as_ref())
            .map_err(|error| HostError::Variable(error.to_string()))
    }

    /// [`value_of`](Self::value_of) for a literal written as JSON text, such as
    /// `{"type": "core/i64@1", "value": "5"}`.
    pub fn value_from_json(&self, json: &str) -> Result<Value, HostError> {
        let literal: Literal = serde_json::from_str(json)
            .map_err(|_| HostError::Variable("the text is not a literal".into()))?;
        self.value_of(&literal)
    }

    /// Sets a shared variable from outside any graph. Every flow sees the value at its
    /// next turn, and a persistent one is stored.
    pub fn set_shared_variable(&self, id: VariableId, value: &Value) -> Result<(), HostError> {
        {
            let mut workspace = self
                .space
                .workspace
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let shared = workspace
                .as_mut()
                .ok_or_else(|| HostError::Variable("no shared variables are declared".into()))?;
            let _ = shared
                .set(id, value)
                .map_err(|error| HostError::Variable(error.to_string()))?;
            let _ = self.space.version.fetch_add(1, Ordering::AcqRel);
        }
        self.space.wake_all();
        self.space.store().map_err(HostError::State)
    }

    /// The variables stored for graph `graph`, or with `None` the stored shared
    /// variables: what a flow opened now would start with, whether or not one is
    /// running. Empty without [`HostOptions::state_dir`].
    pub fn stored_variables(
        &self,
        graph: Option<GraphId>,
    ) -> Result<Vec<StoredVariable>, HostError> {
        let Some(directory) = &self.options.state_dir else {
            return Ok(Vec::new());
        };
        let name = graph.map_or_else(|| state::SHARED.to_owned(), |graph| graph.to_string());
        state::load(directory, &name).map_err(HostError::State)
    }

    /// Changes one stored variable of graph `graph` (or, with `None`, a stored shared
    /// variable) while nothing uses it: the value a flow opened later starts with.
    /// `value` must be a literal of the type the variable was stored with. For a
    /// running flow use [`set_variable`](Self::set_variable), which also stores it.
    pub fn store_variable(
        &self,
        graph: Option<GraphId>,
        id: VariableId,
        value: Literal,
    ) -> Result<(), HostError> {
        let directory = self
            .options
            .state_dir
            .as_ref()
            .ok_or_else(|| HostError::State("the host has no state directory".into()))?;
        let name = graph.map_or_else(|| state::SHARED.to_owned(), |graph| graph.to_string());
        let mut stored = state::load(directory, &name).map_err(HostError::State)?;
        let entry = stored
            .iter_mut()
            .find(|variable| variable.id == id)
            .ok_or_else(|| HostError::State("no such variable is stored".into()))?;
        if entry.literal.ty != value.ty
            || orange_model::value::literal_codec::decode(
                &value,
                self.store.current().types().as_ref(),
            )
            .is_err()
        {
            return Err(HostError::State(
                "the value is not a literal of the stored type".into(),
            ));
        }
        entry.literal = value;
        state::save(directory, &name, &stored).map_err(HostError::State)
    }

    /// Declares the variables flows can share: the `workspace`-scope variables of the
    /// given document, with their initial values. It can be done once, before any
    /// document that uses them is opened. A document then declares the ones it uses
    /// with the same IDs and types, reads them with `core/get-variable` and writes them
    /// with `core/set-shared` or `core/update-shared`. Every flow sees a write at its
    /// next turn; writes from different flows are applied one at a time.
    pub fn share_variables(&mut self, json: &str) -> Result<(), HostError> {
        let document = from_json(json).map_err(|error| HostError::Document(error.to_string()))?;
        let mut workspace = self
            .space
            .workspace
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if workspace.is_some() {
            return Err(HostError::Workspace("they are already declared".into()));
        }
        let created = Workspace::new(
            &document,
            Arc::clone(self.store.current()),
            WorkspaceLimits::default(),
        )
        .map_err(|error| HostError::Workspace(error.to_string()))?;
        let mut created = created;
        // Persistent shared variables take their stored values, where those still fit.
        let kept = state::kept(&document, VarScope::Workspace);
        if let Some(directory) = self.options.state_dir.clone()
            && !kept.is_empty()
        {
            let registry = Arc::clone(self.store.current());
            let stored = state::load(&directory, state::SHARED).map_err(HostError::State)?;
            for (id, value) in state::restore(&stored, &kept, registry.types()) {
                let _ = created
                    .set(id, &value)
                    .map_err(|error| HostError::Workspace(error.to_string()))?;
            }
            *self
                .space
                .keep
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(Keep {
                directory,
                name: state::SHARED.to_owned(),
                variables: kept,
                registry,
            });
        }
        *workspace = Some(created);
        self.shared_variables = document
            .variables()
            .values()
            .filter(|variable| variable.scope == VarScope::Workspace)
            .map(|variable| variable.id)
            .collect();
        Ok(())
    }

    /// Every shared variable with its value now (`None` while unset), in declaration
    /// order: one consistent state, taken between two writes.
    pub fn shared_variables(&self) -> Vec<(VariableId, Option<Value>)> {
        let Some(snapshot) = self.space.snapshot() else {
            return Vec::new();
        };
        self.shared_variables
            .iter()
            .map(|id| (*id, snapshot.get(*id).ok().flatten().cloned()))
            .collect()
    }

    /// Counts the writes flows have made to shared variables. A front end reads
    /// [`shared_variables`](Self::shared_variables) again when this has changed.
    pub fn shared_writes(&self) -> u64 {
        self.space.version.load(Ordering::Relaxed)
    }

    /// The current value of a shared variable; `None` while unset or undeclared.
    pub fn shared_variable(&self, id: VariableId) -> Option<Value> {
        self.space.snapshot()?.get(id).ok().flatten().cloned()
    }
}
