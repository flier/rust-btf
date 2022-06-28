#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use byteorder::{BigEndian, ByteOrder, LittleEndian};
use derive_more::IsVariant;

#[cfg(feature = "serde")]
use serde::Serialize;

use crate::{
    file::{self, Kind, ReadExt},
    Error::{self, *},
};

#[derive(Debug, Clone, PartialEq, IsVariant)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize),
    serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")
)]
pub enum Type<'a> {
    Void,
    Int {
        name: &'a str,
        size: usize,
        bits_offset: usize,
        nr_bits: usize,
        encoding: file::IntEncoding,
    },
    Ptr {
        type_id: u32,
    },
    Array {
        type_id: u32,
        index_type_id: u32,
        nr_elems: u32,
    },
    Struct {
        name: Option<&'a str>,
        size: usize,
        members: Vec<Member<'a>>,
    },
    Union {
        name: Option<&'a str>,
        size: usize,
        members: Vec<Member<'a>>,
    },
    Enum {
        name: Option<&'a str>,
        size: usize,
        values: Vec<Enum<'a>>,
    },
    Fwd {
        name: &'a str,
        fwd_kind: file::Kind,
    },
    Typedef {
        name: &'a str,
        type_id: u32,
    },
    Volatile {
        type_id: u32,
    },
    Const {
        type_id: u32,
    },
    Restrict {
        type_id: u32,
    },
    Func {
        name: &'a str,
        type_id: u32,
        linkage: file::Linkage,
    },
    FuncProto {
        ret_type_id: u32,
        params: Vec<Param<'a>>,
    },
    Variable {
        name: &'a str,
        type_id: u32,
        linkage: file::Linkage,
    },
    DataSec {
        name: &'a str,
        size: usize,
        sections: Vec<file::VarSectInfo>,
    },
    Float {
        name: &'a str,
        size: usize,
    },
    DeclTag {
        name: &'a str,
        type_id: u32,
        component_idx: i32,
    },
    TypeTag {
        name: &'a str,
        type_id: u32,
    },
}

impl<'a> Type<'a> {
    pub const VOID: Type<'a> = Type::Void;

    pub fn name(&self) -> Option<&str> {
        match *self {
            Type::Void => None,
            Type::Int { name, .. } => Some(name),
            Type::Ptr { .. } => None,
            Type::Array { .. } => None,
            Type::Struct { name, .. } => name,
            Type::Union { name, .. } => name,
            Type::Enum { name, .. } => name,
            Type::Fwd { name, .. } => Some(name),
            Type::Typedef { name, .. } => Some(name),
            Type::Volatile { .. } => None,
            Type::Const { .. } => None,
            Type::Restrict { .. } => None,
            Type::Func { name, .. } => Some(name),
            Type::FuncProto { .. } => None,
            Type::Variable { name, .. } => Some(name),
            Type::DataSec { name, .. } => Some(name),
            Type::Float { name, .. } => Some(name),
            Type::DeclTag { name, .. } => Some(name),
            Type::TypeTag { name, .. } => Some(name),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Member<'a> {
    pub name: Option<&'a str>,
    pub type_id: u32,
    pub bits_offset: u32,
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "size_is_zero"))]
    pub bitfield_size: u32,
}

#[cfg(feature = "serde")]
fn size_is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Enum<'a> {
    pub name: Option<&'a str>,
    pub val: u64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Param<'a> {
    pub name: Option<&'a str>,
    pub type_id: u32,
}

impl<'a> Param<'a> {
    pub fn has_variable_argument(params: &[Param]) -> bool {
        params.last().map_or(false, |p| p.is_variable_argument())
    }

    pub fn is_variable_argument(&self) -> bool {
        self.name.is_none() && self.type_id == 0
    }
}

pub struct Types<'a> {
    is_le: bool,
    types: untrusted::Reader<'a>,
    strs: untrusted::Input<'a>,
}

impl<'a> Types<'a> {
    pub fn parse(input: untrusted::Input<'a>) -> Result<Types<'a>, Error> {
        file::parse(input).map(|f| Types {
            is_le: f.header.is_le(),
            types: untrusted::Reader::new(f.types),
            strs: f.strs,
        })
    }
}

impl<'a> Iterator for Types<'a> {
    type Item = Result<Type<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.types.at_end() {
            None
        } else {
            let ty = if self.is_le {
                read_type::<LittleEndian>(&mut self.types, &self.strs)
            } else {
                read_type::<BigEndian>(&mut self.types, &self.strs)
            };

            Some(ty)
        }
    }
}

pub fn read_type<'a, O: ByteOrder>(
    r: &mut untrusted::Reader<'a>,
    strs: &untrusted::Input<'a>,
) -> Result<Type<'a>, Error> {
    let ty = file::Type::read::<O>(r)?;

    let name = file::read_str(strs, ty.name_off)?;

    Ok(match ty.kind() {
        Kind::Unknown => Type::Void,
        Kind::Integer => {
            let int = file::Int::read::<O>(r)?;

            Type::Int {
                name: name.ok_or(Expected("int name"))?,
                size: ty.size(),
                bits_offset: int.offset(),
                nr_bits: int.bits(),
                encoding: int.encoding(),
            }
        }
        Kind::Pointer => Type::Ptr {
            type_id: ty.type_id(),
        },
        Kind::Array => {
            let array = file::Array::read::<O>(r)?;

            Type::Array {
                type_id: array.ty,
                index_type_id: array.index_ty,
                nr_elems: array.nelems,
            }
        }
        Kind::Struct => Type::Struct {
            name,
            size: ty.size(),
            members: (0..ty.vlen())
                .map(|_| {
                    file::Member::read::<O>(r).and_then(|m| {
                        if ty.kflag() {
                            Ok(Member {
                                name: file::read_str(strs, m.name_off)?,
                                type_id: m.ty,
                                bits_offset: m.bit_offset(),
                                bitfield_size: m.bitfield_size(),
                            })
                        } else {
                            Ok(Member {
                                name: file::read_str(strs, m.name_off)?,
                                type_id: m.ty,
                                bits_offset: m.offset,
                                bitfield_size: 0,
                            })
                        }
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?,
        },
        Kind::Union => Type::Union {
            name,
            size: ty.size(),
            members: (0..ty.vlen())
                .map(|_| {
                    file::Member::read::<O>(r).and_then(|m| {
                        if ty.kflag() {
                            Ok(Member {
                                name: file::read_str(strs, m.name_off)?,
                                type_id: m.ty,
                                bits_offset: m.bit_offset(),
                                bitfield_size: m.bitfield_size(),
                            })
                        } else {
                            Ok(Member {
                                name: file::read_str(strs, m.name_off)?,
                                type_id: m.ty,
                                bits_offset: m.offset,
                                bitfield_size: 0,
                            })
                        }
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?,
        },
        Kind::Enum => Type::Enum {
            name,
            size: ty.size(),
            values: (0..ty.vlen())
                .map(|_| {
                    file::Enum::read::<O>(r).and_then(|v| {
                        Ok(Enum {
                            name: file::read_str(strs, v.name_off)?,
                            val: v.val as u64,
                        })
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?,
        },
        Kind::Enum64 => Type::Enum {
            name,
            size: ty.size(),
            values: (0..ty.vlen())
                .map(|_| {
                    file::Enum64::read::<O>(r).and_then(|v| {
                        Ok(Enum {
                            name: file::read_str(strs, v.name_off)?,
                            val: ((v.val_hi32 as u64) << 32) + (v.val_lo32 as u64),
                        })
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?,
        },
        Kind::Forward => Type::Fwd {
            name: name.ok_or(Expected("forward name"))?,
            fwd_kind: if ty.kflag() {
                Kind::Union
            } else {
                Kind::Struct
            },
        },
        Kind::Typedef => Type::Typedef {
            name: name.ok_or(Expected("typedef name"))?,
            type_id: ty.type_id(),
        },
        Kind::Volatile => Type::Volatile {
            type_id: ty.type_id(),
        },
        Kind::Const => Type::Const {
            type_id: ty.type_id(),
        },
        Kind::Restrict => Type::Restrict {
            type_id: ty.type_id(),
        },
        Kind::Func => Type::Func {
            name: name.ok_or(Expected("func name"))?,
            type_id: ty.type_id(),
            linkage: file::Linkage::from(ty.vlen() as u32),
        },
        Kind::FuncProto => Type::FuncProto {
            ret_type_id: ty.type_id(),
            params: (0..ty.vlen())
                .map(|_| {
                    file::Param::read::<O>(r).and_then(|p| {
                        Ok(Param {
                            name: file::read_str(strs, p.name_off)?,
                            type_id: p.ty,
                        })
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?,
        },
        Kind::Variable => Type::Variable {
            name: name.ok_or(Expected("var name"))?,
            type_id: ty.type_id(),
            linkage: file::Var::read::<O>(r)?.linkage,
        },
        Kind::DataSection => Type::DataSec {
            name: name.ok_or(Expected("datasec name"))?,
            size: ty.size(),
            sections: (0..ty.vlen())
                .map(|_| file::VarSectInfo::read::<O>(r))
                .collect::<Result<Vec<_>, Error>>()?,
        },
        Kind::Float => Type::Float {
            name: name.ok_or(Expected("float name"))?,
            size: ty.size(),
        },
        Kind::DeclTag => Type::DeclTag {
            name: name.ok_or(Expected("decl_tag name"))?,
            type_id: ty.type_id(),
            component_idx: file::DeclTag::read::<O>(r)?.component_idx,
        },
        Kind::TypeTag => Type::TypeTag {
            name: name.ok_or(Expected("type_tag name"))?,
            type_id: ty.type_id(),
        },
    })
}
