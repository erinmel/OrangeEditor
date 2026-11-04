//! Per-update validation work counters for an [`AnalysisSession`](super::AnalysisSession)
//! (NFR-15, 16 §3).
//!
//! A scoped update counts the units each pass actually reached: plain counters bumped where
//! the walks visit, with no logging, allocation or global state. A full analysis is not
//! instrumented per pass; every pass is charged the whole document, which bounds what
//! cache hits save ([`LocalWork`](super::LocalWork) reports those).

/// Units one validation pass visited during one session update.
///
/// `nodes` counts node visits (resolutions, checks or walk steps), `edges` the edges a pass
/// read or followed, `ports` the port descriptors it checked and `bodies` the graph bodies
/// it worked in. A unit reached twice by one pass may be counted twice: the counts measure
/// work, not distinct units.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct VisitCounts {
    pub nodes: usize,
    pub edges: usize,
    pub ports: usize,
    pub bodies: usize,
}

impl VisitCounts {
    /// No visits.
    pub const ZERO: Self = Self {
        nodes: 0,
        edges: 0,
        ports: 0,
        bodies: 0,
    };

    /// Whether the pass visited nothing.
    pub fn is_zero(&self) -> bool {
        *self == Self::ZERO
    }

    /// The whole document, as a full pass visits it.
    pub(super) fn whole(document: &crate::document::GraphDocument, ports: usize) -> Self {
        let (nodes, edges, bodies) =
            document
                .bodies()
                .fold((0, 0, 0), |(nodes, edges, bodies), body| {
                    (
                        nodes + body.nodes.len(),
                        edges + body.edges.len(),
                        bodies + 1,
                    )
                });
        Self {
            nodes,
            edges,
            ports,
            bodies,
        }
    }
}

impl std::ops::Add for VisitCounts {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            nodes: self.nodes + other.nodes,
            edges: self.edges + other.edges,
            ports: self.ports + other.ports,
            bodies: self.bodies + other.bodies,
        }
    }
}

impl std::ops::AddAssign for VisitCounts {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

/// What each validation pass visited during one session update, reported in
/// [`EditStats::visits`](super::EditStats::visits).
///
/// A layout-only or unchanged update visits nothing. A scoped update counts the units its
/// walks reached; index maintenance and publishing the patched report are bookkeeping, not
/// passes, and are not counted. A full analysis (`SessionWork::Recompute` or `Rebuild`)
/// charges every pass the whole document.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PassVisits {
    /// Node resolution and type inference: re-resolved nodes with their ports, and the
    /// nodes and edges the generic-component and kept-solution searches read.
    pub resolution: VisitCounts,
    /// Node input checks.
    pub inputs: VisitCounts,
    /// Node configuration checks.
    pub config: VisitCounts,
    /// Connection checks of added and removed edges, with both endpoint ports.
    pub connections: VisitCounts,
    /// The data dependency order, data cycles, downstream and export cone walks.
    pub order: VisitCounts,
    /// Availability: requirement cone walks, lazy readers and the consumers rechecked.
    pub availability: VisitCounts,
    /// Region and control-cycle passes, with the exec component they run over.
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

    /// Every pass's node, edge and port visits summed, with the most bodies any one pass
    /// worked in: passes share bodies, so summing those would count one body per pass.
    pub fn total(&self) -> VisitCounts {
        self.each()
            .into_iter()
            .fold(VisitCounts::ZERO, |sum, pass| VisitCounts {
                bodies: sum.bodies.max(pass.bodies),
                ..sum + pass
            })
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

    fn each_mut(&mut self) -> [&mut VisitCounts; 10] {
        [
            &mut self.resolution,
            &mut self.inputs,
            &mut self.config,
            &mut self.connections,
            &mut self.order,
            &mut self.availability,
            &mut self.control_flow,
            &mut self.state,
            &mut self.zones,
            &mut self.functions,
        ]
    }

    /// Every pass charged `whole`, as a full analysis runs each over the document.
    pub(super) fn full(whole: VisitCounts) -> Self {
        let mut visits = Self::ZERO;
        for pass in visits.each_mut() {
            *pass = whole;
        }
        visits
    }

    /// Charges `bodies` to each pass that visited something but recorded no body: the
    /// scoped passes work inside the edit's own bodies.
    pub(super) fn in_bodies(mut self, bodies: usize) -> Self {
        for pass in self.each_mut() {
            if pass.bodies == 0 && !pass.is_zero() {
                pass.bodies = bodies;
            }
        }
        self
    }
}
