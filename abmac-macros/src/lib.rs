//! ABMAC derive macros.
//!
//! Enable feature flags to pull in derive macros:
//!
//! - `bytecast` — `ToBytes`, `FromBytes`
//! - `pebble` — `Checkpoint`

#[cfg(any(feature = "bytecast", feature = "pebble"))]
mod bytes;
#[cfg(feature = "pebble")]
mod checkpoint;

/// Derive `ToBytes`.
#[cfg(feature = "bytecast")]
#[proc_macro_derive(ToBytes, attributes(bytecast))]
pub fn derive_to_bytes(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    match bytes::derive_to_bytes_impl(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Derive `FromBytes`.
#[cfg(feature = "bytecast")]
#[proc_macro_derive(FromBytes, attributes(bytecast))]
pub fn derive_from_bytes(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    match bytes::derive_from_bytes_impl(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Derive `Checkpointable` (plus `ToBytes` and `FromBytes`).
#[cfg(feature = "pebble")]
#[proc_macro_derive(Checkpoint, attributes(checkpoint, bytecast))]
pub fn derive_checkpoint(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    checkpoint::derive_checkpoint(input)
}
