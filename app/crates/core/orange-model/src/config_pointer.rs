//! RFC 6901 JSON pointers into a node config.

use std::collections::VecDeque;

use serde_json::Value as Json;

use crate::document::ConfigValue;

pub(crate) fn pointer_parts(pointer: &str) -> Option<Vec<String>> {
    if !pointer.starts_with('/') {
        return None;
    }
    pointer
        .split('/')
        .skip(1)
        .map(|part| {
            let mut decoded = String::with_capacity(part.len());
            let mut chars = part.chars();
            while let Some(ch) = chars.next() {
                if ch != '~' {
                    decoded.push(ch);
                    continue;
                }
                match chars.next()? {
                    '0' => decoded.push('~'),
                    '1' => decoded.push('/'),
                    _ => return None,
                }
            }
            Some(decoded)
        })
        .collect()
}

pub(crate) fn pointer_get<'a>(config: &'a ConfigValue, pointer: &str) -> Option<&'a Json> {
    let mut parts = pointer_parts(pointer)?.into_iter();
    let mut value = config.get(&parts.next()?)?;
    for part in parts {
        value = match value {
            Json::Object(map) => map.get(&part)?,
            Json::Array(items) => items.get(array_index(&part)?)?,
            _ => return None,
        };
    }
    Some(value)
}

pub(crate) fn pointer_set(config: &mut ConfigValue, pointer: &str, replacement: Json) -> bool {
    let Some(mut parts) = pointer_parts(pointer).map(VecDeque::from) else {
        return false;
    };
    let Some(first) = parts.pop_front() else {
        return false;
    };
    let Some(root) = config.get_mut(&first) else {
        return false;
    };
    set_nested(root, &mut parts, replacement)
}

fn set_nested(value: &mut Json, parts: &mut VecDeque<String>, replacement: Json) -> bool {
    let Some(part) = parts.pop_front() else {
        *value = replacement;
        return true;
    };
    let Some(child) = (match value {
        Json::Object(map) => map.get_mut(&part),
        Json::Array(items) => array_index(&part).and_then(|index| items.get_mut(index)),
        _ => None,
    }) else {
        return false;
    };
    set_nested(child, parts, replacement)
}

/// An RFC 6901 §4 array index: `0` or a nonzero ASCII digit followed by ASCII digits.
/// `usize::from_str` alone would also accept a leading `+`.
fn array_index(part: &str) -> Option<usize> {
    if part.is_empty()
        || !part.bytes().all(|byte| byte.is_ascii_digit())
        || (part.len() > 1 && part.starts_with('0'))
    {
        return None;
    }
    part.parse().ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn config() -> ConfigValue {
        json!({"targets": ["a", "b"], "named": {"+0": "key", "0": "zero", "a/b~": "escaped"}})
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn array_tokens_follow_rfc_6901_and_object_keys_stay_literal() {
        let config = config();
        let get = |pointer: &str| pointer_get(&config, pointer);
        assert_eq!(get("/targets/0"), Some(&json!("a")));
        assert_eq!(get("/targets/1"), Some(&json!("b")));
        for noncanonical in ["/targets/01", "/targets/+0", "/targets/+1", "/targets/-"] {
            assert_eq!(get(noncanonical), None, "{noncanonical}");
        }
        assert_eq!(get("/targets/99999999999999999999999"), None);
        assert_eq!(get("/named/+0"), Some(&json!("key")));
        assert_eq!(get("/named/0"), Some(&json!("zero")));
        assert_eq!(get("/named/a~1b~0"), Some(&json!("escaped")));
        // serde_json's own pointer lookup agrees on every array token.
        let whole = Json::Object(config.clone());
        for token in ["0", "1", "01", "+0", "+1", "-"] {
            let pointer = format!("/targets/{token}");
            assert_eq!(get(&pointer), whole.pointer(&pointer), "{pointer}");
        }
    }

    #[test]
    fn a_noncanonical_array_token_is_never_rewritten() {
        let mut config = config();
        assert!(!pointer_set(&mut config, "/targets/+0", json!("x")));
        assert!(pointer_set(&mut config, "/named/+0", json!("x")));
        assert!(pointer_set(&mut config, "/targets/0", json!("y")));
        assert_eq!(
            Json::Object(config),
            json!({"targets": ["y", "b"], "named": {"+0": "x", "0": "zero", "a/b~": "escaped"}})
        );
    }
}
