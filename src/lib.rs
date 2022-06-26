#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

mod error;
pub mod file;
mod ty;

pub use self::error::Error;
pub use self::ty::Type;

pub fn parse(b: &[u8]) -> Result<self::ty::Types, Error> {
    self::ty::Types::parse(untrusted::Input::from(b))
}
