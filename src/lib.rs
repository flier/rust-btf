#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

mod error;
pub mod file;
pub mod ty;

#[cfg(feature = "rust")]
pub mod rust;

pub use self::error::Error;
pub use self::ty::{Type, Types};
pub use self::file::Kind;

pub fn parse(b: &[u8]) -> Result<self::Types, Error> {
    self::Types::parse(untrusted::Input::from(b))
}
