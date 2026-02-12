//! ToBytes derive macro implementation.

use super::{disc_capacity, has_boxed_attr, has_skip_attr, repr_int_type, serializable_type};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

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

    let (body, byte_len_body, max_size_body) = match &input.data {
        Data::Struct(data) => (
            generate_struct(&data.fields),
            generate_byte_len_struct(&data.fields),
            generate_max_size_struct(&data.fields),
        ),
        Data::Enum(data) => {
            let disc_ident = validate_enum(input, data)?;
            (
                generate_enum(data, &disc_ident),
                generate_byte_len_enum(data, &disc_ident),
                generate_max_size_enum(data, &disc_ident),
            )
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "ToBytes derive is not supported for unions.",
            ));
        }
    };

    Ok(quote! {
        impl #impl_generics bytecast::ToBytes for #name #ty_generics #where_clause {
            const MAX_SIZE: Option<usize> = #max_size_body;

            fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, bytecast::BytesError> {
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

fn validate_enum(input: &DeriveInput, data: &syn::DataEnum) -> syn::Result<syn::Ident> {
    let disc_ident = repr_int_type(&input.attrs)
        .unwrap_or_else(|| syn::Ident::new("u8", proc_macro2::Span::call_site()));
    let max_variants = disc_capacity(&disc_ident.to_string());
    if data.variants.len() > max_variants {
        return Err(syn::Error::new_spanned(
            input,
            format!(
                "enum has {} variants but discriminant type `{}` supports at most {}. \
                 Add #[repr(u16)], #[repr(u32)], etc. to increase capacity.",
                data.variants.len(),
                disc_ident,
                max_variants,
            ),
        ));
    }
    Ok(disc_ident)
}

/// Returns the accessor token for a field (e.g. `self.name` or `self.0`),
/// with deref for boxed fields.
fn field_accessor(field: &syn::Field, index: usize) -> TokenStream2 {
    let access = match &field.ident {
        Some(name) => quote! { self.#name },
        None => {
            let idx = syn::Index::from(index);
            quote! { self.#idx }
        }
    };
    if has_boxed_attr(field) {
        quote! { *#access }
    } else {
        access
    }
}

/// Iterates non-skipped fields, providing each field and its positional index.
fn active_fields(fields: &Fields) -> Vec<(usize, &syn::Field)> {
    match fields {
        Fields::Named(named) => named
            .named
            .iter()
            .enumerate()
            .filter(|(_, f)| !has_skip_attr(f))
            .collect(),
        Fields::Unnamed(unnamed) => unnamed
            .unnamed
            .iter()
            .enumerate()
            .filter(|(_, f)| !has_skip_attr(f))
            .collect(),
        Fields::Unit => vec![],
    }
}

fn generate_struct(fields: &Fields) -> TokenStream2 {
    let writes: Vec<_> = active_fields(fields)
        .iter()
        .map(|&(i, f)| {
            let access = field_accessor(f, i);
            quote! {
                let written = bytecast::ToBytes::to_bytes(&#access, &mut buf[offset..])?;
                offset += written;
            }
        })
        .collect();
    quote! { #(#writes)* }
}

fn generate_byte_len_struct(fields: &Fields) -> TokenStream2 {
    let fields = active_fields(fields);
    if fields.is_empty() {
        return quote! { Some(0) };
    }
    let lens: Vec<_> = fields
        .iter()
        .map(|&(i, f)| {
            let access = field_accessor(f, i);
            quote! { bytecast::ToBytes::byte_len(&#access)? }
        })
        .collect();
    quote! { Some(0 #(+ #lens)*) }
}

fn generate_max_size_struct(fields: &Fields) -> TokenStream2 {
    let fields = active_fields(fields);
    if fields.is_empty() {
        return quote! { Some(0) };
    }
    let sizes: Vec<_> = fields
        .iter()
        .map(|&(_, f)| {
            let ty = serializable_type(f);
            quote! { <#ty as bytecast::ToBytes>::MAX_SIZE }
        })
        .collect();
    quote! {
        {
            const fn compute_max_size() -> Option<usize> {
                let mut total = 0usize;
                #(
                    match #sizes {
                        Some(s) => total += s,
                        None => return None,
                    }
                )*
                Some(total)
            }
            compute_max_size()
        }
    }
}

fn generate_enum(data: &syn::DataEnum, disc_type: &syn::Ident) -> TokenStream2 {
    let match_arms: Vec<_> = data
        .variants
        .iter()
        .enumerate()
        .map(|(idx, variant)| {
            let variant_name = &variant.ident;
            let idx_lit = syn::LitInt::new(&idx.to_string(), proc_macro2::Span::call_site());
            let disc_write = quote! {
                let written = bytecast::ToBytes::to_bytes(&(#idx_lit as #disc_type), &mut buf[offset..])?;
                offset += written;
            };

            match &variant.fields {
                Fields::Unit => quote! {
                    Self::#variant_name => { #disc_write }
                },
                Fields::Unnamed(fields) => {
                    let names = enum_field_names(fields.unnamed.len());
                    let writes = enum_field_writes(&names);
                    quote! {
                        Self::#variant_name(#(#names),*) => {
                            #disc_write
                            #(#writes)*
                        }
                    }
                }
                Fields::Named(fields) => {
                    let names: Vec<_> = fields.named.iter().map(|f| &f.ident).collect();
                    let writes = enum_field_writes(&names);
                    quote! {
                        Self::#variant_name { #(#names),* } => {
                            #disc_write
                            #(#writes)*
                        }
                    }
                }
            }
        })
        .collect();

    quote! { match self { #(#match_arms)* } }
}

fn generate_byte_len_enum(data: &syn::DataEnum, disc_type: &syn::Ident) -> TokenStream2 {
    let match_arms: Vec<_> = data
        .variants
        .iter()
        .map(|variant| {
            let variant_name = &variant.ident;
            match &variant.fields {
                Fields::Unit => {
                    quote! { Self::#variant_name => Some(core::mem::size_of::<#disc_type>()) }
                }
                Fields::Unnamed(fields) => {
                    let names = enum_field_names(fields.unnamed.len());
                    let lens = enum_field_lens(&names);
                    quote! { Self::#variant_name(#(#names),*) => Some(core::mem::size_of::<#disc_type>() #(+ #lens)*) }
                }
                Fields::Named(fields) => {
                    let names: Vec<_> = fields.named.iter().map(|f| &f.ident).collect();
                    let lens = enum_field_lens(&names);
                    quote! { Self::#variant_name { #(#names),* } => Some(core::mem::size_of::<#disc_type>() #(+ #lens)*) }
                }
            }
        })
        .collect();

    quote! { match self { #(#match_arms),* } }
}

fn generate_max_size_enum(data: &syn::DataEnum, disc_type: &syn::Ident) -> TokenStream2 {
    if data.variants.is_empty() {
        return quote! { Some(core::mem::size_of::<#disc_type>()) };
    }

    let variant_sizes: Vec<Vec<_>> = data
        .variants
        .iter()
        .map(|variant| {
            let fields = match &variant.fields {
                Fields::Named(f) => f.named.iter().collect::<Vec<_>>(),
                Fields::Unnamed(f) => f.unnamed.iter().collect::<Vec<_>>(),
                Fields::Unit => vec![],
            };
            fields
                .iter()
                .map(|f| {
                    let ty = &f.ty;
                    quote! { <#ty as bytecast::ToBytes>::MAX_SIZE }
                })
                .collect()
        })
        .collect();

    quote! {
        {
            const fn compute_max_size() -> Option<usize> {
                let mut max = 0usize;
                #({
                    let mut variant_size = 0usize;
                    #(
                        match #variant_sizes {
                            Some(s) => variant_size += s,
                            None => return None,
                        }
                    )*
                    if variant_size > max {
                        max = variant_size;
                    }
                })*
                Some(core::mem::size_of::<#disc_type>() + max)
            }
            compute_max_size()
        }
    }
}

fn enum_field_names(count: usize) -> Vec<syn::Ident> {
    (0..count)
        .map(|i| syn::Ident::new(&format!("f{}", i), proc_macro2::Span::call_site()))
        .collect()
}

fn enum_field_writes<T: quote::ToTokens>(names: &[T]) -> Vec<TokenStream2> {
    names
        .iter()
        .map(|name| {
            quote! {
                let written = bytecast::ToBytes::to_bytes(#name, &mut buf[offset..])?;
                offset += written;
            }
        })
        .collect()
}

fn enum_field_lens<T: quote::ToTokens>(names: &[T]) -> Vec<TokenStream2> {
    names
        .iter()
        .map(|name| quote! { bytecast::ToBytes::byte_len(#name)? })
        .collect()
}
