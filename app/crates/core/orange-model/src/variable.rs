//! Variable operation tokens shared by contextual contracts and function summaries.

use crate::{catalog::intrinsic_id, document::NodeInstance};

/// The operation fixed by a counter's definition or chosen on a general update.
pub fn update_token(node: &NodeInstance) -> Option<&str> {
    intrinsic_id::counter_operation(node.definition.id.as_str()).or_else(|| {
        node.config
            .get("operation")
            .and_then(serde_json::Value::as_str)
    })
}

/// The operation used by a variable write or function summary; later state analysis can
/// recognize documented commutative update cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WriteOperation {
    Set,
    Add,
    Sub,
    Min,
    Max,
    And,
    Or,
    Xor,
    Append,
    CompareAndSet,
}

impl WriteOperation {
    /// Parses one of the v1 `update-variable` operation tokens. `set` belongs to the
    /// separate `set-variable` intrinsic and is intentionally rejected here.
    pub(crate) fn parse_update(value: &str) -> Option<Self> {
        Some(match value {
            "add" => Self::Add,
            "sub" => Self::Sub,
            "min" => Self::Min,
            "max" => Self::Max,
            "and" => Self::And,
            "or" => Self::Or,
            "xor" => Self::Xor,
            "append" => Self::Append,
            "compare-and-set" => Self::CompareAndSet,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::WriteOperation;

    #[test]
    fn update_parser_recognizes_all_v1_update_tokens_and_rejects_set() {
        for (token, expected) in [
            ("add", WriteOperation::Add),
            ("sub", WriteOperation::Sub),
            ("min", WriteOperation::Min),
            ("max", WriteOperation::Max),
            ("and", WriteOperation::And),
            ("or", WriteOperation::Or),
            ("xor", WriteOperation::Xor),
            ("append", WriteOperation::Append),
            ("compare-and-set", WriteOperation::CompareAndSet),
        ] {
            assert_eq!(WriteOperation::parse_update(token), Some(expected));
        }
        for token in ["set", "ADD", "", "unknown"] {
            assert_eq!(WriteOperation::parse_update(token), None);
        }
    }
}
