//! The flows a terminal run has open.
use crate::*;

/// One open flow: its number in the host, the file it came from and its names.
#[derive(Debug)]
pub(crate) struct Opened {
    pub flow: FlowId,
    pub path: String,
    pub names: Names,
}
