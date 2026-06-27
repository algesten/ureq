use core::{fmt, marker};
use std::borrow::Cow;

use crate::query::QueryParam;

use serde::ser;

pub trait Type {
    fn new_key_value(key: &str, value: &str) -> QueryParam<'static>;
}

pub struct Escaped;

impl Type for Escaped {
    #[inline]
    fn new_key_value(key: &str, value: &str) -> QueryParam<'static> {
        QueryParam::new_key_value(key, value)
    }
}

pub struct Raw;

impl Type for Raw {
    #[inline]
    fn new_key_value(key: &str, value: &str) -> QueryParam<'static> {
        QueryParam::new_key_value_raw(key, value)
    }
}

#[derive(Clone, Debug)]
pub struct Error {
    msg: Cow<'static, str>,
}

impl Error {
    const fn expected_struct_like() -> Self {
        Self {
            msg: Cow::Borrowed("Query params expect struct/map"),
        }
    }

    const fn expected_string_like_value() -> Self {
        Self {
            msg: Cow::Borrowed("Query value must be string or number"),
        }
    }

    const fn expected_value() -> Self {
        Self {
            msg: Cow::Borrowed("Query value cannot be struct/map/sequence/tuple"),
        }
    }
}

impl fmt::Display for Error {
    #[inline]
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.write_str(self.msg.as_ref())
    }
}

impl std::error::Error for Error {}

impl ser::Error for Error {
    #[inline]
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self {
            msg: msg.to_string().into(),
        }
    }
}

pub struct QueryVisitor<'a, T> {
    output: &'a mut Vec<QueryParam<'static>>,
    _typ: marker::PhantomData<T>,
}

impl<'a> QueryVisitor<'a, Escaped> {
    pub fn escaped(output: &'a mut Vec<QueryParam<'static>>) -> Self {
        Self {
            output,
            _typ: marker::PhantomData,
        }
    }
}

impl<'a> QueryVisitor<'a, Raw> {
    pub fn raw(output: &'a mut Vec<QueryParam<'static>>) -> Self {
        Self {
            output,
            _typ: marker::PhantomData,
        }
    }
}

impl<'output, T: Type> ser::Serializer for QueryVisitor<'output, T> {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeMap = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeTuple = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeStruct = QueryVisitor<'output, T>;
    type SerializeTupleStruct = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeTupleVariant = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeStructVariant = QueryVisitor<'output, T>;

    fn serialize_bool(self, _v: bool) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    fn serialize_i8(self, _v: i8) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    fn serialize_i16(self, _v: i16) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    fn serialize_i32(self, _v: i32) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    fn serialize_i64(self, _v: i64) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    fn serialize_u8(self, _v: u8) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    fn serialize_u16(self, _v: u16) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    fn serialize_u32(self, _v: u32) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    fn serialize_u64(self, _v: u64) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    fn serialize_f32(self, _v: f32) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    fn serialize_f64(self, _v: f64) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    fn serialize_char(self, _v: char) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    fn serialize_str(self, _value: &str) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    fn serialize_bytes(self, _value: &[u8]) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    /// Returns `Ok`.
    fn serialize_unit(self) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    /// Returns `Ok`.
    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
    ) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    /// Serializes the inner value, ignoring the newtype name.
    fn serialize_newtype_struct<V: ?Sized + ser::Serialize>(
        self,
        _name: &'static str,
        value: &V,
    ) -> Result<Self::Ok, Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<V: ?Sized + ser::Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &V,
    ) -> Result<Self::Ok, Error> {
        Err(Error::expected_struct_like())
    }

    /// Returns `Ok`.
    fn serialize_none(self) -> Result<Self::Ok, Error> {
        Ok(())
    }

    /// Serializes the given value.
    fn serialize_some<V: ?Sized + ser::Serialize>(self, value: &V) -> Result<Self::Ok, Error> {
        value.serialize(self)
    }

    /// Serialize a sequence, given length (if any) is ignored.
    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Error> {
        todo!();
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Error> {
        todo!();
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Error> {
        todo!();
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        todo!();
    }

    /// Serializes a map, given length is ignored.
    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Error> {
        todo!();
    }

    /// Serializes a struct, given length is ignored.
    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Error> {
        Ok(self)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        Ok(self)
    }
}

impl<'output, T: Type> ser::SerializeStructVariant for QueryVisitor<'output, T> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<V: ?Sized + ser::Serialize>(
        &mut self,
        key: &'static str,
        value: &V,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(QueryFieldVisitor::<T>::new(key, self.output))
    }

    fn skip_field(&mut self, _key: &'static str) -> Result<(), Self::Error> {
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl<'output, T: Type> ser::SerializeStruct for QueryVisitor<'output, T> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<V: ?Sized + ser::Serialize>(
        &mut self,
        key: &'static str,
        value: &V,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(QueryFieldVisitor::<T>::new(key, self.output))
    }

    fn skip_field(&mut self, _key: &'static str) -> Result<(), Self::Error> {
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

struct QueryFieldVisitor<'a, T> {
    params: &'a mut Vec<QueryParam<'static>>,
    key: &'a str,
    _typ: marker::PhantomData<T>,
}

impl<'a, T: Type> QueryFieldVisitor<'a, T> {
    fn new(key: &'a str, params: &'a mut Vec<QueryParam<'static>>) -> Self {
        Self {
            params,
            key,
            _typ: marker::PhantomData,
        }
    }
}

impl<T: Type> ser::Serializer for QueryFieldVisitor<'_, T> {
    type Ok = ();
    type Error = Error;
    type SerializeStructVariant = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeTupleVariant = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeTupleStruct = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeStruct = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeTuple = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeMap = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeSeq = ser::Impossible<Self::Ok, Self::Error>;

    fn serialize_bool(self, value: bool) -> Result<Self::Ok, Error> {
        let value = if value { "true" } else { "false" };
        self.serialize_str(value)
    }

    fn serialize_i8(self, value: i8) -> Result<Self::Ok, Error> {
        self.serialize_str(itoa::Buffer::new().format(value))
    }

    fn serialize_i16(self, value: i16) -> Result<Self::Ok, Error> {
        self.serialize_str(itoa::Buffer::new().format(value))
    }

    fn serialize_i32(self, value: i32) -> Result<Self::Ok, Error> {
        self.serialize_str(itoa::Buffer::new().format(value))
    }

    fn serialize_i64(self, value: i64) -> Result<Self::Ok, Error> {
        self.serialize_str(itoa::Buffer::new().format(value))
    }

    fn serialize_u8(self, value: u8) -> Result<Self::Ok, Error> {
        self.serialize_str(itoa::Buffer::new().format(value))
    }

    fn serialize_u16(self, value: u16) -> Result<Self::Ok, Error> {
        self.serialize_str(itoa::Buffer::new().format(value))
    }

    fn serialize_u32(self, value: u32) -> Result<Self::Ok, Error> {
        self.serialize_str(itoa::Buffer::new().format(value))
    }

    fn serialize_u64(self, value: u64) -> Result<Self::Ok, Error> {
        self.serialize_str(itoa::Buffer::new().format(value))
    }

    fn serialize_f32(self, value: f32) -> Result<Self::Ok, Error> {
        self.serialize_str(ryu::Buffer::new().format(value))
    }

    fn serialize_f64(self, value: f64) -> Result<Self::Ok, Error> {
        self.serialize_str(ryu::Buffer::new().format(value))
    }

    fn serialize_char(self, value: char) -> Result<Self::Ok, Error> {
        //unicode max length is 4 bytes
        let mut buffer = [0u8; 4];
        let value = value.encode_utf8(&mut buffer);
        self.serialize_str(value)
    }

    fn serialize_str(self, value: &str) -> Result<Self::Ok, Error> {
        self.params.push(T::new_key_value(self.key, value));
        Ok(())
    }

    fn serialize_bytes(self, _value: &[u8]) -> Result<Self::Ok, Error> {
        Err(Error::expected_string_like_value())
    }

    fn serialize_unit(self) -> Result<Self::Ok, Error> {
        Err(Error::expected_value())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Error> {
        Err(Error::expected_value())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
    ) -> Result<Self::Ok, Error> {
        Err(Error::expected_value())
    }

    /// Serializes the inner value, ignoring the newtype name.
    fn serialize_newtype_struct<V: ?Sized + ser::Serialize>(
        self,
        _name: &'static str,
        value: &V,
    ) -> Result<Self::Ok, Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<V: ?Sized + ser::Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &V,
    ) -> Result<Self::Ok, Error> {
        Err(Error::expected_value())
    }

    /// Skip field if it is None
    fn serialize_none(self) -> Result<Self::Ok, Error> {
        Ok(())
    }

    /// Serializes the given value.
    fn serialize_some<V: ?Sized + ser::Serialize>(self, value: &V) -> Result<Self::Ok, Error> {
        value.serialize(self)
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Error> {
        Err(Error::expected_value())
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Error> {
        Err(Error::expected_value())
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Error> {
        Err(Error::expected_value())
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        Err(Error::expected_value())
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Error> {
        Err(Error::expected_value())
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Error> {
        Err(Error::expected_value())
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        Err(Error::expected_value())
    }
}
