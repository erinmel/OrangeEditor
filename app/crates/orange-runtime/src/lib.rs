//! Checked data epochs and host-driven structured control with virtual time.
//!
//! Current profile: `DataActivation` evaluates checked data plans, iteration zones
//! included. `Run` owns bounded scheduling, local/run cells, invocation frames for
//! graph-local Action/Async calls and staged stop/disposal for the admitted control
//! subset, including Fork joins, typed collection iteration and atomic state. Events and
//! general host resources and services remain open.

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
