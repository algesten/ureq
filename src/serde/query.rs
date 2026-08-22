use core::{fmt, marker, mem};
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
    const fn expected_string_key() -> Self {
        Self {
            msg: Cow::Borrowed("Query key must be string"),
        }
    }

    #[cold]
    #[inline(never)]
    const fn expected_non_empty_string_value() -> Self {
        Self {
            msg: Cow::Borrowed("Query key cannot be empty string"),
        }
    }

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

    const fn expected_pair() -> Self {
        Self {
            msg: Cow::Borrowed("Element in sequence must be pair of key and value"),
        }
    }

    #[cold]
    #[inline(never)]
    const fn unexpected_pair_serde_error() -> Self {
        Self {
            msg: Cow::Borrowed(
                "Internal error: Pair was supposed to have 2 elements but it is not the case",
            ),
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

///QueryParam serializer
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
    type SerializeSeq = QueryFieldPairSeqVisitor<'output, T>;
    type SerializeMap = QueryFieldMapVisitor<'output, T>;
    type SerializeTuple = QueryFieldPairSeqVisitor<'output, T>;
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
        Ok(())
    }

    /// Returns `Ok`.
    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Error> {
        Ok(())
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
        Ok(QueryFieldPairSeqVisitor::new(self.output))
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Error> {
        Ok(QueryFieldPairSeqVisitor::new(self.output))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Error> {
        Err(Error::expected_struct_like())
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        Err(Error::expected_struct_like())
    }

    /// Serializes a map, given length is ignored.
    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Error> {
        Ok(QueryFieldMapVisitor::new(self.output))
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

///Struct field's value visitor
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

//Serializer for dynamic key field.
//
//Key field must be string or string like type
struct KeyField<'output> {
    key: &'output mut String,
}

impl<'output> ser::Serializer for KeyField<'output> {
    type Ok = ();
    type Error = Error;
    type SerializeStructVariant = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeTupleVariant = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeTupleStruct = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeStruct = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeTuple = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeMap = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeSeq = ser::Impossible<Self::Ok, Self::Error>;

    fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
        self.key.push_str(value);
        Ok(())
    }

    fn serialize_char(self, value: char) -> Result<Self::Ok, Self::Error> {
        self.key.push(value);
        Ok(())
    }

    fn collect_str<T: ?Sized + fmt::Display>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        use fmt::Write;

        //Cannot fail
        let _ = write!(self.key, "{value}");
        Ok(())
    }

    fn serialize_some<T: ?Sized + ser::Serialize>(
        self,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_bytes(self, _: &[u8]) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_newtype_struct<T: ?Sized + ser::Serialize>(
        self,
        _: &'static str,
        _: &T,
    ) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_newtype_variant<T: ?Sized + ser::Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_bool(self, _: bool) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_f32(self, _: f32) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_f64(self, _: f64) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_i8(self, _: i8) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_i16(self, _: i16) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_i32(self, _: i32) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_i64(self, _: i64) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_i128(self, _: i128) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_u8(self, _: u8) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_u16(self, _: u16) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_u32(self, _: u32) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_u64(self, _: u64) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn serialize_u128(self, _: u128) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_string_key())
    }

    fn collect_seq<I: IntoIterator>(self, _: I) -> Result<Self::Ok, Self::Error>
    where
        <I as IntoIterator>::Item: ser::Serialize,
    {
        Err(Error::expected_string_key())
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Err(Error::expected_string_key())
    }
}

///Map visitor
pub struct QueryFieldMapVisitor<'a, T> {
    params: &'a mut Vec<QueryParam<'static>>,
    key: String,
    _typ: marker::PhantomData<T>,
}

impl<'a, T: Type> QueryFieldMapVisitor<'a, T> {
    fn new(params: &'a mut Vec<QueryParam<'static>>) -> Self {
        Self {
            params,
            key: String::new(),
            _typ: marker::PhantomData,
        }
    }
}

impl<'output, T: Type> ser::SerializeMap for QueryFieldMapVisitor<'output, T> {
    type Ok = ();
    type Error = Error;

    fn serialize_key<K: ?Sized + ser::Serialize>(&mut self, key: &K) -> Result<(), Self::Error> {
        let dest = KeyField { key: &mut self.key };
        key.serialize(dest)
    }

    fn serialize_value<V: ?Sized + ser::Serialize>(
        &mut self,
        value: &V,
    ) -> Result<(), Self::Error> {
        if self.key.is_empty() {
            return Err(Error::expected_non_empty_string_value());
        }

        let params = &mut *self.params;
        let ser = QueryFieldVisitor::<T>::new(&self.key, params);
        let result = value.serialize(ser);
        self.key.clear();
        result
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

enum QueryPairState {
    Empty,
    HasKey,
    Done,
}

///Pair visitor that expects tuple with 2 elements
struct QueryPairVisitor<'a, T> {
    params: &'a mut Vec<QueryParam<'static>>,
    key: &'a mut String,
    state: QueryPairState,
    _typ: marker::PhantomData<T>,
}

impl<'output, T: Type> ser::Serializer for QueryPairVisitor<'output, T> {
    type Ok = ();
    type Error = Error;
    type SerializeStructVariant = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeTupleVariant = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeTupleStruct = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeStruct = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeTuple = QueryPairVisitor<'output, T>;
    type SerializeMap = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeSeq = ser::Impossible<Self::Ok, Self::Error>;

    fn serialize_str(self, _: &str) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_char(self, _: char) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn collect_str<V: ?Sized + fmt::Display>(self, _: &V) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_some<V: ?Sized + ser::Serialize>(
        self,
        value: &V,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_bytes(self, _: &[u8]) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        if len == 2 {
            Ok(self)
        } else {
            Err(Error::expected_pair())
        }
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_newtype_struct<V: ?Sized + ser::Serialize>(
        self,
        _: &'static str,
        _: &V,
    ) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_newtype_variant<V: ?Sized + ser::Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &V,
    ) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_bool(self, _: bool) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_f32(self, _: f32) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_f64(self, _: f64) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_i8(self, _: i8) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_i16(self, _: i16) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_i32(self, _: i32) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_i64(self, _: i64) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_i128(self, _: i128) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_u8(self, _: u8) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_u16(self, _: u16) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_u32(self, _: u32) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_u64(self, _: u64) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn serialize_u128(self, _: u128) -> Result<Self::Ok, Self::Error> {
        Err(Error::expected_pair())
    }

    fn collect_seq<I: IntoIterator>(self, _: I) -> Result<Self::Ok, Self::Error>
    where
        <I as IntoIterator>::Item: ser::Serialize,
    {
        Err(Error::expected_pair())
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Err(Error::expected_pair())
    }
}

impl<'output, T: Type> ser::SerializeTuple for QueryPairVisitor<'output, T> {
    type Ok = ();
    type Error = Error;

    fn serialize_element<V: ?Sized + ser::Serialize>(
        &mut self,
        value: &V,
    ) -> Result<(), Self::Error> {
        match mem::replace(&mut self.state, QueryPairState::Done) {
            QueryPairState::Empty => {
                let dest = KeyField { key: self.key };
                value.serialize(dest)?;
                self.state = QueryPairState::HasKey;
                Ok(())
            }
            QueryPairState::HasKey => {
                if self.key.is_empty() {
                    return Err(Error::expected_non_empty_string_value());
                }
                let params = &mut *self.params;
                let ser = QueryFieldVisitor::<T>::new(self.key, params);
                value.serialize(ser)
            }
            //Unreachable unless bug in code
            QueryPairState::Done => Err(Error::unexpected_pair_serde_error()),
        }
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        match self.state {
            QueryPairState::Done => Ok(()),
            _ => Err(Error::unexpected_pair_serde_error()),
        }
    }
}

///Pair Sequence visitor
pub struct QueryFieldPairSeqVisitor<'a, T> {
    params: &'a mut Vec<QueryParam<'static>>,
    key: String,
    _typ: marker::PhantomData<T>,
}

impl<'a, T: Type> QueryFieldPairSeqVisitor<'a, T> {
    fn new(params: &'a mut Vec<QueryParam<'static>>) -> Self {
        Self {
            params,
            key: String::new(),
            _typ: marker::PhantomData,
        }
    }

    fn get_visitor(&mut self) -> QueryPairVisitor<'_, T> {
        //Clear key storage for new pair
        self.key.clear();
        QueryPairVisitor::<T> {
            params: &mut *self.params,
            key: &mut self.key,
            state: QueryPairState::Empty,
            _typ: marker::PhantomData,
        }
    }
}

impl<'output, T: Type> ser::SerializeTuple for QueryFieldPairSeqVisitor<'output, T> {
    type Ok = ();
    type Error = Error;

    fn serialize_element<E: ?Sized + ser::Serialize>(
        &mut self,
        value: &E,
    ) -> Result<(), Self::Error> {
        value.serialize(self.get_visitor())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl<'output, T: Type> ser::SerializeSeq for QueryFieldPairSeqVisitor<'output, T> {
    type Ok = ();
    type Error = Error;

    fn serialize_element<E: ?Sized + ser::Serialize>(
        &mut self,
        value: &E,
    ) -> Result<(), Self::Error> {
        value.serialize(self.get_visitor())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::QueryVisitor;
    use std::collections::BTreeMap;

    use serde::Serialize;

    #[test]
    fn should_serde_encode_query_from_map() {
        let mut query = BTreeMap::new();
        query.insert("key 1", "value 1");
        query.insert("key 2", "value 2");

        let mut output = Vec::new();
        query
            .serialize(QueryVisitor::escaped(&mut output))
            .expect("to serialize");
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].as_str(), "key%201=value%201");
        assert_eq!(output[1].as_str(), "key%202=value%202");

        output.clear();
        query
            .serialize(QueryVisitor::raw(&mut output))
            .expect("to serialize");
        assert_eq!(output[0].as_str(), "key 1=value 1");
        assert_eq!(output[1].as_str(), "key 2=value 2");
    }

    #[test]
    fn should_serde_encode_query_from_seq() {
        let query = [("key 1", "value 1"), ("key 2", "value 2")];

        let mut output = Vec::new();
        query
            .serialize(QueryVisitor::escaped(&mut output))
            .expect("to serialize");
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].as_str(), "key%201=value%201");
        assert_eq!(output[1].as_str(), "key%202=value%202");

        output.clear();
        query
            .serialize(QueryVisitor::raw(&mut output))
            .expect("to serialize");
        assert_eq!(output[0].as_str(), "key 1=value 1");
        assert_eq!(output[1].as_str(), "key 2=value 2");
    }

    #[test]
    fn should_serde_encode_query_from_tuple() {
        let query = (("key 1", "value 1"), ("key 2", "value 2"));

        let mut output = Vec::new();
        query
            .serialize(QueryVisitor::escaped(&mut output))
            .expect("to serialize");
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].as_str(), "key%201=value%201");
        assert_eq!(output[1].as_str(), "key%202=value%202");

        output.clear();
        query
            .serialize(QueryVisitor::raw(&mut output))
            .expect("to serialize");
        assert_eq!(output[0].as_str(), "key 1=value 1");
        assert_eq!(output[1].as_str(), "key 2=value 2");
    }
}
