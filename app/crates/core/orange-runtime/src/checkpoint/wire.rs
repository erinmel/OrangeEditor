use super::*;
use serde::Deserializer;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::fmt;
use std::io::{self, Write};
struct Count<'a> {
    limit: usize,
    exceeded: &'a mut bool,
}
impl<'de> DeserializeSeed<'de> for Count<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for Count<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a bounded checkpoint value list")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let mut count = 0usize;
        while count < self.limit {
            if seq.next_element_seed(Discard)?.is_none() {
                return Ok(());
            }
            count += 1;
        }
        seq.next_element_seed(Exceeded(self.exceeded))?;
        Ok(())
    }
}
struct Exceeded<'a>(&'a mut bool);
impl<'de> DeserializeSeed<'de> for Exceeded<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, _: D) -> Result<(), D::Error> {
        *self.0 = true;
        Err(serde::de::Error::custom("checkpoint limit"))
    }
}
/// Structural traversal rejects ambiguous object keys without building a value tree.
struct Discard;
impl<'de> DeserializeSeed<'de> for Discard {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Discard {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unambiguous JSON")
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while seq.next_element_seed(Discard)?.is_some() {}
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let mut seen = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key) {
                return Err(serde::de::Error::custom("duplicate checkpoint key"));
            }
            map.next_value_seed(Discard)?;
        }
        Ok(())
    }
}
struct Preflight<'a>(Count<'a>);
impl<'de> Visitor<'de> for Preflight<'_> {
    type Value = (String, u32);
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a checkpoint document")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut seen = BTreeSet::new();
        let (mut format, mut version) = (None, None);
        while let Some(key) = map.next_key::<String>()? {
            let values = key == "values";
            let is_format = key == "format";
            let is_version = key == "format_version";
            if !seen.insert(key) {
                return Err(serde::de::Error::custom("duplicate checkpoint key"));
            }
            if values {
                map.next_value_seed(Count {
                    limit: self.0.limit,
                    exceeded: self.0.exceeded,
                })?;
            } else if is_format {
                format = Some(map.next_value()?);
            } else if is_version {
                version = Some(map.next_value()?);
            } else {
                map.next_value_seed(Discard)?;
            }
        }
        Ok((
            format.ok_or_else(|| serde::de::Error::custom("missing checkpoint format"))?,
            version.ok_or_else(|| serde::de::Error::custom("missing checkpoint version"))?,
        ))
    }
}
/// Count entries without building a literal tree/Vec before the configured bound.
pub(super) fn preflight(bytes: &[u8], limit: usize) -> Result<(), CheckpointError> {
    let mut exceeded = false;
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let result = de
        .deserialize_map(Preflight(Count {
            limit,
            exceeded: &mut exceeded,
        }))
        .and_then(|header| de.end().map(|_| header));
    let (format, version) = result.map_err(|_| {
        if exceeded {
            CheckpointError::Limit
        } else {
            CheckpointError::Invalid
        }
    })?;
    if format != "orange.checkpoint" || version != 1 {
        return Err(CheckpointError::Version);
    }
    Ok(())
}
struct Capped {
    bytes: Vec<u8>,
    limit: usize,
    full: bool,
}
impl Write for Capped {
    fn write(&mut self, chunk: &[u8]) -> io::Result<usize> {
        let Some(total) = self
            .bytes
            .len()
            .checked_add(chunk.len())
            .filter(|n| *n <= self.limit)
        else {
            self.full = true;
            return Err(io::Error::other("checkpoint limit"));
        };
        if self.bytes.capacity() < total {
            let capacity = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(64)
                .max(total)
                .min(self.limit);
            if self
                .bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .is_err()
            {
                self.full = true;
                return Err(io::Error::other("checkpoint allocation"));
            }
        }
        self.bytes.extend_from_slice(chunk);
        Ok(chunk.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn encode(document: &Document, limit: usize) -> Result<Vec<u8>, CheckpointError> {
    let mut writer = Capped {
        bytes: Vec::new(),
        limit,
        full: false,
    };
    serde_json::to_writer(&mut writer, document).map_err(|_| {
        if writer.full {
            CheckpointError::Limit
        } else {
            CheckpointError::Invalid
        }
    })?;
    Ok(writer.bytes)
}
