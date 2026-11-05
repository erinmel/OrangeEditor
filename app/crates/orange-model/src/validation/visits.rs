//! Per-update validation work counters for an [`AnalysisSession`](super::AnalysisSession)
//! (NFR-15, 16 §3).
//!
//! A scoped update counts the items each pass actually iterates: plain counters bumped in
//! the loops and walks themselves, with no logging, allocation or global state. A full
//! analysis is not instrumented per pass; every pass is charged the whole document, which
//! bounds what cache hits save ([`LocalWork`](super::LocalWork) reports those).

/// The items one validation pass iterated during one session update.
///
/// - `nodes`: node items the pass took from a list, set, index lookup or walk stack and
///   processed (resolved, checked, projected, compared or expanded).
/// - `edges`: edges the pass read, including incident edges it scanned and skipped.
/// - `ports`: port descriptors the pass looked up, projected or compared.
/// - `diagnostics`: published diagnostics the pass scanned.
/// - `bodies`: graph bodies (the root or a function's) the pass ran in, once per run in
///   each body.
///
/// These measure work, not distinct units: an item a pass iterates twice counts twice.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct VisitCounts {
    pub nodes: usize,
    pub edges: usize,
    pub ports: usize,
    pub diagnostics: usize,
    pub bodies: usize,
}

impl VisitCounts {
    /// No visits.
    pub const ZERO: Self = Self {
        nodes: 0,
        edges: 0,
        ports: 0,
        diagnostics: 0,
        bodies: 0,
    };

    /// Whether the pass visited nothing.
    pub fn is_zero(&self) -> bool {
        *self == Self::ZERO
    }

    /// Every body, node and edge of `document`, with `ports` port descriptors: what a pass
    /// that iterates every body's nodes and edges once reads.
    pub(super) fn whole(document: &crate::document::GraphDocument, ports: usize) -> Self {
        let mut whole = Self {
            ports,
            ..Self::ZERO
        };
        for body in document.bodies() {
            whole.add_body(body);
        }
        whole
    }

    /// One body with its nodes and edges, as a pass iterating both lists once reads it.
    pub(super) fn add_body(&mut self, body: &crate::document::GraphBody) {
        self.nodes += body.nodes.len();
        self.edges += body.edges.len();
        self.bodies += 1;
    }
}

impl std::ops::Add for VisitCounts {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            nodes: self.nodes + other.nodes,
            edges: self.edges + other.edges,
            ports: self.ports + other.ports,
            diagnostics: self.diagnostics + other.diagnostics,
            bodies: self.bodies + other.bodies,
        }
    }
}

impl std::ops::AddAssign for VisitCounts {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

/// What each validation pass iterated during one session update, reported in
/// [`EditStats::visits`](super::EditStats::visits).
///
/// A layout-only or unchanged update visits nothing. A scoped update counts the items its
/// passes iterate, inside their loops, including the passes it reruns over a whole set (the
/// state pass, a function check's body passes); index maintenance and patching the
/// published report are bookkeeping, not passes, and are not counted. A full analysis
/// (`SessionWork::Recompute` or `Rebuild`) is not instrumented: every pass is charged every
/// body, node and edge of the document and every resolved port.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PassVisits {
    /// Node resolution and type inference: contract preflight, the generic-component and
    /// kept-solution searches, the incomplete-neighbour scan, projection, the comparison
    /// with the retained result and the neighbour walks that choose the nodes.
    pub resolution: VisitCounts,
    /// Node input checks.
    pub inputs: VisitCounts,
    /// Node configuration checks.
    pub config: VisitCounts,
    /// Connection checks of added and removed edges: the published-diagnostic scan, both
    /// endpoint ports and the fan-in and fan-out scans.
    pub connections: VisitCounts,
    /// The data dependency order (build, two-way search and cycle region), data cycles,
    /// downstream and export cone walks.
    pub order: VisitCounts,
    /// Availability: requirement cone walks, lazy readers, the subset edge walk and the
    /// subset availability pass.
    pub availability: VisitCounts,
    /// The exec component walk and the region and control-cycle passes over it.
    pub control_flow: VisitCounts,
    /// The state pass.
    pub state: VisitCounts,
    /// Zone body re-checks.
    pub zones: VisitCounts,
    /// Function acceptance and summary checks, call-graph and call-site scans.
    pub functions: VisitCounts,
}

impl PassVisits {
    /// No visits.
    pub const ZERO: Self = Self {
        resolution: VisitCounts::ZERO,
        inputs: VisitCounts::ZERO,
        config: VisitCounts::ZERO,
        connections: VisitCounts::ZERO,
        order: VisitCounts::ZERO,
        availability: VisitCounts::ZERO,
        control_flow: VisitCounts::ZERO,
        state: VisitCounts::ZERO,
        zones: VisitCounts::ZERO,
        functions: VisitCounts::ZERO,
    };

    /// The update's work summed over every pass, field by field: an item, or a body, that
    /// several passes iterate counts once per pass.
    pub fn total(&self) -> VisitCounts {
        self.each()
            .into_iter()
            .fold(VisitCounts::ZERO, |sum, pass| sum + pass)
    }

    fn each(&self) -> [VisitCounts; 10] {
        [
            self.resolution,
            self.inputs,
            self.config,
            self.connections,
            self.order,
            self.availability,
            self.control_flow,
            self.state,
            self.zones,
            self.functions,
        ]
    }

    /// Every pass charged `whole`, as a full analysis runs each over the document.
    pub(super) fn full(whole: VisitCounts) -> Self {
        Self {
            resolution: whole,
            inputs: whole,
            config: whole,
            connections: whole,
            order: whole,
            availability: whole,
            control_flow: whole,
            state: whole,
            zones: whole,
            functions: whole,
        }
    }
}
