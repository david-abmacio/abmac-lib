//! Derive macro for pebble's `Checkpointable` trait.

use proc_macro::TokenStream;
mod checkpoint;

/// Derive `Checkpointable` for a struct.
///
/// Exactly one field must be annotated with `#[checkpoint(id)]` to designate
/// the checkpoint identifier. The generated implementation returns `Ok(base)`
/// from `compute_from_dependencies` — types with custom rebuild logic should
/// implement the trait manually.
///
/// # Example
///
/// ```ignore
/// #[derive(Clone, Checkpoint)]
/// struct MyState {
///     #[checkpoint(id)]
///     id: u64,
///     data: String,
/// }
/// ```
#[proc_macro_derive(Checkpoint, attributes(checkpoint))]
pub fn derive_checkpoint(input: TokenStream) -> TokenStream {
    checkpoint::derive_checkpoint(input)
}
