use core::mem;
use core::str::{from_utf8, FromStr};

use byteorder::{BigEndian, ByteOrder, LittleEndian};
use derive_more::{Deref, Display, From};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::Error::{self, *};

pub trait ReadExt<'a>
where
    Self: Sized,
{
    type Error;

    fn read<O: ByteOrder>(r: &mut untrusted::Reader<'a>) -> Result<Self, Self::Error>;
}

pub trait ReadBytesExt {
    type Error;

    fn read_ty<T: Sized, F: FnOnce(&[u8]) -> T>(&mut self, f: F) -> Result<T, Self::Error>;

    fn read_u16<O: ByteOrder>(&mut self) -> Result<u16, Self::Error> {
        self.read_ty(O::read_u16)
    }

    fn read_u32<O: ByteOrder>(&mut self) -> Result<u32, Self::Error> {
        self.read_ty(O::read_u32)
    }

    fn read_i16<O: ByteOrder>(&mut self) -> Result<i16, Self::Error> {
        self.read_ty(O::read_i16)
    }

    fn read_i32<O: ByteOrder>(&mut self) -> Result<i32, Self::Error> {
        self.read_ty(O::read_i32)
    }
}

impl<'a> ReadBytesExt for untrusted::Reader<'a> {
    type Error = untrusted::EndOfInput;

    fn read_ty<T: Sized, F: FnOnce(&[u8]) -> T>(&mut self, f: F) -> Result<T, Self::Error> {
        self.read_bytes(mem::size_of::<T>())
            .map(|i| f(i.as_slice_less_safe()))
    }
}

#[repr(C)]
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub magic: u16,
    pub version: u8,
    pub flags: u8,
    pub len: u32,

    /* All offsets are in bytes relative to the end of this header */
    pub type_off: u32, // offset of type section
    pub type_len: u32, // length of type section
    pub str_off: u32,  // offset of string section
    pub str_len: u32,  // length of string section
}

impl Header {
    pub const MAGIC: u16 = 0xeb9f;
    pub const VERSION: u8 = 1;

    pub fn is_le(&self) -> bool {
        self.magic == Self::MAGIC
    }

    pub fn read<O: ByteOrder>(r: &mut untrusted::Reader) -> Result<Self, Error> {
        let hdr = Header {
            magic: r.read_u16::<LittleEndian>()?,
            version: r.read_byte()?,
            flags: r.read_byte()?,
            len: r.read_u32::<O>()?,
            type_off: r.read_u32::<O>()?,
            type_len: r.read_u32::<O>()?,
            str_off: r.read_u32::<O>()?,
            str_len: r.read_u32::<O>()?,
        };

        match (hdr.len as usize).checked_sub(mem::size_of::<Self>()) {
            Some(n) if n > 0 => {
                r.skip(n)?;
            }
            _ => {}
        }

        Ok(hdr)
    }
}

#[repr(C)]
#[derive(Debug, Clone, PartialEq, Deref)]
pub struct Type {
    pub name_off: u32,
    /* "info" bits arrangement
     * bits  0-15: vlen (e.g. # of struct's members)
     * bits 16-23: unused
     * bits 24-28: kind (e.g. int, ptr, array...etc)
     * bits 29-30: unused
     * bit     31: kind_flag, currently used by
     *             struct, union and fwd
     */
    #[deref]
    pub info: Info,
    /* "size" is used by INT, ENUM, STRUCT, UNION and DATASEC.
     * "size" tells the size of the type it is describing.
     *
     * "type" is used by PTR, TYPEDEF, VOLATILE, CONST, RESTRICT,
     * FUNC, FUNC_PROTO and VAR.
     * "type" is a type_id referring to another type.
     */
    pub size_or_type: u32,
}

impl Type {
    pub fn size(&self) -> usize {
        self.size_or_type as usize
    }

    pub fn type_id(&self) -> u32 {
        self.size_or_type
    }
}

impl<'a> ReadExt<'a> for Type {
    type Error = Error;

    fn read<O: ByteOrder>(r: &mut untrusted::Reader) -> Result<Self, Error> {
        Ok(Type {
            name_off: r.read_u32::<O>()?,
            info: Info(r.read_u32::<O>()?),
            size_or_type: r.read_u32::<O>()?,
        })
    }
}

/* "info" bits arrangement
 * bits  0-15: vlen (e.g. # of struct's members)
 * bits 16-23: unused
 * bits 24-27: kind (e.g. int, ptr, array...etc)
 * bits 28-30: unused
 * bit     31: kind_flag, currently used by
 *             struct, union and fwd
 */
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, From)]
pub struct Info(pub u32);

impl Info {
    const VLEN_MASK: u32 = 0x0000_ffff;
    const KIND_MASK: u32 = 0x1f00_0000;
    const KIND_FLAG: u32 = 0x8000_0000;
    const KIND_SHIFT: usize = 24;

    pub fn vlen(&self) -> usize {
        (self.0 & Self::VLEN_MASK) as usize
    }

    pub fn kind(&self) -> Kind {
        unsafe { mem::transmute(((self.0 & Self::KIND_MASK) >> Self::KIND_SHIFT) as u8) }
    }

    pub fn kflag(&self) -> bool {
        (self.0 & Self::KIND_FLAG) != 0
    }

    pub fn type_size(&self) -> usize {
        mem::size_of::<Type>()
            + match self.kind() {
                Kind::Integer => mem::size_of::<u32>(),
                Kind::Enum => mem::size_of::<Enum>() * self.vlen(),
                Kind::Enum64 => mem::size_of::<Enum64>() * self.vlen(),
                Kind::Array => mem::size_of::<Array>(),
                Kind::Struct | Kind::Union => mem::size_of::<Member>() * self.vlen(),
                Kind::FuncProto => mem::size_of::<Param>() * self.vlen(),
                Kind::Variable => mem::size_of::<Var>(),
                Kind::DataSection => mem::size_of::<VarSectInfo>() * self.vlen(),
                Kind::DeclTag => mem::size_of::<DeclTag>(),
                Kind::Unknown
                | Kind::Forward
                | Kind::Const
                | Kind::Volatile
                | Kind::Restrict
                | Kind::Pointer
                | Kind::Typedef
                | Kind::Func
                | Kind::Float
                | Kind::TypeTag => 0,
            }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Display, From)]
#[cfg_attr(
    feature = "serde",
    derive(Deserialize, Serialize),
    serde(rename_all = "lowercase")
)]
pub enum Kind {
    Unknown = 0,
    Integer = 1,
    Pointer = 2,
    Array = 3,
    Struct = 4,
    Union = 5,
    Enum = 6,
    Forward = 7,
    Typedef = 8,
    Volatile = 9,
    Const = 10,
    Restrict = 11,
    Func = 12,
    FuncProto = 13,
    Variable = 14,
    DataSection = 15,
    Float = 16,
    DeclTag = 17,
    TypeTag = 18,
    Enum64 = 19,
}

impl Kind {
    pub fn is_void(&self) -> bool {
        *self == Kind::Unknown
    }

    pub fn is_int(&self) -> bool {
        *self == Kind::Integer
    }

    pub fn is_ptr(&self) -> bool {
        *self == Kind::Pointer
    }

    pub fn is_array(&self) -> bool {
        *self == Kind::Array
    }

    pub fn is_struct(&self) -> bool {
        *self == Kind::Struct
    }

    pub fn is_union(&self) -> bool {
        *self == Kind::Union
    }

    pub fn is_composite(&self) -> bool {
        matches!(self, Kind::Struct | Kind::Union)
    }

    pub fn is_enum(&self) -> bool {
        *self == Kind::Enum
    }

    pub fn is_enum64(&self) -> bool {
        *self == Kind::Enum64
    }

    pub fn is_forword(&self) -> bool {
        *self == Kind::Forward
    }

    pub fn is_typedef(&self) -> bool {
        *self == Kind::Typedef
    }

    pub fn is_volatile(&self) -> bool {
        *self == Kind::Volatile
    }

    pub fn is_const(&self) -> bool {
        *self == Kind::Const
    }

    pub fn is_restrict(&self) -> bool {
        *self == Kind::Restrict
    }

    pub fn is_modifier(&self) -> bool {
        matches!(
            self,
            Kind::Volatile | Kind::Const | Kind::Restrict | Kind::TypeTag
        )
    }

    pub fn is_func(&self) -> bool {
        *self == Kind::Func
    }

    pub fn is_func_proto(&self) -> bool {
        *self == Kind::FuncProto
    }

    pub fn is_var(&self) -> bool {
        *self == Kind::Variable
    }

    pub fn is_data_section(&self) -> bool {
        *self == Kind::DataSection
    }

    pub fn is_float(&self) -> bool {
        *self == Kind::Float
    }

    pub fn is_decl_tag(&self) -> bool {
        *self == Kind::DeclTag
    }

    pub fn is_type_tag(&self) -> bool {
        *self == Kind::TypeTag
    }

    pub fn is_any_enum(&self) -> bool {
        matches!(self, Kind::Enum | Kind::Enum64)
    }
}

/// BTF_KIND_INT is followed by a u32 and the following is the 32 bits arrangement:

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Int(pub u32);

impl Int {
    const ENCODING_MASK: u32 = 0x0f000000;
    const OFFSET_MASK: u32 = 0x00ff0000;
    const BITS_MASK: u32 = 0x000000ff;

    /* Attributes stored in the BTF_INT_ENCODING */

    const ENCODING_SHIFT: usize = 24;
    const OFFSET_SHIFT: usize = 16;

    pub fn offset(&self) -> usize {
        ((self.0 & Self::OFFSET_MASK) >> Self::OFFSET_SHIFT) as usize
    }

    pub fn bits(&self) -> usize {
        (self.0 & Self::BITS_MASK) as usize
    }

    pub fn encoding(&self) -> IntEncoding {
        IntEncoding::from_bits_truncate((self.0 & Self::ENCODING_MASK) >> Self::ENCODING_SHIFT)
    }

    pub fn is_signed(&self) -> bool {
        self.encoding().contains(IntEncoding::SIGNED)
    }

    pub fn is_char(&self) -> bool {
        self.encoding().contains(IntEncoding::CHAR)
    }

    pub fn is_bool(&self) -> bool {
        self.encoding().contains(IntEncoding::BOOL)
    }
}

impl<'a> ReadExt<'a> for Int {
    type Error = Error;

    fn read<O: ByteOrder>(r: &mut untrusted::Reader) -> Result<Self, Error> {
        Ok(Int(r.read_u32::<O>()?))
    }
}

bitflags::bitflags! {
    #[derive(Default)]
    pub struct IntEncoding: u32 {
        const SIGNED = 1 << 0;
        const CHAR = 1 << 1;
        const BOOL = 1 << 2;
    }
}

impl IntEncoding {
    pub fn is_signed(&self) -> bool {
        self.contains(IntEncoding::SIGNED)
    }

    pub fn is_char(&self) -> bool {
        self.contains(IntEncoding::CHAR)
    }

    pub fn is_bool(&self) -> bool {
        self.contains(IntEncoding::BOOL)
    }
}

impl FromStr for IntEncoding {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.split('|').map(str::trim).map(str::to_lowercase).fold(
            Ok(IntEncoding::default()),
            |e, s| match s.as_str() {
                "signed" => e.map(|e| e | Self::SIGNED),
                "char" => e.map(|e| e | Self::CHAR),
                "bool" => e.map(|e| e | Self::BOOL),
                _ => Err(Unexpected("int encoding")),
            },
        )
    }
}

#[cfg(feature = "serde")]
impl Serialize for IntEncoding {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u32(self.bits)
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for IntEncoding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = IntEncoding;

            fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
                formatter.write_str("an integer between 0 and 2^32")
            }

            fn visit_u64<Error>(self, value: u64) -> Result<Self::Value, Error> {
                Ok(IntEncoding::from_bits_truncate(value as u32))
            }
        }

        deserializer.deserialize_u32(Visitor)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Array {
    pub ty: u32,
    pub index_ty: u32,
    pub nelems: u32,
}

impl<'a> ReadExt<'a> for Array {
    type Error = Error;

    fn read<O: ByteOrder>(r: &mut untrusted::Reader) -> Result<Self, Error> {
        Ok(Array {
            ty: r.read_u32::<O>()?,
            index_ty: r.read_u32::<O>()?,
            nelems: r.read_u32::<O>()?,
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Member {
    pub name_off: u32,
    pub ty: u32,
    pub offset: u32,
}

impl Member {
    pub fn bitfield_size(&self) -> u32 {
        self.offset >> 24
    }

    pub fn bit_offset(&self) -> u32 {
        self.offset & 0x00ff_ffff
    }
}

impl<'a> ReadExt<'a> for Member {
    type Error = Error;

    fn read<O: ByteOrder>(r: &mut untrusted::Reader) -> Result<Self, Error> {
        Ok(Member {
            name_off: r.read_u32::<O>()?,
            ty: r.read_u32::<O>()?,
            offset: r.read_u32::<O>()?,
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Enum {
    pub name_off: u32,
    pub val: u32,
}

impl<'a> ReadExt<'a> for Enum {
    type Error = Error;

    fn read<O: ByteOrder>(r: &mut untrusted::Reader) -> Result<Self, Error> {
        Ok(Enum {
            name_off: r.read_u32::<O>()?,
            val: r.read_u32::<O>()?,
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Enum64 {
    pub name_off: u32,
    pub val_lo32: u32,
    pub val_hi32: u32,
}

impl<'a> ReadExt<'a> for Enum64 {
    type Error = Error;

    fn read<O: ByteOrder>(r: &mut untrusted::Reader) -> Result<Self, Error> {
        Ok(Enum64 {
            name_off: r.read_u32::<O>()?,
            val_lo32: r.read_u32::<O>()?,
            val_hi32: r.read_u32::<O>()?,
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Param {
    pub name_off: u32,
    pub ty: u32,
}

impl Param {
    pub fn is_variable_argument(&self) -> bool {
        self.name_off == 0 || self.ty == 0
    }
}

impl<'a> ReadExt<'a> for Param {
    type Error = Error;

    fn read<O: ByteOrder>(r: &mut untrusted::Reader) -> Result<Self, Error> {
        Ok(Param {
            name_off: r.read_u32::<O>()?,
            ty: r.read_u32::<O>()?,
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Var {
    pub linkage: Linkage,
}

impl<'a> ReadExt<'a> for Var {
    type Error = Error;

    fn read<O: ByteOrder>(r: &mut untrusted::Reader) -> Result<Self, Error> {
        Ok(Var {
            linkage: Linkage::from(r.read_u32::<O>()?),
        })
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Deserialize, Serialize),
    serde(rename_all = "lowercase")
)]
pub enum Linkage {
    Static = 0,
    Global = 1,
    Extern = 2,
}

impl core::fmt::Display for Linkage {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Linkage::Static => write!(f, "static"),
            Linkage::Global => write!(f, "global"),
            Linkage::Extern => write!(f, "extern"),
        }
    }
}

impl From<u32> for Linkage {
    fn from(v: u32) -> Self {
        unsafe { mem::transmute(v) }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct VarSectInfo {
    pub type_id: u32,
    pub offset: u32,
    pub size: u32,
}

impl<'a> ReadExt<'a> for VarSectInfo {
    type Error = Error;

    fn read<O: ByteOrder>(r: &mut untrusted::Reader) -> Result<Self, Error> {
        Ok(VarSectInfo {
            type_id: r.read_u32::<O>()?,
            offset: r.read_u32::<O>()?,
            size: r.read_u32::<O>()?,
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeclTag {
    pub component_idx: i32,
}

impl<'a> ReadExt<'a> for DeclTag {
    type Error = Error;

    fn read<O: ByteOrder>(r: &mut untrusted::Reader) -> Result<Self, Error> {
        Ok(DeclTag {
            component_idx: r.read_i32::<O>()?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct File<'a> {
    pub header: Header,
    pub types: untrusted::Input<'a>,
    pub strs: untrusted::Input<'a>,
}

impl<'a> ReadExt<'a> for File<'a> {
    type Error = Error;

    fn read<O: ByteOrder>(r: &mut untrusted::Reader<'a>) -> Result<File<'a>, Error> {
        let header = Header::read::<O>(r)?;

        r.skip(header.type_off as usize)?;

        let types = r.read_bytes(header.type_len as usize)?;

        r.skip((header.str_off - header.type_off - header.type_len) as usize)?;

        let strs = r.read_bytes(header.str_len as usize)?;

        r.skip_to_end();

        Ok(File {
            header,
            types,
            strs,
        })
    }
}

pub fn parse(input: untrusted::Input) -> Result<File, Error> {
    match input.as_slice_less_safe() {
        [0x9f, 0xeb, ..] => input.read_all(EndOfInput, File::read::<LittleEndian>),
        [0xeb, 0x9f, ..] => input.read_all(EndOfInput, File::read::<BigEndian>),
        _ => Err(Malformed("invalid magic")),
    }
}

pub fn read_str<'a>(input: &untrusted::Input<'a>, off: u32) -> Result<Option<&'a str>, Error> {
    if off == 0 {
        Ok(None)
    } else {
        input
            .as_slice_less_safe()
            .get(off as usize..)
            .and_then(|s| s.split(|&b| b == 0).next())
            .map(from_utf8)
            .transpose()
            .map_err(Utf8Error)
    }
}
