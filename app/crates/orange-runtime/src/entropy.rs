use std::collections::BTreeMap;
use std::sync::Arc;

use orange_model::id::NodeId;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Versioned replay identifier. This is deterministic graph entropy, not a secret source.
pub const ENTROPY_ALGORITHM: &str = "orange.entropy.sha256-counter@1";

/// Host-owned scope identity; invocation advances on actual demand, not draw count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntropyKey {
    pub seed: u64,
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

/// One activation's counters and epoch cache. Create a new owner per activation.
/// Epoch changes use generation stamps; bound sampling neither hashes UUIDs nor
/// allocates. This is one raw sample per node/epoch, not a graph scheduler.
#[derive(Debug)]
pub struct ActivationEntropy {
    layout: Arc<Layout>,
    seed: u64,
    sequence: u64,
    epoch: Option<u64>,
    nodes: Vec<NodeState>,
}
impl ActivationEntropy {
    pub fn replay_metadata(&self) -> EntropyReplayMetadata {
        EntropyReplayMetadata {
            algorithm: ENTROPY_ALGORITHM,
            seed: self.seed,
            activation_sequence: self.sequence,
        }
    }
    pub fn new(layout: &EntropyLayout, seed: u64, activation_sequence: u64) -> Self {
        Self {
            layout: layout.0.clone(),
            seed,
            sequence: activation_sequence,
            epoch: None,
            nodes: vec![
                NodeState {
                    next: Some(0),
                    memo: None
                };
                layout.len()
            ],
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
        let key = self.invocation_key(slot)?;
        if let Some((_, _, Some(value))) = self.nodes[slot.index].memo {
            return Ok(value);
        }
        let value = EntropyStream::new(key).next_u64()?;
        let epoch = self.epoch.ok_or(EntropyError::NoEpoch)?;
        self.nodes[slot.index].memo = Some((epoch, key.invocation, Some(value)));
        Ok(value)
    }
    /// Stable invocation identity within an epoch, independent of stream draw count.
    pub fn invocation_key(&mut self, slot: &EntropySlot) -> Result<EntropyKey, EntropyError> {
        if !Arc::ptr_eq(&self.layout, &slot.layout) {
            return Err(EntropyError::ForeignSlot);
        }
        let epoch = self.epoch.ok_or(EntropyError::NoEpoch)?;
        let state = &mut self.nodes[slot.index];
        if let Some((sample_epoch, invocation, _)) = state.memo
            && sample_epoch == epoch
        {
            return Ok(EntropyKey {
                seed: self.seed,
                node: slot.node(),
                activation_sequence: self.sequence,
                invocation,
            });
        }
        let invocation = state.next.ok_or(EntropyError::InvocationExhausted)?;
        let key = EntropyKey {
            seed: self.seed,
            node: slot.node(),
            activation_sequence: self.sequence,
            invocation,
        };
        state.next = invocation.checked_add(1);
        state.memo = Some((epoch, invocation, None));
        Ok(key)
    }
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
            node: node(n),
            activation_sequence: 0,
            invocation: 0,
        }
    }
    fn first(key: EntropyKey) -> u64 {
        EntropyStream::new(key).next_u64().unwrap()
    }

    #[test]
    fn sha256_counter_matches_independent_dotnet_known_vectors_across_blocks() {
        let expected = [
            0xfba761e9efd54e44,
            0x3538c8eb780ce4c0,
            0xd129fd2cda25af4b,
            0xf28797f93f3028f8,
            0x00f539c51f4148ac,
            0xe1bf14277991190e,
            0xa3a31329e719b298,
            0xea6f7f1fb872bed6,
        ];
        let mut stream = EntropyStream::new(key(1));
        for value in expected {
            assert_eq!(stream.next_u64(), Ok(value));
        }
        assert_eq!(stream.key(), key(1));
        assert_eq!(ENTROPY_ALGORITHM, "orange.entropy.sha256-counter@1");
    }

    #[test]
    fn final_counter_block_is_available_once_without_wraparound() {
        let mut stream = EntropyStream::new(EntropyKey {
            seed: u64::MAX,
            node: node(2),
            activation_sequence: u64::MAX,
            invocation: u64::MAX,
        });
        stream.next_block = Some(u64::MAX);
        for expected in [
            0x5574c35e75b83926,
            0x3243d5bef1ab163d,
            0x58b32849e808940a,
            0x4f825086a6918104,
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
    fn repeated_consumers_share_one_sample_and_skipped_nodes_do_not_advance() {
        let layout = EntropyLayout::new([node(1), node(2)], 2).unwrap();
        let a = layout.slot(node(1)).unwrap();
        let b = layout.slot(node(2)).unwrap();
        let mut activation = ActivationEntropy::new(&layout, 0, 0);
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
        let mut owner = ActivationEntropy::new(&layout, 0, 0);
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
        let mut left = ActivationEntropy::new(&a, 123, 456);
        let mut right = ActivationEntropy::new(&b, 123, 456);
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
        let mut owner = ActivationEntropy::new(&layout.clone(), 0, 0);
        drop(layout);
        owner.begin_epoch().unwrap();
        assert!(owner.sample(&slot).is_ok());
        let replacement = EntropyLayout::new([node(1)], 1).unwrap();
        let mut other = ActivationEntropy::new(&replacement, 0, 0);
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
        let mut owner = ActivationEntropy::new(&layout, 0, 0);
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
