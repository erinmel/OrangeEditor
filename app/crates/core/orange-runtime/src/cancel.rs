use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Owned cooperative stop request shared by a host and the activations it starts.
/// Clones observe one flag, so an owner can store it without borrowing the host.
/// This is a flat signal, not the proposed run cancellation tree.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);
impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    /// Withdraw the request. Already rejected scopes still need an explicit restart.
    pub fn clear(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
    // The flag communicates a stop request only, not related memory publication.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    /// Borrowed view for synchronous APIs such as compile or analysis options.
    pub fn flag(&self) -> &AtomicBool {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_one_request_and_the_borrowed_flag_sees_it() {
        let token = CancelToken::new();
        let held = token.clone();
        assert!(!held.is_cancelled());
        token.cancel();
        assert!(held.is_cancelled());
        assert!(held.flag().load(Ordering::Relaxed));
        held.clear();
        assert!(!token.is_cancelled());
    }
}
