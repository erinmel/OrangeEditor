//! Checked data epochs and host-driven structured control with virtual time.
//!
//! Current profile: `DataActivation` evaluates checked data plans, iteration zones
//! included. `Run` owns bounded scheduling, local/run cells, invocation frames for
//! graph-local Action/Async calls and staged stop/disposal for the admitted control
//! subset, including Fork joins, typed collection iteration, atomic state and bounded
//! run-scoped Queue, busy Drop, serial latest-only Replace and bounded Parallel handlers.
//! Runs own authenticated graph resource aliases and close each resource once under
//! region/run ownership. Runtime Watch, trusted effects and host pull streams are
//! supported, including graph stream iteration and bounded Tee/push bridges. Reference-only
//! replay pins an exact plan and substitutes recorded value effects. Portable replay,
//! authoring preview and concrete services remain open.
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
    AcquireRefused, CloseReason, ControlOutcome, EffectError, EffectId, EffectLease, EffectRequest,
    EffectResourceRefused, EffectStreamRefused, EventMetadata, EventOrigin, EventTarget,
    HostSourceId, IngressError, PullStream, PushAdmission, PushStream, PushStreamSource,
    RecordedRun, ReplayArchive, ReplayArchiveError, ReplayError, ReplayLog, ReplayOptions,
    ReplayPayloadRef, ReplayPlayback, ReplayStats, ReplayedRun, ResourceError, ResourceHandle, Run,
    RunError, RunEvent, RunEventSource, RunEventStats, RunLimits, RunOptions, RunOwnership,
    RunProgress, RunResource, RunRetention, RunState, StopMode, StreamBranchStats, StreamBridge,
    StreamBridgeStats, StreamBufferOptions, StreamCollectPoll, StreamConsumer, StreamDemand,
    StreamError, StreamHandle, StreamNextId, StreamOverflow, StreamRead, StreamRefused, StreamStep,
    TeeStreams,
};
