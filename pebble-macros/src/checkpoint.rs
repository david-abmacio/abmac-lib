//! Checkpoint derive macro implementation.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

pub fn derive_checkpoint(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match derive_impl(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn derive_impl(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;

    let data = match &input.data {
        Data::Struct(data) => data,
        Data::Enum(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "Checkpoint can only be derived for structs",
            ));
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "Checkpoint can only be derived for structs",
            ));
        }
    };

    let fields = match &data.fields {
        Fields::Named(fields) => fields,
        Fields::Unnamed(_) | Fields::Unit => {
            return Err(syn::Error::new_spanned(
                input,
                "Checkpoint requires named fields (use `#[checkpoint(id)]` on the ID field)",
            ));
        }
    };

    // Find the single #[checkpoint(id)] field.
    let mut id_field = None;
    for field in &fields.named {
        if has_checkpoint_id_attr(field)? {
            if id_field.is_some() {
                return Err(syn::Error::new_spanned(
                    field,
                    "only one field may be annotated with `#[checkpoint(id)]`",
                ));
            }
            id_field = Some(field);
        }
    }

    let id_field = id_field.ok_or_else(|| {
        syn::Error::new_spanned(
            input,
            "missing `#[checkpoint(id)]` annotation on exactly one field",
        )
    })?;

    let id_field_name = id_field.ident.as_ref().unwrap();
    let id_field_type = &id_field.ty;

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Generate Checkpointable impl.
    let checkpointable_impl = quote! {
        impl #impl_generics ::pebble::Checkpointable for #name #ty_generics #where_clause {
            type Id = #id_field_type;
            type RebuildError = ();

            fn checkpoint_id(&self) -> Self::Id {
                self.#id_field_name
            }

            fn compute_from_dependencies(
                base: Self,
                _deps: &::hashbrown::HashMap<Self::Id, &Self>,
            ) -> ::core::result::Result<Self, Self::RebuildError> {
                Ok(base)
            }
        }
    };

    // Generate ToBytes + FromBytes via bytecast so BytecastSerializer works automatically.
    let to_bytes_tokens = bytecast_macros_impl::bytes::derive_to_bytes_impl(input)?;
    let from_bytes_tokens = bytecast_macros_impl::bytes::derive_from_bytes_impl(input)?;

    Ok(quote! {
        #checkpointable_impl
        #to_bytes_tokens
        #from_bytes_tokens
    })
}

/// Check if a field has `#[checkpoint(id)]`.
fn has_checkpoint_id_attr(field: &syn::Field) -> syn::Result<bool> {
    for attr in &field.attrs {
        if !attr.path().is_ident("checkpoint") {
            continue;
        }
        let mut found_id = false;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("id") {
                found_id = true;
                Ok(())
            } else {
                Err(meta.error("unknown checkpoint attribute; expected `id`"))
            }
        })?;
        if found_id {
            return Ok(true);
        }
    }
    Ok(false)
}
