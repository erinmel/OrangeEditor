//! A document being edited: the parsed graph, its undo history, and validation that
//! follows each edit instead of starting over.
//!
//! A [`Draft`] is made once from a document's text ([`Host::draft`]) and changed only
//! through the host's editing operations, each one model transaction. Text is produced
//! only when it is needed ([`Draft::text`]: saving, copying, handing the graph to a flow).
//! [`Host::problems`] keeps an [`AnalysisSession`] beside the draft and gives it every
//! committed change, so a small edit of a large graph re-checks what the edit touched.
use super::*;
use orange_model::validation::{AnalysisOptions, AnalysisSession};
use orange_model::{ChangeSet, Committed, DocCommand, Inverse, TxError};

/// The analysis kept beside a draft, with the registry it was made against.
#[derive(Debug)]
struct Analysis {
    registry: Arc<RegistrySnapshot>,
    effects: BTreeMap<NodeId, LeafEffects>,
    session: AnalysisSession,
}

impl Analysis {
    fn new(document: &GraphDocument, registry: &Arc<RegistrySnapshot>) -> Self {
        let effects = leaf_effects(document);
        Self {
            registry: Arc::clone(registry),
            session: AnalysisSession::new(
                document,
                registry,
                effects.clone(),
                AnalysisOptions::default(),
            ),
            effects,
        }
    }

    /// Follows one committed change. A session is given each change while the document
    /// is in that change's resulting state; handing it several at once would make all
    /// but the last look stale and force a full analysis for each.
    fn follow(&mut self, document: &GraphDocument, change: &ChangeSet) {
        let options = AnalysisOptions::default();
        let _ = self
            .session
            .update(document, &self.registry, change, options);
        let effects = leaf_effects(document);
        if effects != self.effects {
            let _ =
                self.session
                    .set_leaf_effects(document, &self.registry, effects.clone(), options);
            self.effects = effects;
        }
    }
}

/// A document being edited: the parsed graph, its undo history, and validation that
/// follows each edit. Made with [`Host::draft`] and changed only through the host's
/// editing operations, each one model transaction; [`Host::problems`] checks again only
/// what an edit touched. Text is produced only when needed ([`Draft::text`]).
#[derive(Debug)]
pub struct Draft {
    document: GraphDocument,
    undo: Vec<Inverse>,
    redo: Vec<Inverse>,
    /// Kept once [`Host::problems`] has been asked; then it follows every edit.
    analysis: Option<Analysis>,
}

/// Most edits a draft can undo.
pub const MAX_UNDO: usize = 200;

impl Draft {
    fn new(document: GraphDocument) -> Self {
        Self {
            document,
            undo: Vec::new(),
            redo: Vec::new(),
            analysis: None,
        }
    }

    /// The document as it is now.
    pub fn document(&self) -> &GraphDocument {
        &self.document
    }

    /// The document's text, in the saved format.
    pub fn text(&self) -> Result<String, HostError> {
        to_json(&self.document).map_err(|error| refused(&error))
    }

    /// What the kept analysis did for the last edit, once [`Host::problems`] has been
    /// asked: whether it re-checked a scope or ran in full, and why.
    pub fn analysis_work(&self) -> Option<&EditStats> {
        self.analysis.as_ref().map(|kept| kept.session.stats())
    }

    /// Whether there is an edit to undo, and one to do again.
    pub fn can_undo(&self) -> (bool, bool) {
        (!self.undo.is_empty(), !self.redo.is_empty())
    }

    /// Takes back the last edit. Returns whether there was one.
    pub fn undo(&mut self) -> bool {
        self.step(true)
    }

    /// Does again the edit last taken back. Returns whether there was one.
    pub fn redo(&mut self) -> bool {
        self.step(false)
    }

    fn step(&mut self, back: bool) -> bool {
        let popped = if back {
            self.undo.pop()
        } else {
            self.redo.pop()
        };
        let Some(inverse) = popped else {
            return false;
        };
        match self.document.revert(inverse) {
            Ok(committed) => {
                if back {
                    self.redo.push(committed.inverse);
                } else {
                    self.undo.push(committed.inverse);
                }
                self.follow(&committed.change_set);
                true
            }
            // Every inverse was made from this document, so it always applies; a
            // history that no longer fits is dropped rather than half applied.
            Err(_) => {
                self.undo.clear();
                self.redo.clear();
                false
            }
        }
    }

    /// Moves nodes: writes where they are into the document's layout (`editor.nodes`),
    /// which the compiler never reads. Only places that changed are written, as one
    /// edit that can be undone. Returns whether anything moved.
    pub fn place(&mut self, places: &[(NodeId, [f32; 2])]) -> Result<bool, HostError> {
        let kept = self.document.editor().get("nodes");
        let moved: Vec<DocCommand> = places
            .iter()
            .filter(|(node, [x, y])| {
                let here = kept.and_then(|places| places.get(node.to_string()));
                let at = |axis: &str| here.and_then(|place| place[axis].as_f64());
                at("x") != Some(f64::from(x.round())) || at("y") != Some(f64::from(y.round()))
            })
            .map(|(node, at)| edit::place_command(*node, Some(*at)))
            .collect();
        if moved.is_empty() {
            return Ok(false);
        }
        self.commit(moved)
    }

    /// Applies `commands` as one edit that can be undone. An edit that changes nothing
    /// is not one: `Ok(false)`.
    pub(crate) fn commit(
        &mut self,
        commands: impl IntoIterator<Item = DocCommand>,
    ) -> Result<bool, HostError> {
        match self.document.apply(commands) {
            Ok(committed) => {
                self.record(committed);
                Ok(true)
            }
            Err(TxError::Empty) => Ok(false),
            Err(error) => Err(refused(&error)),
        }
    }

    fn record(&mut self, committed: Committed) {
        self.undo.push(committed.inverse);
        if self.undo.len() > MAX_UNDO {
            let _ = self.undo.remove(0);
        }
        self.redo.clear();
        self.follow(&committed.change_set);
    }

    fn follow(&mut self, change: &ChangeSet) {
        if let Some(analysis) = &mut self.analysis {
            analysis.follow(&self.document, change);
        }
    }

    /// Applies `commands` as a trial that [`Draft::withdraw`] can take back with the
    /// history exactly as it was, redo included.
    pub(crate) fn try_commit(
        &mut self,
        commands: impl IntoIterator<Item = DocCommand>,
    ) -> Result<Option<Vec<Inverse>>, HostError> {
        let redo = self.redo.clone();
        Ok(self.commit(commands)?.then_some(redo))
    }

    /// Takes back a trial from [`Draft::try_commit`] and restores the redo history it
    /// cleared: for an operation that finds its result is not allowed.
    pub(crate) fn withdraw(&mut self, redo: Vec<Inverse>) {
        if self.undo() {
            self.redo = redo;
        }
    }

    /// An ID that nothing in the document uses yet, as UUID version 4 text. It comes
    /// from the document's identity and revision, so the same edits of the same document
    /// give the same IDs: edits repeat exactly in tests.
    pub(crate) fn fresh_id(&self, what: &str) -> String {
        let mut attempt = 0u32;
        loop {
            let mut hasher = Sha256::new();
            hasher.update(self.document.graph_id().to_string().as_bytes());
            hasher.update(self.document.revision().to_le_bytes());
            hasher.update(what.as_bytes());
            hasher.update(attempt.to_le_bytes());
            let id = uuid_text(&hasher.finalize());
            if !self.uses(&id) {
                return id;
            }
            attempt += 1;
        }
    }

    fn uses(&self, id: &str) -> bool {
        let document = &self.document;
        let Ok(uuid) = id.parse::<uuid::Uuid>() else {
            return false;
        };
        let text = uuid.to_string();
        let bodies = std::iter::once(document.root())
            .chain(document.functions().values().map(|function| &function.body));
        bodies.into_iter().any(|body| {
            body.nodes.keys().any(|node| node.to_string() == text)
                || body.edges.keys().any(|edge| edge.to_string() == text)
        }) || document.variables().keys().any(|v| v.to_string() == text)
            || document.connections().keys().any(|c| c.to_string() == text)
            || document.events().keys().any(|e| e.to_string() == text)
            || document.functions().keys().any(|f| f.to_string() == text)
    }
}

/// The first 16 bytes of `digest` as UUID version 4 text.
pub(crate) fn uuid_text(digest: &[u8]) -> String {
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(digest.get(..16).unwrap_or(&[0; 16]));
    // Version 4, variant 1: the form every ID of a document has.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

pub(crate) fn refused(error: &dyn std::fmt::Display) -> HostError {
    HostError::Document(error.to_string())
}

/// What the host knows about package nodes used inside functions: none of them writes
/// a graph variable. Package code, whether a component or a native service, has no
/// access to a run's variables, so this is a fact about the execution boundary, not a
/// claim taken from the package.
pub(crate) fn leaf_effects(document: &GraphDocument) -> BTreeMap<NodeId, LeafEffects> {
    document
        .functions()
        .values()
        .flat_map(|function| function.body.nodes.values())
        .filter(|node| !node.definition.id.is_core())
        .map(|node| (node.id, LeafEffects::Known(Vec::new())))
        .collect()
}

impl Host {
    /// Starts editing a document as it is written. A document written by hand can be
    /// completed with [`Host::bind`] first; until then its problems say what it lacks.
    pub fn draft(&self, json: &str) -> Result<Draft, HostError> {
        Ok(Draft::new(
            from_json(json).map_err(|error| refused(&error))?,
        ))
    }

    /// What stands in the way of running the draft, as [`Host::check`] reports it for
    /// text. The analysis is kept with the draft and follows its edits: only what an
    /// edit touched is checked again, unless packages changed or graph-implemented
    /// nodes have to be linked in first.
    pub fn problems(&self, draft: &mut Draft) -> Result<Vec<Diagnostic>, HostError> {
        let registry = self.store.current();
        if !self.graphs.is_empty() {
            // Linked graphs rewrite the document before it is checked, so there is no
            // edit of the linked form to follow.
            draft.analysis = None;
            return self.check(&draft.text()?);
        }
        // Edits were followed as they were made; only new packages start over.
        let current = draft
            .analysis
            .as_ref()
            .is_some_and(|kept| kept.registry.generation() == registry.generation());
        if !current {
            draft.analysis = Some(Analysis::new(&draft.document, registry));
        }
        let Some(kept) = &draft.analysis else {
            return Ok(Vec::new());
        };
        let analysis = kept.session.analysis();
        Ok(
            if analysis.is_ready_for_lowering(&draft.document, registry) {
                Vec::new()
            } else {
                analysis.report().diagnostics.clone()
            },
        )
    }
}
