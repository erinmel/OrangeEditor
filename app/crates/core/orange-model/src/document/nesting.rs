//! The structural nesting budget shared by editing, saving and loading (12 §6).
//!
//! Nesting counts JSON objects and arrays from the saved document's root object, which is 1.
//! Every edit is checked before it is applied and every load before the document is built,
//! so whatever a transaction commits can be saved and read back with the same budget.

use std::fmt;

use serde::ser::{self, Serialize, Serializer};

/// The deepest container nesting a saved document may have, its root object included.
pub const MAX_NESTING: usize = 64;

/// Containers `value` adds when saved as JSON: 0 for a scalar, 1 for a flat object.
///
/// Counted without building any JSON, by mirroring the shapes `serde_json` writes:
/// enum variants with content are wrapped in one object, newtypes and `Some` are transparent.
pub(crate) fn nesting<T: Serialize + ?Sized>(value: &T) -> usize {
    value.serialize(Depth).unwrap_or(usize::MAX)
}

struct Depth;

/// Never produced: counting cannot fail, but `Serializer` needs an error type.
#[derive(Debug)]
struct Never;

impl fmt::Display for Never {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("nesting count failed")
    }
}

impl std::error::Error for Never {}

impl ser::Error for Never {
    fn custom<T: fmt::Display>(_: T) -> Self {
        Never
    }
}

/// A container being counted: its own levels plus its deepest member.
struct Container {
    levels: usize,
    deepest: usize,
}

impl Container {
    fn new(levels: usize) -> Self {
        Self { levels, deepest: 0 }
    }

    fn member<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Never> {
        self.deepest = self.deepest.max(value.serialize(Depth)?);
        Ok(())
    }

    fn total(self) -> Result<usize, Never> {
        Ok(self.levels.saturating_add(self.deepest))
    }
}

macro_rules! scalars {
    ($($method:ident($ty:ty)),+ $(,)?) => {$(
        fn $method(self, _: $ty) -> Result<usize, Never> {
            Ok(0)
        }
    )+};
}

impl Serializer for Depth {
    type Ok = usize;
    type Error = Never;
    type SerializeSeq = Container;
    type SerializeTuple = Container;
    type SerializeTupleStruct = Container;
    type SerializeTupleVariant = Container;
    type SerializeMap = Container;
    type SerializeStruct = Container;
    type SerializeStructVariant = Container;

    scalars!(
        serialize_bool(bool),
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_i128(i128),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_u128(u128),
        serialize_f32(f32),
        serialize_f64(f64),
        serialize_char(char),
        serialize_str(&str),
        serialize_bytes(&[u8]),
        serialize_unit_struct(&'static str),
    );

    fn serialize_none(self) -> Result<usize, Never> {
        Ok(0)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<usize, Never> {
        value.serialize(Depth)
    }
    fn serialize_unit(self) -> Result<usize, Never> {
        Ok(0)
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<usize, Never> {
        Ok(0)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<usize, Never> {
        value.serialize(Depth)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        value: &T,
    ) -> Result<usize, Never> {
        Ok(value.serialize(Depth)?.saturating_add(1))
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Container, Never> {
        Ok(Container::new(1))
    }
    fn serialize_tuple(self, _: usize) -> Result<Container, Never> {
        Ok(Container::new(1))
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Container, Never> {
        Ok(Container::new(1))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Container, Never> {
        Ok(Container::new(2))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Container, Never> {
        Ok(Container::new(1))
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Container, Never> {
        Ok(Container::new(1))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Container, Never> {
        Ok(Container::new(2))
    }
}

impl ser::SerializeSeq for Container {
    type Ok = usize;
    type Error = Never;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Never> {
        self.member(value)
    }
    fn end(self) -> Result<usize, Never> {
        self.total()
    }
}

impl ser::SerializeTuple for Container {
    type Ok = usize;
    type Error = Never;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Never> {
        self.member(value)
    }
    fn end(self) -> Result<usize, Never> {
        self.total()
    }
}

impl ser::SerializeTupleStruct for Container {
    type Ok = usize;
    type Error = Never;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Never> {
        self.member(value)
    }
    fn end(self) -> Result<usize, Never> {
        self.total()
    }
}

impl ser::SerializeTupleVariant for Container {
    type Ok = usize;
    type Error = Never;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Never> {
        self.member(value)
    }
    fn end(self) -> Result<usize, Never> {
        self.total()
    }
}

/// Keys are written as strings, so only values can nest.
impl ser::SerializeMap for Container {
    type Ok = usize;
    type Error = Never;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, _: &T) -> Result<(), Never> {
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Never> {
        self.member(value)
    }
    fn end(self) -> Result<usize, Never> {
        self.total()
    }
}

impl ser::SerializeStruct for Container {
    type Ok = usize;
    type Error = Never;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Never> {
        self.member(value)
    }
    fn end(self) -> Result<usize, Never> {
        self.total()
    }
}

impl ser::SerializeStructVariant for Container {
    type Ok = usize;
    type Error = Never;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Never> {
        self.member(value)
    }
    fn end(self) -> Result<usize, Never> {
        self.total()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The count must equal the nesting of the JSON `serde_json` actually writes.
    fn written(value: &serde_json::Value) -> usize {
        match value {
            serde_json::Value::Array(items) => 1 + items.iter().map(written).max().unwrap_or(0),
            serde_json::Value::Object(map) => 1 + map.values().map(written).max().unwrap_or(0),
            _ => 0,
        }
    }

    #[derive(serde::Serialize)]
    enum Shape {
        Unit,
        Newtype(Vec<u8>),
        Tuple(u8, Vec<u8>),
        Struct { inner: Option<Vec<u8>> },
    }

    #[test]
    fn nesting_matches_the_written_json() {
        for value in [
            json!(1),
            json!([]),
            json!({"a": [1, {"b": {}}]}),
            json!([[[[]]], {"c": null}]),
        ] {
            assert_eq!(nesting(&value), written(&value), "{value}");
        }
        for shape in [
            Shape::Unit,
            Shape::Newtype(vec![1]),
            Shape::Tuple(1, vec![2]),
            Shape::Struct {
                inner: Some(vec![3]),
            },
        ] {
            let value = serde_json::to_value(&shape).unwrap();
            assert_eq!(nesting(&shape), written(&value), "{value}");
        }
    }
}
