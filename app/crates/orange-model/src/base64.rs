//! Strict standard base64 with padding (RFC 4648 §4), for `bytes` literals (12 §6.1).
//! Decoding accepts only the canonical encoding, so load→save is byte-stable.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let block = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let bits = u32::from_be_bytes([0, block[0], block[1], block[2]]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(bits >> (18 - 6 * i) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `None` unless `text` is the canonical encoding of some byte string.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let text = text.as_bytes();
    if !text.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let blocks = text.len() / 4;
    for (index, block) in text.chunks(4).enumerate() {
        let padding = block.iter().rev().take_while(|&&b| b == b'=').count();
        if padding > 2 || (padding > 0 && index + 1 != blocks) {
            return None;
        }
        let mut bits = 0u32;
        for &byte in &block[..4 - padding] {
            bits = bits << 6 | u32::from(sextet(byte)?);
        }
        bits <<= 6 * padding as u32;
        let decoded = bits.to_be_bytes();
        let kept = 3 - padding;
        // Canonical form: the bits dropped by padding must be zero.
        if decoded[1 + kept..].iter().any(|&b| b != 0) {
            return None;
        }
        out.extend_from_slice(&decoded[1..=kept]);
    }
    Some(out)
}

fn sextet(byte: u8) -> Option<u8> {
    ALPHABET.iter().position(|&b| b == byte).map(|p| p as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4648_vectors() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(plain.as_bytes()), encoded);
            assert_eq!(decode(encoded).unwrap(), plain.as_bytes());
        }
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&all)).unwrap(), all);
    }

    #[test]
    fn rejects_non_canonical_text() {
        for bad in [
            "Zg", "Zg=", "Zh==", "Zm9=", "Z===", "Zg==Zg==", "Zm9v\n", "Zm-v", "Zm_v",
        ] {
            assert!(decode(bad).is_none(), "{bad:?}");
        }
    }
}
