//! The crate's integration tests as one binary: one link instead of one per file, and
//! every module's tests share the harness's thread pool. Each file is a module.

// Support shared by several modules is compiled once and named from the crate root.
#[path = "support/budget_drive.rs"]
#[allow(dead_code)]
mod budget_drive;
#[path = "support/data_graph.rs"]
#[allow(dead_code)]
mod data_graph;
#[path = "support/reference.rs"]
#[allow(dead_code)]
mod reference;
#[path = "support/seeded_provider.rs"]
#[allow(dead_code)]
mod seeded_provider;

mod analysis_session_embedding;
mod analysis_session_scoped;
mod analysis_session_visits;
mod binding_request;
mod bindings;
mod checkpoints;
mod comparisons;
mod compile_data;
mod compile_pure;
mod control;
mod conversions;
mod core_integrity;
mod data_epochs;
mod effects;
mod entropy;
mod enum_declarations;
mod function_epochs;
mod graph_handles;
mod graph_streams;
mod integers;
mod pure_preview;
mod record_declarations;
mod reference_equivalence;
mod replay;
mod run_events;
mod scalars;
mod seeded_equivalence;
mod stream_bridges;
mod streams;
mod third_party_package;
mod workspace_values;
mod zone_epochs;
