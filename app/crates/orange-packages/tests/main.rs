//! The crate's integration tests as one binary: one link instead of one per file, and
//! every module's tests share the harness's thread pool. Each file is a module.

mod common;

mod defaults;
mod lifecycle;
mod recovery;
mod redaction;
mod retention;
mod specializations;
