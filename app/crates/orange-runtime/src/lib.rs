//! Checked data epochs and host-driven structured control with virtual time.
//!
//! Current profile: `DataActivation` evaluates checked data plans, iteration zones
//! included. `Run` owns bounded scheduling, local/run cells, invocation frames for
//! graph-local Action/Async calls and staged stop/disposal for the admitted control
//! subset, including Fork joins, typed collection iteration, atomic state and bounded
//! run-scoped Queue, busy Drop and bounded Parallel event handlers. Replace, general event
//! ingress/replay/reactivity, resources and services remain open.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

mod budget;
mod cancel;
mod data;
mod entropy;
mod run;
pub use cancel::CancelToken;
pub use data::{DataActivation, DataError, DataLimits, DataOptions, DataPoll};
pub use entropy::{
    ActivationEntropy, ENTROPY_ALGORITHM, EntropyError, EntropyKey, EntropyLayout,
    EntropyReplayMetadata, EntropySlot, EntropyStream, FrameId,
};
pub use run::{
    ControlOutcome, EventMetadata, Run, RunError, RunEvent, RunEventStats, RunLimits, RunOptions,
    RunOwnership, RunProgress, RunRetention, RunState, StopMode,
};
