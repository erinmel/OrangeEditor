use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::Arc;

use orange_model::id::NodeId;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Versioned replay identifier. This is deterministic graph entropy, not a secret source.
/// Version 2 adds the call-frame identity; version 1 streams are not reproduced.
pub const ENTROPY_ALGORITHM: &str = "orange.entropy.sha256-counter@2";
const FRAME_DERIVATION: &str = "orange.entropy.frame@1";

/// Call-path identity of one graph frame. The root frame is 32 zero bytes; a callee
/// frame hashes its caller frame and the call-site node, so the same body node
/// reached through different call paths draws from different streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameId([u8; 32]);
impl FrameId {
    pub const ROOT: Self = Self([0; 32]);
    pub fn call(self, call_site: NodeId) -> Self {
        let mut hash = Sha256::new();
        hash.update(FRAME_DERIVATION.as_bytes());
        hash.update([0]);
        hash.update(self.0);
        hash.update(call_site.as_uuid().as_bytes());
        Self(hash.finalize().into())
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Host-owned scope identity; invocation advances on actual demand, not draw count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntropyKey {
    pub seed: u64,
    pub frame: FrameId,
    pub node: NodeId,
    pub activation_sequence: u64,
    pub invocation: u64,
}
/// Entropy-specific replay fields. Plan/provider/ingestion/clock records are separate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntropyReplayMetadata {
    pub algorithm: &'static str,
    pub seed: u64,
    pub activation_sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum EntropyError {
    #[error("Entropy layout exceeds the host node limit.")]
    ResourceLimit,
    #[error("Entropy layout contains a repeated node identity.")]
    DuplicateNode,
    #[error("The entropy slot belongs to another layout.")]
    ForeignSlot,
    #[error("No consumption epoch has begun.")]
    NoEpoch,
    #[error("The consumption epoch counter is exhausted.")]
    EpochExhausted,
    #[error("The node invocation counter is exhausted.")]
    InvocationExhausted,
    #[error("The entropy stream counter is exhausted.")]
    StreamExhausted,
}
impl EntropyError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::ResourceLimit => "core/entropy-resource-limit",
            Self::DuplicateNode => "core/entropy-duplicate-node",
            Self::ForeignSlot => "core/entropy-foreign-slot",
            Self::NoEpoch => "core/entropy-no-epoch",
            Self::EpochExhausted => "core/entropy-epoch-exhausted",
            Self::InvocationExhausted => "core/entropy-invocation-exhausted",
            Self::StreamExhausted => "core/entropy-stream-exhausted",
        }
    }
}

/// Reproducible stream for one node invocation. Four little-endian u64 words are
/// read per SHA-256 block. No counter wraps; no mutable global generator exists.
#[derive(Debug)]
pub struct EntropyStream {
    key: EntropyKey,
    next_block: Option<u64>,
    words: [u64; 4],
    cursor: usize,
}
impl EntropyStream {
    pub fn new(key: EntropyKey) -> Self {
        Self {
            key,
            next_block: Some(0),
            words: [0; 4],
            cursor: 4,
        }
    }
    pub fn key(&self) -> EntropyKey {
        self.key
    }
    pub fn next_u64(&mut self) -> Result<u64, EntropyError> {
        if self.cursor == 4 {
            let block = self.next_block.ok_or(EntropyError::StreamExhausted)?;
            let mut hash = Sha256::new();
            hash.update(ENTROPY_ALGORITHM.as_bytes());
            hash.update([0]);
            hash.update(self.key.seed.to_le_bytes());
            hash.update(self.key.frame.0);
            hash.update(self.key.node.as_uuid().as_bytes());
            hash.update(self.key.activation_sequence.to_le_bytes());
            hash.update(self.key.invocation.to_le_bytes());
            hash.update(block.to_le_bytes());
            let digest = hash.finalize();
            for (word, bytes) in self.words.iter_mut().zip(digest.as_chunks::<8>().0) {
                *word = u64::from_le_bytes(*bytes);
            }
            self.next_block = block.checked_add(1);
            self.cursor = 0;
        }
        let word = self.words[self.cursor];
        self.cursor += 1;
        Ok(word)
    }
}

#[derive(Debug)]
struct Layout {
    nodes: Vec<NodeId>,
    indices: BTreeMap<NodeId, usize>,
}

/// Frozen entropy-node layout. UUID lookup occurs only while binding a slot.
/// Construction admits at most max_nodes entries before allocation can grow further.
#[derive(Debug, Clone)]
pub struct EntropyLayout(Arc<Layout>);
impl EntropyLayout {
    pub fn new(
        nodes: impl IntoIterator<Item = NodeId>,
        max_nodes: usize,
    ) -> Result<Self, EntropyError> {
        let mut layout = Layout {
            nodes: vec![],
            indices: BTreeMap::new(),
        };
        for node in nodes {
            if layout.nodes.len() >= max_nodes {
                return Err(EntropyError::ResourceLimit);
            }
            if layout.indices.insert(node, layout.nodes.len()).is_some() {
                return Err(EntropyError::DuplicateNode);
            }
            layout.nodes.push(node);
        }
        Ok(Self(Arc::new(layout)))
    }
    pub fn len(&self) -> usize {
        self.0.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.nodes.is_empty()
    }
    pub fn slot(&self, node: NodeId) -> Option<EntropySlot> {
        self.0.indices.get(&node).map(|index| EntropySlot {
            layout: self.0.clone(),
            index: *index,
        })
    }
}

/// Private construction retains the exact layout, including across later publication.
#[derive(Debug, Clone)]
pub struct EntropySlot {
    layout: Arc<Layout>,
    index: usize,
}
impl EntropySlot {
    pub fn node(&self) -> NodeId {
        self.layout.nodes[self.index]
    }
}

#[derive(Debug, Clone, Copy)]
struct NodeState {
    next: Option<u64>,
    memo: Option<(u64, u64, Option<u64>)>,
}
const FRESH: NodeState = NodeState {
    next: Some(0),
    memo: None,
};

/// One activation's counters and epoch cache. Create a new owner per activation.
/// Epoch changes use generation stamps; bound sampling in the root frame neither
/// hashes UUIDs nor allocates. Each callee frame gets its own counters on first
/// demand, up to max_frames. This is one raw sample per frame/node/epoch, not a
/// graph scheduler.
#[derive(Debug)]
pub struct ActivationEntropy {
    layout: Arc<Layout>,
    seed: u64,
    sequence: u64,
    epoch: Option<u64>,
    nodes: Vec<NodeState>,
    frames: BTreeMap<FrameId, Box<[NodeState]>>,
    max_frames: usize,
}
impl ActivationEntropy {
    pub fn replay_metadata(&self) -> EntropyReplayMetadata {
        EntropyReplayMetadata {
            algorithm: ENTROPY_ALGORITHM,
            seed: self.seed,
            activation_sequence: self.sequence,
        }
    }
    /// max_frames bounds callee frames; the root frame is always available.
    pub fn new(
        layout: &EntropyLayout,
        seed: u64,
        activation_sequence: u64,
        max_frames: usize,
    ) -> Self {
        Self::with_counters(layout, seed, activation_sequence, max_frames, layout.len())
    }
    // Expanded plans already assign one bounded dense counter to each actual
    // frame/node instruction. They do not need sparse whole-layout frame arrays.
    pub(crate) fn new_dense(
        layout: &EntropyLayout,
        seed: u64,
        sequence: u64,
        count: usize,
    ) -> Self {
        Self::with_counters(layout, seed, sequence, 0, count)
    }
    fn with_counters(
        layout: &EntropyLayout,
        seed: u64,
        activation_sequence: u64,
        max_frames: usize,
        count: usize,
    ) -> Self {
        Self {
            layout: layout.0.clone(),
            seed,
            sequence: activation_sequence,
            epoch: None,
            nodes: vec![FRESH; count],
            frames: BTreeMap::new(),
            max_frames,
        }
    }
    fn state(&mut self, frame: FrameId, index: usize) -> Result<&mut NodeState, EntropyError> {
        if frame == FrameId::ROOT {
            return Ok(&mut self.nodes[index]);
        }
        let full = self.frames.len() >= self.max_frames;
        let len = self.nodes.len();
        match self.frames.entry(frame) {
            Entry::Occupied(states) => Ok(&mut states.into_mut()[index]),
            Entry::Vacant(_) if full => Err(EntropyError::ResourceLimit),
            Entry::Vacant(states) => Ok(&mut states.insert(vec![FRESH; len].into())[index]),
        }
    }
    pub fn begin_epoch(&mut self) -> Result<u64, EntropyError> {
        let epoch = match self.epoch {
            None => 0,
            Some(epoch) => epoch.checked_add(1).ok_or(EntropyError::EpochExhausted)?,
        };
        self.epoch = Some(epoch);
        Ok(epoch)
    }
    pub fn sample(&mut self, slot: &EntropySlot) -> Result<u64, EntropyError> {
        self.sample_in(FrameId::ROOT, slot)
    }
    pub fn sample_in(&mut self, frame: FrameId, slot: &EntropySlot) -> Result<u64, EntropyError> {
        let key = self.invocation_key_in(frame, slot)?;
        let state = self.state(frame, slot.index)?;
        if let Some((_, _, Some(value))) = state.memo {
            return Ok(value);
        }
        let value = EntropyStream::new(key).next_u64()?;
        // invocation_key_in stamped this epoch; keep that stamp with the value.
        if let Some((_, _, sample)) = &mut state.memo {
            *sample = Some(value);
        }
        Ok(value)
    }
    /// Root-frame identity; see invocation_key_in.
    pub fn invocation_key(&mut self, slot: &EntropySlot) -> Result<EntropyKey, EntropyError> {
        self.invocation_key_in(FrameId::ROOT, slot)
    }
    /// Stable invocation identity within an epoch, independent of stream draw count.
    /// Invocation counters are per frame and node.
    pub fn invocation_key_in(
        &mut self,
        frame: FrameId,
        slot: &EntropySlot,
    ) -> Result<EntropyKey, EntropyError> {
        if !Arc::ptr_eq(&self.layout, &slot.layout) {
            return Err(EntropyError::ForeignSlot);
        }
        let epoch = self.epoch.ok_or(EntropyError::NoEpoch)?;
        let key = EntropyKey {
            seed: self.seed,
            frame,
            node: slot.node(),
            activation_sequence: self.sequence,
            invocation: 0,
        };
        let state = self.state(frame, slot.index)?;
        invocation(key, epoch, state)
    }
    pub(crate) fn invocation_key_dense(
        &mut self,
        frame: FrameId,
        slot: &EntropySlot,
        index: usize,
    ) -> Result<EntropyKey, EntropyError> {
        if !Arc::ptr_eq(&self.layout, &slot.layout) {
            return Err(EntropyError::ForeignSlot);
        }
        let epoch = self.epoch.ok_or(EntropyError::NoEpoch)?;
        let key = EntropyKey {
            seed: self.seed,
            frame,
            node: slot.node(),
            activation_sequence: self.sequence,
            invocation: 0,
        };
        let state = self
            .nodes
            .get_mut(index)
            .ok_or(EntropyError::ResourceLimit)?;
        invocation(key, epoch, state)
    }
}
fn invocation(
    mut key: EntropyKey,
    epoch: u64,
    state: &mut NodeState,
) -> Result<EntropyKey, EntropyError> {
    if let Some((sample_epoch, invocation, _)) = state.memo
        && sample_epoch == epoch
    {
        key.invocation = invocation;
        return Ok(key);
    }
    key.invocation = state.next.ok_or(EntropyError::InvocationExhausted)?;
    state.next = key.invocation.checked_add(1);
    state.memo = Some((epoch, key.invocation, None));
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(n: u8) -> NodeId {
        format!("10000000-0000-4000-8000-{n:012x}").parse().unwrap()
    }
    fn key(n: u8) -> EntropyKey {
        EntropyKey {
            seed: 0,
            frame: FrameId::ROOT,
            node: node(n),
            activation_sequence: 0,
            invocation: 0,
        }
    }
    fn first(key: EntropyKey) -> u64 {
        EntropyStream::new(key).next_u64().unwrap()
    }

    #[test]
    fn dense_expansion_matches_explicit_frames_without_sparse_frame_arrays() {
        let layout = EntropyLayout::new((1..=64).map(node), 64).unwrap();
        let mut dense = ActivationEntropy::new_dense(&layout, 42, 7, 128);
        let mut reference = ActivationEntropy::new(&layout, 42, 7, 2);
        let foreign = EntropyLayout::new([node(1)], 1)
            .unwrap()
            .slot(node(1))
            .unwrap();
        assert_eq!(
            dense.invocation_key_dense(FrameId::ROOT, &foreign, 0),
            Err(EntropyError::ForeignSlot)
        );
        for epoch in 0..3 {
            dense.begin_epoch().unwrap();
            reference.begin_epoch().unwrap();
            for (frame_index, frame) in
                [FrameId::ROOT.call(node(100)), FrameId::ROOT.call(node(101))]
                    .into_iter()
                    .enumerate()
            {
                for n in 1..=64 {
                    let slot = layout.slot(node(n)).unwrap();
                    let index = frame_index * 64 + usize::from(n - 1);
                    let key = dense.invocation_key_dense(frame, &slot, index).unwrap();
                    assert_eq!(key, reference.invocation_key_in(frame, &slot).unwrap());
                    assert_eq!(key.invocation, epoch);
                    assert_eq!(
                        key,
                        dense.invocation_key_dense(frame, &slot, index).unwrap()
                    );
                }
            }
            assert_eq!(dense.nodes.len(), 128);
            assert!(dense.frames.is_empty());
        }
    }

    #[test]
    fn sha256_counter_matches_independent_dotnet_known_vectors_across_blocks() {
        let expected = [
            0x217b7f7d1090c139,
            0xfa09ae9852b0c5ec,
            0x28a4088e78a239b1,
            0x8a11b8b40bbef07a,
            0x423e8f4ded336c5c,
            0x99471cddd1ee4bde,
            0xdff933d09e32f4de,
            0xf5d5fc4885966a8d,
        ];
        let mut stream = EntropyStream::new(key(1));
        for value in expected {
            assert_eq!(stream.next_u64(), Ok(value));
        }
        assert_eq!(stream.key(), key(1));
        assert_eq!(ENTROPY_ALGORITHM, "orange.entropy.sha256-counter@2");
    }

    #[test]
    fn final_counter_block_is_available_once_without_wraparound() {
        let mut stream = EntropyStream::new(EntropyKey {
            seed: u64::MAX,
            frame: FrameId::ROOT,
            node: node(2),
            activation_sequence: u64::MAX,
            invocation: u64::MAX,
        });
        stream.next_block = Some(u64::MAX);
        for expected in [
            0xf151454f85815bbf,
            0xe119f6842e69c571,
            0x69e86b0878e5ffe2,
            0x07a568da5042871d,
        ] {
            assert_eq!(stream.next_u64(), Ok(expected));
        }
        assert_eq!(stream.next_u64(), Err(EntropyError::StreamExhausted));
        assert_eq!(stream.next_u64(), Err(EntropyError::StreamExhausted));
    }

    #[test]
    fn every_scope_field_changes_the_recorded_stream_and_replay_is_identical() {
        let base = key(1);
        let values = [
            first(base),
            first(EntropyKey { seed: 1, ..base }),
            first(key(2)),
            first(EntropyKey {
                activation_sequence: 1,
                ..base
            }),
            first(EntropyKey {
                invocation: 1,
                ..base
            }),
            first(EntropyKey {
                frame: FrameId::ROOT.call(node(9)),
                ..base
            }),
        ];
        for (index, value) in values.iter().enumerate() {
            assert!(!values[..index].contains(value));
        }
        let mut a = EntropyStream::new(base);
        let mut b = EntropyStream::new(base);
        for _ in 0..33 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn frame_derivation_matches_independent_dotnet_vector_and_separates_call_paths() {
        let child = FrameId::ROOT.call(node(9));
        assert_eq!(
            child.as_bytes(),
            &[
                0x36, 0x4c, 0xa4, 0xce, 0x2c, 0xc1, 0xdc, 0x19, 0x7e, 0x47, 0x5f, 0xc1, 0x29, 0x91,
                0x50, 0x42, 0x3b, 0x62, 0xff, 0x42, 0x15, 0x38, 0x69, 0x86, 0x04, 0xcc, 0xa2, 0x6d,
                0x7c, 0x21, 0x65, 0x93,
            ]
        );
        assert_eq!(
            first(EntropyKey {
                frame: child,
                ..key(1)
            }),
            0xfd166ace36438f7e
        );
        // Order and depth both matter: a→b, b→a and a alone are distinct paths.
        let a = FrameId::ROOT.call(node(3));
        let b = FrameId::ROOT.call(node(4));
        let paths = [a, b, a.call(node(4)), b.call(node(3)), a.call(node(3))];
        for (index, path) in paths.iter().enumerate() {
            assert_ne!(*path, FrameId::ROOT);
            assert!(!paths[..index].contains(path));
        }
        assert_eq!(a.call(node(4)), FrameId::ROOT.call(node(3)).call(node(4)));
    }

    #[test]
    fn callee_frames_keep_separate_counters_and_a_bounded_frame_table() {
        let layout = EntropyLayout::new([node(1)], 1).unwrap();
        let slot = layout.slot(node(1)).unwrap();
        let (left, right) = (FrameId::ROOT.call(node(7)), FrameId::ROOT.call(node(8)));
        let mut owner = ActivationEntropy::new(&layout, 0, 0, 2);
        assert_eq!(owner.sample_in(left, &slot), Err(EntropyError::NoEpoch));
        assert!(owner.frames.is_empty());
        owner.begin_epoch().unwrap();
        // Two call sites of one body node in one epoch draw different samples.
        let in_left = owner.sample_in(left, &slot).unwrap();
        let in_right = owner.sample_in(right, &slot).unwrap();
        let in_root = owner.sample(&slot).unwrap();
        assert_eq!(
            in_left,
            first(EntropyKey {
                frame: left,
                ..key(1)
            })
        );
        assert_eq!(
            in_right,
            first(EntropyKey {
                frame: right,
                ..key(1)
            })
        );
        assert_eq!(in_root, first(key(1)));
        assert_eq!(owner.sample_in(left, &slot), Ok(in_left));
        // A third callee frame exceeds the table before allocating or drawing.
        let third = FrameId::ROOT.call(node(9));
        assert_eq!(
            owner.invocation_key_in(third, &slot),
            Err(EntropyError::ResourceLimit)
        );
        assert_eq!(owner.frames.len(), 2);
        // Only frames demanded in a later epoch advance their own invocation.
        owner.begin_epoch().unwrap();
        assert_eq!(owner.invocation_key_in(left, &slot).unwrap().invocation, 1);
        owner.begin_epoch().unwrap();
        assert_eq!(owner.invocation_key_in(right, &slot).unwrap().invocation, 1);
        assert_eq!(owner.invocation_key(&slot).unwrap().invocation, 1);
        // A foreign slot is rejected without creating counters for its frame.
        let other = EntropyLayout::new([node(1)], 1).unwrap();
        let mut fresh = ActivationEntropy::new(&layout, 0, 0, 2);
        fresh.begin_epoch().unwrap();
        assert_eq!(
            fresh.invocation_key_in(left, &other.slot(node(1)).unwrap()),
            Err(EntropyError::ForeignSlot)
        );
        assert!(fresh.frames.is_empty());
    }

    #[test]
    fn repeated_consumers_share_one_sample_and_skipped_nodes_do_not_advance() {
        let layout = EntropyLayout::new([node(1), node(2)], 2).unwrap();
        let a = layout.slot(node(1)).unwrap();
        let b = layout.slot(node(2)).unwrap();
        let mut activation = ActivationEntropy::new(&layout, 0, 0, 0);
        assert_eq!(activation.sample(&a), Err(EntropyError::NoEpoch));
        assert_eq!(activation.begin_epoch(), Ok(0));
        assert_eq!(activation.sample(&a), Ok(first(key(1))));
        assert_eq!(activation.sample(&a), Ok(first(key(1))));
        assert_eq!(activation.begin_epoch(), Ok(1));
        let later = first(EntropyKey {
            invocation: 1,
            ..key(1)
        });
        assert_eq!(activation.sample(&a), Ok(later));
        assert_eq!(activation.sample(&b), Ok(first(key(2))));
        assert_eq!(activation.nodes[0].next, Some(2));
        assert_eq!(activation.nodes[1].next, Some(1));
    }
    #[test]
    fn invocation_identity_is_shared_by_all_stream_words_in_one_epoch() {
        let layout = EntropyLayout::new([node(1)], 1).unwrap();
        let slot = layout.slot(node(1)).unwrap();
        let mut owner = ActivationEntropy::new(&layout, 0, 0, 0);
        owner.begin_epoch().unwrap();
        let first_key = owner.invocation_key(&slot).unwrap();
        let mut stream = EntropyStream::new(first_key);
        for _ in 0..8 {
            stream.next_u64().unwrap();
        }
        assert_eq!(owner.invocation_key(&slot), Ok(first_key));
        assert_eq!(owner.sample(&slot), Ok(first(first_key)));
        assert_eq!(owner.nodes[0].next, Some(1));
        owner.begin_epoch().unwrap();
        assert_eq!(owner.invocation_key(&slot).unwrap().invocation, 1);
    }

    #[test]
    fn layout_order_and_demand_order_do_not_change_node_stream_identity() {
        let a = EntropyLayout::new([node(1), node(2)], 2).unwrap();
        let b = EntropyLayout::new([node(2), node(1)], 2).unwrap();
        let mut left = ActivationEntropy::new(&a, 123, 456, 0);
        let mut right = ActivationEntropy::new(&b, 123, 456, 0);
        for _ in 0..3 {
            left.begin_epoch().unwrap();
            right.begin_epoch().unwrap();
            let l1 = left.sample(&a.slot(node(1)).unwrap()).unwrap();
            let l2 = left.sample(&a.slot(node(2)).unwrap()).unwrap();
            let r2 = right.sample(&b.slot(node(2)).unwrap()).unwrap();
            let r1 = right.sample(&b.slot(node(1)).unwrap()).unwrap();
            assert_eq!((l1, l2), (r1, r2));
        }
    }

    #[test]
    fn a_slot_retains_its_layout_and_cannot_access_another_activation_layout() {
        let layout = EntropyLayout::new([node(1)], 1).unwrap();
        let slot = layout.slot(node(1)).unwrap();
        assert_eq!(slot.node(), node(1));
        let mut owner = ActivationEntropy::new(&layout.clone(), 0, 0, 0);
        drop(layout);
        owner.begin_epoch().unwrap();
        assert!(owner.sample(&slot).is_ok());
        let replacement = EntropyLayout::new([node(1)], 1).unwrap();
        let mut other = ActivationEntropy::new(&replacement, 0, 0, 0);
        other.begin_epoch().unwrap();
        assert_eq!(other.sample(&slot), Err(EntropyError::ForeignSlot));
        assert_eq!(other.nodes[0].next, Some(0));
    }

    #[test]
    fn node_admission_is_bounded_and_duplicate_or_unknown_ids_are_explicit() {
        assert!(matches!(
            EntropyLayout::new([node(1)], 0),
            Err(EntropyError::ResourceLimit)
        ));
        assert!(matches!(
            EntropyLayout::new([node(1), node(1)], 2),
            Err(EntropyError::DuplicateNode)
        ));
        let empty = EntropyLayout::new([], 0).unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        assert!(empty.slot(node(1)).is_none());
    }

    #[test]
    fn activation_invocation_and_epoch_counters_fail_instead_of_reusing_identity() {
        let layout = EntropyLayout::new([node(1)], 1).unwrap();
        let slot = layout.slot(node(1)).unwrap();
        let mut owner = ActivationEntropy::new(&layout, 0, 0, 0);
        owner.begin_epoch().unwrap();
        owner.nodes[0].next = Some(u64::MAX);
        let last = first(EntropyKey {
            invocation: u64::MAX,
            ..key(1)
        });
        assert_eq!(owner.sample(&slot), Ok(last));
        assert_eq!(owner.sample(&slot), Ok(last));
        owner.begin_epoch().unwrap();
        assert_eq!(owner.sample(&slot), Err(EntropyError::InvocationExhausted));
        assert_eq!(owner.nodes[0].next, None);
        owner.epoch = Some(u64::MAX);
        assert_eq!(owner.begin_epoch(), Err(EntropyError::EpochExhausted));
        assert_eq!(owner.epoch, Some(u64::MAX));
        assert_eq!(
            EntropyError::EpochExhausted.message_key(),
            "core/entropy-epoch-exhausted"
        );
    }
}
