//! Derive macros for bytecast.

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

/// Derive `ToBytes`.
#[proc_macro_derive(ToBytes, attributes(bytecast))]
pub fn derive_to_bytes(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match bytecast_macros_impl::bytes::derive_to_bytes_impl(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Derive `FromBytes`.
#[proc_macro_derive(FromBytes, attributes(bytecast))]
pub fn derive_from_bytes(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match bytecast_macros_impl::bytes::derive_from_bytes_impl(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}
