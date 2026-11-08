use super::*;
use orange_model::Value;
use orange_model::document::{NodeInstance, SignatureSnapshot};
use orange_model::id::{GraphId, NodeId, PortKey};
use std::collections::BTreeMap;
use std::io::{self, Write};

struct Header {
    graph: GraphId,
    registry: u64,
    sites: Vec<PureCallSite>,
}
struct Entry {
    node: NodeInstance,
    signature: SignatureSnapshot,
    inputs: Box<[Value]>,
    input_ports: Box<[PortKey]>,
    output_ports: Box<[PortKey]>,
    outcome: Result<Box<[Value]>, KernelFault>,
    bytes: usize,
    seen: u64,
}
pub(crate) struct FoldKey<'a> {
    pub scope: usize,
    pub node: &'a NodeInstance,
    pub signature: &'a SignatureSnapshot,
    pub input_bytes: Option<usize>,
}
pub(crate) struct Cache {
    limits: PreviewLimits,
    header: Option<Header>,
    entries: BTreeMap<(usize, NodeId), Entry>,
    bytes: usize,
    stats: PreviewStats,
}
impl Cache {
    pub fn new(limits: PreviewLimits) -> Self {
        Self {
            limits,
            header: None,
            entries: BTreeMap::new(),
            bytes: 0,
            stats: PreviewStats {
                request: 0,
                folded: 0,
                reused: 0,
                uncached: 0,
                entries: 0,
                retained_bytes: 0,
                reset: PreviewReset::First,
                published: false,
            },
        }
    }
    pub fn stats(&self) -> PreviewStats {
        PreviewStats {
            entries: self.entries.len(),
            retained_bytes: self.bytes,
            ..self.stats
        }
    }
    fn clear(&mut self) {
        self.entries.clear();
        self.header = None;
        self.bytes = 0;
    }
    pub fn start_request(&mut self, request: u64, interrupted: bool) {
        self.stats = PreviewStats {
            request,
            folded: 0,
            reused: 0,
            uncached: 0,
            entries: 0,
            retained_bytes: 0,
            reset: PreviewReset::None,
            published: false,
        };
        if interrupted {
            self.clear();
            self.stats.reset = PreviewReset::Interrupted;
        }
    }
    pub fn prepare(&mut self, graph: GraphId, registry: u64, sites: &[PureCallSite]) {
        let reset = match &self.header {
            None => PreviewReset::First,
            Some(h) if h.graph != graph => PreviewReset::GraphChanged,
            Some(h) if h.registry != registry => PreviewReset::RegistryChanged,
            Some(h) if h.sites != sites => PreviewReset::CallLayoutChanged,
            _ => return,
        };
        self.clear();
        if self.stats.reset != PreviewReset::Interrupted {
            self.stats.reset = reset;
        }
        let Some(bytes) = sites
            .len()
            .checked_mul(std::mem::size_of::<PureCallSite>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Header>()))
        else {
            return;
        };
        if self.limits.max_entries == 0 || bytes > self.limits.max_bytes {
            return;
        }
        self.header = Some(Header {
            graph,
            registry,
            sites: sites.to_vec(),
        });
        self.bytes = bytes;
    }
    pub fn finish(&mut self, published: bool) {
        self.stats.published = published;
        if !published {
            self.clear();
            return;
        }
        let request = self.stats.request;
        let mut removed = 0;
        self.entries.retain(|_, e| {
            if e.seen == request {
                true
            } else {
                removed += e.bytes;
                false
            }
        });
        self.bytes -= removed;
    }
    pub fn fold(
        &mut self,
        key: FoldKey<'_>,
        binding: &KernelBinding,
        inputs: &[Value],
        outputs: &mut [Value],
        shapes: &[ValueShape],
        limit: usize,
    ) -> Result<Result<(), KernelFault>, CompileError> {
        let index = (key.scope, key.node.id);
        if let Some(entry) = self.entries.get_mut(&index)
            && entry.node == *key.node
            && entry.signature == *key.signature
            && entry.input_ports == binding.inputs
            && entry.output_ports == binding.outputs
            && entry.inputs.as_ref() == inputs
        {
            entry.seen = self.stats.request;
            self.stats.reused += 1;
            return Ok(match &entry.outcome {
                Ok(values) => {
                    outputs.clone_from_slice(values);
                    Ok(())
                }
                Err(fault) => Err(*fault),
            });
        }
        self.stats.folded += 1;
        let outcome = binding.kernel.fold(inputs, outputs);
        let mut output_bytes = Some(0usize);
        match outcome {
            Err(fault) if fault.is_valid() => {
                output_bytes = Some(std::mem::size_of::<KernelFault>())
            }
            Err(_) => return Err(CompileError::KernelContract { node: key.node.id }),
            Ok(()) => {
                if shapes.len() != outputs.len() {
                    return Err(CompileError::KernelContract { node: key.node.id });
                }
                for (shape, value) in shapes.iter().zip(outputs.iter()) {
                    let bytes = shape.checked_charge(value, limit).map_err(|e| match e {
                        ValueCheckError::ResourceLimit => CompileError::ResourceLimit,
                        _ => CompileError::KernelContract { node: key.node.id },
                    })?;
                    output_bytes = output_bytes.and_then(|n| n.checked_add(bytes));
                }
            }
        }
        let bytes = key
            .input_bytes
            .zip(output_bytes)
            .and_then(|(a, b)| a.checked_add(b))
            .and_then(|values| {
                metadata_bytes(&key, binding, self.limits.max_bytes)
                    .and_then(|meta| values.checked_add(meta))
            })
            .and_then(|n| n.checked_add(std::mem::size_of::<Entry>()));
        let old = self.entries.get(&index).map_or(0, |e| e.bytes);
        if self.header.is_none()
            || (old == 0 && self.entries.len() >= self.limits.max_entries)
            || bytes.is_none_or(|n| {
                self.bytes
                    .checked_sub(old)
                    .and_then(|base| base.checked_add(n))
                    .is_none_or(|total| total > self.limits.max_bytes)
            })
        {
            self.stats.uncached += 1;
            return Ok(outcome);
        }
        let bytes = bytes.ok_or(CompileError::ResourceLimit)?;
        self.entries.insert(
            index,
            Entry {
                node: key.node.clone(),
                signature: key.signature.clone(),
                inputs: inputs.into(),
                input_ports: binding.inputs.clone(),
                output_ports: binding.outputs.clone(),
                outcome: outcome.map(|()| outputs.to_vec().into_boxed_slice()),
                bytes,
                seen: self.stats.request,
            },
        );
        self.bytes = self.bytes - old + bytes;
        Ok(outcome)
    }
}
/// Counts bounded serialized metadata without constructing a temporary JSON value
/// or exposing it to logging. Nominal metadata is distinct from allocator/RSS cost.
fn metadata_bytes(key: &FoldKey<'_>, binding: &KernelBinding, max: usize) -> Option<usize> {
    let mut counter = Counter { bytes: 0, max };
    serde_json::to_writer(&mut counter, key.node).ok()?;
    serde_json::to_writer(&mut counter, key.signature).ok()?;
    serde_json::to_writer(&mut counter, &binding.inputs).ok()?;
    serde_json::to_writer(&mut counter, &binding.outputs).ok()?;
    Some(counter.bytes)
}
struct Counter {
    bytes: usize,
    max: usize,
}
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|n| *n <= self.max)
            .ok_or_else(|| io::Error::other("Preview metadata exceeds its bound."))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
