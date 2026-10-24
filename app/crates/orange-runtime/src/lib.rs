//! Checked root data evaluation with deterministic entropy and explicit epochs.
//! General run actors, control/functions and host services remain separate slices.

mod data;
mod entropy;
pub use data::{DataActivation, DataError, DataLimits, DataOptions};
pub use entropy::{
    ActivationEntropy, ENTROPY_ALGORITHM, EntropyError, EntropyKey, EntropyLayout,
    EntropyReplayMetadata, EntropySlot, EntropyStream,
};
