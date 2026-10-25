//! Checked root data evaluation with deterministic entropy and explicit epochs.
//! General run actors, control/functions and host services remain separate slices.

mod cancel;
mod data;
mod entropy;
mod run;
pub use cancel::CancelToken;
pub use data::{DataActivation, DataError, DataLimits, DataOptions};
pub use entropy::{
    ActivationEntropy, ENTROPY_ALGORITHM, EntropyError, EntropyKey, EntropyLayout,
    EntropyReplayMetadata, EntropySlot, EntropyStream, FrameId,
};
pub use run::{
    ControlOutcome, Run, RunError, RunEvent, RunLimits, RunOptions, RunProgress, RunState,
};
