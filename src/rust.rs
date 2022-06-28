use core::cell::RefCell;

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        use std::{borrow::Cow, rc::Rc};
    } else {
        use alloc::{borrow::Cow, rc::Rc};
    }
}

use check_keyword::CheckKeyword;
use derive_new::new;
use proc_macro2::{Ident, Literal, Span, TokenStream};
use quote::{quote, ToTokens, TokenStreamExt};

use crate::{
    ty,
    Error::{self, *},
    Kind, Type,
};

trait EscapeKeyword {
    fn escape_keyword(&self) -> Cow<str>;
}

impl EscapeKeyword for str {
    fn escape_keyword(&self) -> Cow<str> {
        if self.is_keyword() {
            format!("_{}", self).into()
        } else {
            self.into()
        }
    }
}

#[derive(new)]
struct TypeFmt<'a> {
    types: &'a Types<'a>,
    ns: Rc<RefCell<Namespace>>,
    type_id: u32,
}

impl<'a> ToTokens for TypeFmt<'a> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let ty = self.types.get_type(self.type_id).expect("type");

        tokens.append_all(match *ty {
            Type::Void => quote! { c_void },
            Type::Int {
                name,
                size,
                bits_offset,
                nr_bits,
                encoding,
            } => {
                if encoding.is_bool() {
                    quote! { bool }
                } else if bits_offset == 0 {
                    let ident = Ident::new(
                        &format!(
                            "{}{}",
                            if encoding.is_signed() { "i" } else { "u" },
                            size * 8
                        ),
                        Span::call_site(),
                    );

                    quote! { #ident }
                } else {
                    let ident = Ident::new(name, Span::call_site());

                    quote! { #ident }
                }
            }
            Type::Float { size, .. } => {
                let ident = Ident::new(&format!("f{}", size * 8), Span::call_site());

                quote! { #ident }
            }
            Type::Ptr { type_id, .. } => {
                let ty = self.types.get_type(type_id).expect("pointee type");

                match ty {
                    Type::Const { type_id } => {
                        let t = TypeFmt::new(self.types, self.ns.clone(), *type_id);

                        quote! {
                            *const #t
                        }
                    }
                    Type::FuncProto {
                        ret_type_id,
                        params,
                    } => {
                        let f = FuncProto::new(self.types, self.ns.clone(), *ret_type_id, params);

                        quote! {
                            ::core::option::Option<unsafe extern "C" fn #f>
                        }
                    }
                    _ => {
                        let t = TypeFmt::new(self.types, self.ns.clone(), type_id);

                        quote! {
                            *mut #t
                        }
                    }
                }
            }
            Type::Array {
                type_id, nr_elems, ..
            } => {
                let t = TypeFmt::new(self.types, self.ns.clone(), type_id);
                let n = Literal::u32_unsuffixed(nr_elems);

                quote! { [#t; #n] }
            }
            Type::Struct { name, .. } | Type::Union { name, .. } | Type::Enum { name, .. }
                if name.is_some() =>
            {
                let ident = Ident::new(
                    &self
                        .ns
                        .borrow()
                        .get_name(self.type_id)
                        .or(name)
                        .expect("name")
                        .escape_keyword(),
                    Span::call_site(),
                );

                quote! { #ident }
            }

            Type::Struct { name, .. } if name.is_none() => {
                let ident =
                    Ident::new(&StructDecl::anon_type_name(self.type_id), Span::call_site());

                quote! { #ident }
            }
            Type::Union { name, .. } if name.is_none() => {
                let ident = Ident::new(&UnionDecl::anon_type_name(self.type_id), Span::call_site());

                quote! { #ident }
            }
            Type::Enum { name, .. } if name.is_none() => {
                let ident = Ident::new(&EnumDecl::anon_type_name(self.type_id), Span::call_site());

                quote! { #ident }
            }
            Type::Typedef { name, .. } | Type::Fwd { name, .. } => {
                let ident = Ident::new(name, Span::call_site());

                quote! { #ident }
            }
            Type::Const { type_id } | Type::Volatile { type_id } | Type::Restrict { type_id } => {
                let t = TypeFmt::new(self.types, self.ns.clone(), type_id);

                quote! { #t }
            }
            Type::FuncProto {
                ret_type_id,
                ref params,
            } => {
                let f = FuncProto::new(self.types, self.ns.clone(), ret_type_id, params);

                quote! { fn #f }
            }
            _ => quote! {},
        })
    }
}

#[derive(new)]
struct TypeDecl<'a> {
    types: &'a Types<'a>,
    ns: Rc<RefCell<Namespace>>,
    type_id: u32,
    ty: &'a Type<'a>,
}

impl<'a> TypeDecl<'a> {
    const BUILDIN_TYPES: [&'static str; 13] = [
        "bool", "i8", "u8", "i16", "u16", "i32", "u32", "i64", "u64", "i128", "u128", "f32", "f64",
    ];
}

impl<'a> ToTokens for TypeDecl<'a> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.append_all(match *self.ty {
            Type::Int { name, encoding, .. } if encoding.is_bool() => {
                let ident = Ident::new(&name.escape_keyword(), Span::call_site());

                Some(quote! {
                    pub type #ident = bool;
                })
            }
            Type::Float { name, size } => {
                let ident = Ident::new(&name.escape_keyword(), Span::call_site());
                let t = Ident::new(&format!("f{}", size * 8), Span::call_site());

                Some(quote! {
                    pub type #ident = #t;
                })
            }
            Type::Struct {
                name, ref members, ..
            } => {
                let name = name.map_or_else(
                    || StructDecl::anon_type_name(self.type_id),
                    |s| s.escape_keyword(),
                );
                let name = self.ns.borrow_mut().get_unique_name(&name, self.type_id);

                let s = StructDecl::new(self.types, self.ns.clone(), &name, members);

                Some(quote! { #s })
            }
            Type::Union {
                name, ref members, ..
            } => {
                let name = name.map_or_else(
                    || UnionDecl::anon_type_name(self.type_id),
                    |s| s.escape_keyword(),
                );
                let name = self.ns.borrow_mut().get_unique_name(&name, self.type_id);
                let u = UnionDecl::new(self.types, self.ns.clone(), &name, members);

                Some(quote! { #u })
            }
            Type::Enum {
                name,
                size,
                ref values,
            } => {
                let name = name.map_or_else(
                    || EnumDecl::anon_type_name(self.type_id),
                    |s| s.escape_keyword(),
                );
                let name = self.ns.borrow_mut().get_unique_name(&name, self.type_id);
                let e = EnumDecl::new(&name, size, values);

                Some(quote! { #e })
            }
            Type::Fwd { name, fwd_kind } => {
                let fwd_name = name;

                let found = self.types.types.iter().any(|t| match t {
                    Type::Struct { name, .. } if fwd_kind == Kind::Struct => {
                        name.unwrap_or_default() == fwd_name
                    }
                    Type::Union { name, .. } if fwd_kind == Kind::Union => {
                        name.unwrap_or_default() == fwd_name
                    }
                    _ => false,
                });

                if found {
                    None
                } else {
                    let ident = Ident::new(fwd_name, Span::call_site());

                    Some(quote! {
                        pub type #ident = c_void;
                    })
                }
            }
            Type::Typedef { name, type_id } => {
                if Self::BUILDIN_TYPES.contains(&name) {
                    None
                } else {
                    let inner = self.types.get_type(type_id).expect("typedef");

                    let ignore = match inner.name() {
                        Some(inner_name) if inner_name == name => true,
                        _ => false,
                    };

                    if ignore {
                        None
                    } else {
                        let name = name.escape_keyword();
                        let name = self.ns.borrow_mut().get_unique_name(&name, self.type_id);

                        let t = TypedefDecl::new(self.types, self.ns.clone(), &name, type_id);

                        Some(quote! { #t })
                    }
                }
            }
            Type::Func { name, type_id, .. } => {
                let f = FuncDecl::new(self.types, self.ns.clone(), name, type_id);

                Some(quote! { #f })
            }
            _ => None,
        })
    }
}

#[derive(new)]
struct TypedefDecl<'a> {
    types: &'a Types<'a>,
    ns: Rc<RefCell<Namespace>>,
    name: &'a str,
    type_id: u32,
}

impl<'a> ToTokens for TypedefDecl<'a> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let ident = Ident::new(&self.name.escape_keyword(), Span::call_site());
        let t = TypeFmt::new(self.types, self.ns.clone(), self.type_id);

        tokens.append_all(quote! {
            pub type #ident = #t;
        })
    }
}

fn anon_type_name<'a>(ty: &str, id: u32) -> Cow<'a, str> {
    format!("_anon_{}_{}", ty, id).into()
}

#[derive(new)]
struct StructDecl<'a> {
    types: &'a Types<'a>,
    ns: Rc<RefCell<Namespace>>,
    name: &'a str,
    members: &'a [ty::Member<'a>],
}

impl<'a> StructDecl<'a> {
    pub fn anon_type_name(id: u32) -> Cow<'a, str> {
        anon_type_name("struct", id)
    }

    pub fn anon_field_name(id: usize) -> Cow<'a, str> {
        anon_type_name("field", id as u32)
    }
}

impl<'a> ToTokens for StructDecl<'a> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let ident = Ident::new(&self.name.escape_keyword(), Span::call_site());
        let members = self.members.iter().enumerate().map(|(i, m)| {
            let name = m
                .name
                .map_or_else(|| Self::anon_field_name(i), EscapeKeyword::escape_keyword);
            let ident = Ident::new(&name, Span::call_site());
            let ty = TypeFmt::new(self.types, self.ns.clone(), m.type_id);

            quote! {
                pub #ident: #ty,
            }
        });

        tokens.append_all(quote! {
            #[repr(C)]
            #[derive(Clone, Copy)]
            pub struct #ident {
                #(#members)*
            }
        })
    }
}

#[derive(new)]
struct UnionDecl<'a> {
    types: &'a Types<'a>,
    ns: Rc<RefCell<Namespace>>,
    name: &'a str,
    members: &'a [ty::Member<'a>],
}

impl<'a> UnionDecl<'a> {
    pub fn anon_type_name(id: u32) -> Cow<'a, str> {
        anon_type_name("union", id)
    }

    pub fn anon_field_name(id: usize) -> Cow<'a, str> {
        anon_type_name("field", id as u32)
    }
}

impl<'a> ToTokens for UnionDecl<'a> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let ident = Ident::new(&self.name.escape_keyword(), Span::call_site());
        let members = self.members.iter().enumerate().map(|(i, m)| {
            let name = m
                .name
                .map_or_else(|| Self::anon_field_name(i), EscapeKeyword::escape_keyword);
            let field = Ident::new(&name, Span::call_site());
            let t = TypeFmt::new(self.types, self.ns.clone(), m.type_id);

            quote! {
                pub #field: core::mem::ManuallyDrop<#t>,
            }
        });

        tokens.append_all(quote! {
            #[repr(C)]
            #[derive(Clone, Copy)]
            pub union #ident {
                #(#members)*
            }
        })
    }
}

#[derive(new)]
struct EnumDecl<'a> {
    name: &'a str,
    size: usize,
    values: &'a [ty::Enum<'a>],
}

impl<'a> EnumDecl<'a> {
    pub fn anon_type_name(id: u32) -> Cow<'a, str> {
        anon_type_name("enum", id)
    }
}

impl<'a> ToTokens for EnumDecl<'a> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let repr = if self.values.is_empty() {
            None
        } else {
            let ty = Ident::new(&format!("u{}", self.size * 8), Span::call_site());

            Some(quote! { #[repr(#ty)] })
        };
        let ident = Ident::new(&self.name.escape_keyword(), Span::call_site());
        let mut consts = Vec::new();
        let values = self
            .values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let name = v.name.map_or_else(
                    || Self::anon_type_name(i as u32),
                    EscapeKeyword::escape_keyword,
                );
                let val_ident = Ident::new(&name, Span::call_site());

                if let Some(e) = self.values.iter().take(i).find(|e| e.val == v.val) {
                    let val =
                        Ident::new(&e.name.expect("name").escape_keyword(), Span::call_site());

                    consts.push(quote! {
                        pub const #val_ident: Self = Self::#val;
                    });

                    None
                } else {
                    let val = Literal::u64_unsuffixed(v.val);

                    Some(quote! {
                        #val_ident = #val,
                    })
                }
            })
            .collect::<Vec<_>>();

        let impl_enum = if consts.is_empty() {
            None
        } else {
            Some(quote! {
                impl #ident {
                    #(#consts)*
                }
            })
        };

        tokens.append_all(quote! {
            #repr
            #[derive(Debug, Clone, Copy)]
            pub enum #ident {
                #(#values)*
            }

            #impl_enum
        })
    }
}

#[derive(new)]
struct FuncDecl<'a> {
    types: &'a Types<'a>,
    ns: Rc<RefCell<Namespace>>,
    name: &'a str,
    proto_type_id: u32,
}

impl<'a> ToTokens for FuncDecl<'a> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let ident = Ident::new(&self.name.escape_keyword(), Span::call_site());

        let proto = if let Type::FuncProto {
            ret_type_id,
            params,
        } = self.types.get_type(self.proto_type_id).expect("ret type")
        {
            Some(FuncProto::new(
                self.types,
                self.ns.clone(),
                *ret_type_id,
                params,
            ))
        } else {
            None
        };

        tokens.append_all(quote! {
            extern "C" {
                pub fn #ident #proto;
            }
        })
    }
}

#[derive(new)]
struct FuncProto<'a> {
    types: &'a Types<'a>,
    ns: Rc<RefCell<Namespace>>,
    ret_type: u32,
    params: &'a [ty::Param<'a>],
}

impl<'a> ToTokens for FuncProto<'a> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let params = self.params.iter().map(|p| {
            if p.is_variable_argument() {
                quote! { ... }
            } else if let Some(name) = p.name {
                let ident = Ident::new(&name.escape_keyword(), Span::call_site());
                let t = TypeFmt::new(self.types, self.ns.clone(), p.type_id);

                quote! { #ident: #t }
            } else {
                let t = TypeFmt::new(self.types, self.ns.clone(), p.type_id);

                quote! { #t }
            }
        });

        let ret = if self.ret_type != 0 {
            let t = TypeFmt::new(self.types, self.ns.clone(), self.ret_type);

            Some(quote! { -> #t })
        } else {
            None
        };

        tokens.append_all(quote! {
            (#(#params),*) #ret
        })
    }
}

#[derive(Default)]
pub struct Namespace {
    pub names: Vec<String>,
    pub name_by_id: Vec<String>,
}

impl Namespace {
    pub fn get_name(&self, type_id: u32) -> Option<&str> {
        self.name_by_id
            .get((type_id - 1) as usize)
            .map(|s| s.as_str())
    }

    pub fn get_unique_name(&mut self, name: &str, type_id: u32) -> String {
        if let Some(s) = self.name_by_id.get((type_id - 1) as usize) {
            s.clone()
        } else {
            let idx = match self.names.binary_search(&name.to_owned()) {
                Ok(_) => {
                    let name = format!("{}_{}", name, type_id);
                    let idx = self
                        .names
                        .binary_search(&name)
                        .expect_err("name already exists");
                    self.names.insert(idx, name);
                    idx
                }
                Err(idx) => {
                    self.names.insert(idx, name.to_owned());
                    idx
                }
            };

            let s = self.names.get(idx).expect("name");

            self.name_by_id.push(s.clone());

            s.clone()
        }
    }
}

#[derive(new)]
pub struct Types<'a> {
    pub base: Option<&'a [Type<'a>]>,
    pub types: &'a [Type<'a>],
    #[new(value = "2021")]
    pub edition: usize,
    #[new(value = "true")]
    pub core_ffi: bool,
}

impl<'a> Types<'a> {
    pub fn get_type(&self, type_id: u32) -> Result<&Type<'a>, Error> {
        if type_id == 0 {
            return Ok(&Type::VOID);
        }

        let start_id = self
            .base
            .as_ref()
            .map(|v| v.len() as u32)
            .unwrap_or_default()
            + 1;

        let types = if type_id < start_id {
            self.base.as_ref().expect("base")
        } else {
            &self.types
        };

        types
            .get((type_id - start_id) as usize)
            .ok_or(OutOfRange("type_id", type_id as u64))
    }

    pub fn find_type<F>(&self, f: F) -> Option<&Type<'a>>
    where
        F: FnMut(&&Type<'a>) -> bool,
    {
        self.types.iter().find(f)
    }
}

impl<'a> ToTokens for Types<'a> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let half_crate = if self.edition < 2018 {
            Some(quote! {
                extern crate half;
            })
        } else {
            None
        };

        let use_c_void = if self.core_ffi {
            quote! {
                use core::ffi::c_void;
            }
        } else {
            quote! {
                use ::libc::c_void;
            }
        };

        let f16_decl = if cfg!(feature = "half") {
            quote! {
                use half::f16;
            }
        } else {
            quote! {
                pub type f16 = i16;
            }
        };

        let ns = Rc::new(RefCell::new(Namespace::default()));

        let types = self.types.iter().enumerate().map(|(idx, ty)| {
            let t = TypeDecl::new(self, ns.clone(), (idx + 1) as u32, ty);

            quote! {
                #t
            }
        });

        tokens.append_all(quote! {
            #![allow(non_camel_case_types)]
            #![allow(non_upper_case_globals)]

            #half_crate
            #use_c_void
            #f16_decl

            #(#types)*
        })
    }
}

pub fn dump<'a>(base: Option<&'a [Type<'a>]>, types: &'a [Type<'a>]) -> String {
    Types::new(base, types).into_token_stream().to_string()
}
