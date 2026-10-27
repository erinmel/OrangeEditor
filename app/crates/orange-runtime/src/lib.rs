//! Checked data epochs and host-driven structured control with virtual time.
//! Run owns bounded scheduling, local/run cells and staged stop/disposal for the
//! admitted subset, including Fork joins, node state and atomic Core variable updates.
//! Action/async functions, events and general host resources remain open.

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
    ControlOutcome, Run, RunError, RunEvent, RunLimits, RunOptions, RunOwnership, RunProgress,
    RunState, StopMode,
};
