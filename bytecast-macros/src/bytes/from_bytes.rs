//! FromBytes derive macro implementation.

use super::{disc_capacity, has_boxed_attr, has_skip_attr, repr_int_type, serializable_type};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

pub fn derive_from_bytes(input: TokenStream) -> TokenStream {
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

    let body = match &input.data {
        Data::Struct(data) => {
            let (reads, constructor) = generate_struct(name, &data.fields);
            quote! {
                #reads
                Ok((#constructor, offset))
            }
        }
        Data::Enum(data) => {
            let disc_ident = validate_enum(input, data)?;
            generate_enum(name, data, &disc_ident)
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "FromBytes derive is not supported for unions.",
            ));
        }
    };

    Ok(quote! {
        impl #impl_generics bytecast::FromBytes for #name #ty_generics #where_clause {
            fn from_bytes(buf: &[u8]) -> Result<(Self, usize), bytecast::BytesError> {
                let mut offset = 0usize;
                #body
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

/// Generate a read statement for a single struct field.
fn field_read(field: &syn::Field, var_name: &syn::Ident) -> TokenStream2 {
    let field_type = &field.ty;

    if has_skip_attr(field) {
        return quote! { let #var_name: #field_type = Default::default(); };
    }

    let ser_type = serializable_type(field);
    let read = quote! {
        let (val, consumed) = <#ser_type as bytecast::FromBytes>::from_bytes(&buf[offset..])?;
        offset += consumed;
    };

    if has_boxed_attr(field) {
        quote! { #read let #var_name = Box::new(val); }
    } else {
        quote! { #read let #var_name = val; }
    }
}

fn generate_struct(name: &syn::Ident, fields: &Fields) -> (TokenStream2, TokenStream2) {
    match fields {
        Fields::Named(named) => {
            let reads: Vec<_> = named
                .named
                .iter()
                .map(|f| {
                    let var = f.ident.clone().unwrap();
                    field_read(f, &var)
                })
                .collect();
            let field_names: Vec<_> = named.named.iter().map(|f| &f.ident).collect();
            (
                quote! { #(#reads)* },
                quote! { #name { #(#field_names),* } },
            )
        }
        Fields::Unnamed(unnamed) => {
            let var_names: Vec<_> = (0..unnamed.unnamed.len())
                .map(|i| syn::Ident::new(&format!("field_{i}"), proc_macro2::Span::call_site()))
                .collect();
            let reads: Vec<_> = unnamed
                .unnamed
                .iter()
                .zip(&var_names)
                .map(|(f, var)| field_read(f, var))
                .collect();
            (quote! { #(#reads)* }, quote! { #name(#(#var_names),*) })
        }
        Fields::Unit => (quote! {}, quote! { #name }),
    }
}

fn generate_enum(name: &syn::Ident, data: &syn::DataEnum, disc_type: &syn::Ident) -> TokenStream2 {
    let match_arms: Vec<_> = data
        .variants
        .iter()
        .enumerate()
        .map(|(idx, variant)| {
            let variant_name = &variant.ident;
            let idx_lit = syn::LitInt::new(&idx.to_string(), proc_macro2::Span::call_site());

            match &variant.fields {
                Fields::Unit => quote! {
                    #idx_lit => Ok((#name::#variant_name, offset))
                },
                Fields::Unnamed(fields) => {
                    let names: Vec<_> = (0..fields.unnamed.len())
                        .map(|i| syn::Ident::new(&format!("field_{i}"), proc_macro2::Span::call_site()))
                        .collect();
                    let reads: Vec<_> = fields.unnamed.iter().zip(&names)
                        .map(|(f, var)| {
                            let ty = &f.ty;
                            quote! {
                                let (#var, consumed) = <#ty as bytecast::FromBytes>::from_bytes(&buf[offset..])?;
                                offset += consumed;
                            }
                        })
                        .collect();
                    quote! {
                        #idx_lit => {
                            #(#reads)*
                            Ok((#name::#variant_name(#(#names),*), offset))
                        }
                    }
                }
                Fields::Named(fields) => {
                    let names: Vec<_> = fields.named.iter().map(|f| &f.ident).collect();
                    let reads: Vec<_> = fields.named.iter()
                        .map(|f| {
                            let field_name = &f.ident;
                            let ty = &f.ty;
                            quote! {
                                let (#field_name, consumed) = <#ty as bytecast::FromBytes>::from_bytes(&buf[offset..])?;
                                offset += consumed;
                            }
                        })
                        .collect();
                    quote! {
                        #idx_lit => {
                            #(#reads)*
                            Ok((#name::#variant_name { #(#names),* }, offset))
                        }
                    }
                }
            }
        })
        .collect();

    quote! {
        let (discriminant, consumed) = <#disc_type as bytecast::FromBytes>::from_bytes(&buf[offset..])?;
        offset += consumed;

        match discriminant {
            #(#match_arms,)*
            _ => Err(bytecast::BytesError::InvalidData {
                message: "invalid enum discriminant"
            })
        }
    }
}
