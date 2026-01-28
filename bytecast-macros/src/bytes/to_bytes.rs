//! ToBytes derive macro implementation.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

/// Derive the `ToBytes` trait for a struct or enum.
pub fn derive_to_bytes(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match derive_impl(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn derive_impl(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let (body, byte_len_body) = match &input.data {
        Data::Struct(data) => {
            let body = generate_struct(&data.fields)?;
            let byte_len = generate_byte_len_struct(&data.fields);
            (body, byte_len)
        }
        Data::Enum(data) => {
            if data.variants.len() > 256 {
                return Err(syn::Error::new_spanned(
                    input,
                    "ToBytes derive only supports enums with up to 256 variants",
                ));
            }
            let body = generate_enum(data)?;
            let byte_len = generate_byte_len_enum(data);
            (body, byte_len)
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "ToBytes derive is not supported for unions.",
            ));
        }
    };

    Ok(quote! {
        impl #impl_generics spill_ring_core::ToBytes for #name #ty_generics #where_clause {
            fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, spill_ring_core::BytesError> {
                let mut offset = 0usize;
                #body
                Ok(offset)
            }

            fn byte_len(&self) -> Option<usize> {
                #byte_len_body
            }
        }
    })
}

// =============================================================================
// Struct serialization
// =============================================================================

fn generate_struct(fields: &Fields) -> syn::Result<TokenStream2> {
    match fields {
        Fields::Named(named) => {
            let field_writes: Vec<_> = named
                .named
                .iter()
                .map(|f| {
                    let name = &f.ident;
                    quote! {
                        let written = spill_ring_core::ToBytes::to_bytes(&self.#name, &mut buf[offset..])?;
                        offset += written;
                    }
                })
                .collect();
            Ok(quote! { #(#field_writes)* })
        }
        Fields::Unnamed(unnamed) => {
            let field_writes: Vec<_> = unnamed
                .unnamed
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let index = syn::Index::from(i);
                    quote! {
                        let written = spill_ring_core::ToBytes::to_bytes(&self.#index, &mut buf[offset..])?;
                        offset += written;
                    }
                })
                .collect();
            Ok(quote! { #(#field_writes)* })
        }
        Fields::Unit => Ok(quote! {}),
    }
}

fn generate_byte_len_struct(fields: &Fields) -> TokenStream2 {
    match fields {
        Fields::Named(named) => {
            let field_lens: Vec<_> = named
                .named
                .iter()
                .map(|f| {
                    let name = &f.ident;
                    quote! { spill_ring_core::ToBytes::byte_len(&self.#name)? }
                })
                .collect();

            if field_lens.is_empty() {
                quote! { Some(0) }
            } else {
                quote! { Some(0 #(+ #field_lens)*) }
            }
        }
        Fields::Unnamed(unnamed) => {
            let field_lens: Vec<_> = unnamed
                .unnamed
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let index = syn::Index::from(i);
                    quote! { spill_ring_core::ToBytes::byte_len(&self.#index)? }
                })
                .collect();

            if field_lens.is_empty() {
                quote! { Some(0) }
            } else {
                quote! { Some(0 #(+ #field_lens)*) }
            }
        }
        Fields::Unit => quote! { Some(0) },
    }
}

// =============================================================================
// Enum serialization
// =============================================================================

fn generate_enum(data: &syn::DataEnum) -> syn::Result<TokenStream2> {
    let match_arms: Vec<_> = data
        .variants
        .iter()
        .enumerate()
        .map(|(idx, variant)| {
            let variant_name = &variant.ident;
            let discriminant = idx as u8;

            match &variant.fields {
                Fields::Unit => {
                    quote! {
                        Self::#variant_name => {
                            let written = spill_ring_core::ToBytes::to_bytes(&#discriminant, &mut buf[offset..])?;
                            offset += written;
                        }
                    }
                }
                Fields::Unnamed(fields) => {
                    let field_names: Vec<_> = (0..fields.unnamed.len())
                        .map(|i| {
                            syn::Ident::new(&format!("f{}", i), proc_macro2::Span::call_site())
                        })
                        .collect();
                    let field_writes: Vec<_> = field_names
                        .iter()
                        .map(|name| {
                            quote! {
                                let written = spill_ring_core::ToBytes::to_bytes(#name, &mut buf[offset..])?;
                                offset += written;
                            }
                        })
                        .collect();

                    quote! {
                        Self::#variant_name(#(#field_names),*) => {
                            let written = spill_ring_core::ToBytes::to_bytes(&#discriminant, &mut buf[offset..])?;
                            offset += written;
                            #(#field_writes)*
                        }
                    }
                }
                Fields::Named(fields) => {
                    let field_names: Vec<_> = fields.named.iter().map(|f| &f.ident).collect();
                    let field_writes: Vec<_> = field_names
                        .iter()
                        .map(|name| {
                            quote! {
                                let written = spill_ring_core::ToBytes::to_bytes(#name, &mut buf[offset..])?;
                                offset += written;
                            }
                        })
                        .collect();

                    quote! {
                        Self::#variant_name { #(#field_names),* } => {
                            let written = spill_ring_core::ToBytes::to_bytes(&#discriminant, &mut buf[offset..])?;
                            offset += written;
                            #(#field_writes)*
                        }
                    }
                }
            }
        })
        .collect();

    Ok(quote! {
        match self {
            #(#match_arms)*
        }
    })
}

fn generate_byte_len_enum(data: &syn::DataEnum) -> TokenStream2 {
    let match_arms: Vec<_> = data
        .variants
        .iter()
        .map(|variant| {
            let variant_name = &variant.ident;

            match &variant.fields {
                Fields::Unit => {
                    quote! { Self::#variant_name => Some(1) }
                }
                Fields::Unnamed(fields) => {
                    let field_names: Vec<_> = (0..fields.unnamed.len())
                        .map(|i| {
                            syn::Ident::new(&format!("f{}", i), proc_macro2::Span::call_site())
                        })
                        .collect();
                    let field_lens: Vec<_> = field_names
                        .iter()
                        .map(|name| quote! { spill_ring_core::ToBytes::byte_len(#name)? })
                        .collect();

                    quote! { Self::#variant_name(#(#field_names),*) => Some(1 #(+ #field_lens)*) }
                }
                Fields::Named(fields) => {
                    let field_names: Vec<_> = fields.named.iter().map(|f| &f.ident).collect();
                    let field_lens: Vec<_> = field_names
                        .iter()
                        .map(|name| quote! { spill_ring_core::ToBytes::byte_len(#name)? })
                        .collect();

                    quote! { Self::#variant_name { #(#field_names),* } => Some(1 #(+ #field_lens)*) }
                }
            }
        })
        .collect();

    quote! {
        match self {
            #(#match_arms),*
        }
    }
}
