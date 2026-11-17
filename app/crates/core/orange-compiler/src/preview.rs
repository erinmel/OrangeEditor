//! Effect-free authoring exports with an opportunistic bounded fold cache.
use crate::*;
use orange_model::{GraphDocument, catalog::RegistrySnapshot, validation::ValidationAnalysis};
use std::sync::Arc;
mod cache;
pub(crate) use cache::{Cache, FoldKey};

/// Separate nominal cache limits; the compiled publication retains its normal
/// CompileLimits. A disabled/full cache changes work, never graph outcomes.
#[derive(Debug, Clone, Copy)]
pub struct PreviewLimits {
    pub max_entries: usize,
    pub max_bytes: usize,
}
impl Default for PreviewLimits {
    fn default() -> Self {
        Self {
            max_entries: 20_000,
            max_bytes: 16 * 1024 * 1024,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewReset {
    None,
    First,
    GraphChanged,
    RegistryChanged,
    CallLayoutChanged,
    Interrupted,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewStats {
    pub request: u64,
    /// Actual pure-kernel invocations, including compiler-owned lazy projections.
    pub folded: usize,
    pub reused: usize,
    /// Valid computations that could not be retained under the cache limits.
    pub uncached: usize,
    pub entries: usize,
    pub retained_bytes: usize,
    pub reset: PreviewReset,
    pub published: bool,
}
/// Immutable typed export publication. It has no Run or service executor.
#[derive(Debug)]
pub struct PreviewSnapshot {
    plan: PurePlan,
    request: u64,
}
impl PreviewSnapshot {
    pub fn request(&self) -> u64 {
        self.request
    }
    pub fn is_current(&self, document: &GraphDocument, registry: &RegistrySnapshot) -> bool {
        self.plan.is_current(document, registry)
    }
    pub fn evaluate<'a>(
        &'a self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        options: EvaluationOptions<'a>,
    ) -> Result<PureEvaluation<'a>, EvaluationError> {
        self.plan.evaluate(document, registry, options)
    }
    pub fn diagnostics(&self) -> &[orange_model::validation::Diagnostic] {
        self.plan.diagnostics()
    }
}
/// Owns the resolver and authoring cache; updates never mutate an existing Run,
/// plan, provider binding or previously returned immutable snapshot. The resolver
/// must keep each locked provider's semantics immutable across updates.
pub struct PurePreview<R: PureKernelResolver> {
    resolver: R,
    cache: Cache,
    published: Option<Arc<PreviewSnapshot>>,
    request: u64,
    open: bool,
}
impl<R: PureKernelResolver> std::fmt::Debug for PurePreview<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PurePreview")
            .field("stats", &self.stats())
            .field("interrupted", &self.open)
            .finish()
    }
}
impl<R: PureKernelResolver> PurePreview<R> {
    pub fn new(resolver: R, limits: PreviewLimits) -> Self {
        Self {
            resolver,
            cache: Cache::new(limits),
            published: None,
            request: 0,
            open: false,
        }
    }
    pub fn snapshot(&self) -> Option<&Arc<PreviewSnapshot>> {
        self.published.as_ref()
    }
    pub fn stats(&self) -> PreviewStats {
        self.cache.stats()
    }
    /// Fresh analysis and normal whole-cone admission are always required. Only
    /// exact semantic/input matches skip a fold; cached values grant no authority
    /// to bypass validation, type/provider binding, cancellation or quotas.
    pub fn update(
        &mut self,
        document: &GraphDocument,
        registry: Arc<RegistrySnapshot>,
        analysis: &ValidationAnalysis,
        options: CompileOptions<'_>,
    ) -> Result<Arc<PreviewSnapshot>, CompileError> {
        self.published = None;
        self.request = self
            .request
            .checked_add(1)
            .ok_or(CompileError::ResourceLimit)?;
        let interrupted = self.open;
        self.cache.start_request(self.request, interrupted);
        self.open = true;
        let result = crate::compile::compile_preview(
            document,
            registry,
            analysis,
            &self.resolver,
            options,
            &mut self.cache,
        );
        self.open = false;
        match result {
            Ok(plan) => {
                self.cache.finish(true);
                let snapshot = Arc::new(PreviewSnapshot {
                    plan,
                    request: self.request,
                });
                self.published = Some(snapshot.clone());
                Ok(snapshot)
            }
            Err(error) => {
                self.cache.finish(false);
                Err(error)
            }
        }
    }
}
