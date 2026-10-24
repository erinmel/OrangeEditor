//! Headless execution primitives. This initial subset owns deterministic entropy
//! and consumption epochs, not general actors, graph execution or host services.

mod entropy;
pub use entropy::{
    ActivationEntropy, ENTROPY_ALGORITHM, EntropyError, EntropyKey, EntropyLayout, EntropySlot,
    EntropyStream,
};
