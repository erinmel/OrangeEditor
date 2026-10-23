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

fn array_index(part: &str) -> Option<usize> {
    if part.is_empty() || (part.len() > 1 && part.starts_with('0')) {
        return None;
    }
    part.parse().ok()
}
