//! FromBytes derive macro implementation.

use super::{
    disc_capacity, field_type_bounds, has_boxed_attr, has_skip_attr, parse_crate_path,
    reject_enum_field_attrs, repr_int_type, resolve_discriminants, serializable_type,
    validate_struct_field_attrs,
};
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields};

pub fn derive_impl(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let krate = parse_crate_path(&input.attrs)?;
    let name = &input.ident;
    let mut generics = input.generics.clone();
    let extra_bounds = field_type_bounds(input, syn::parse_quote!(#krate::FromBytes))?;
    if !extra_bounds.is_empty() {
        generics.make_where_clause().predicates.extend(extra_bounds);
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let body = match &input.data {
        Data::Struct(data) => {
            validate_struct_field_attrs(&data.fields)?;
            let (reads, constructor) = generate_struct(&krate, name, &data.fields)?;
            quote! {
                #reads
                Ok((#constructor, offset))
            }
        }
        Data::Enum(data) => {
            let disc_ident = validate_enum(input, data)?;
            let disc_values = resolve_discriminants(data)?;
            generate_enum(&krate, name, data, &disc_ident, &disc_values)
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "FromBytes derive is not supported for unions.",
            ));
        }
    };

    Ok(quote! {
        impl #impl_generics #krate::FromBytes for #name #ty_generics #where_clause {
            fn from_bytes(buf: &[u8]) -> ::core::result::Result<(Self, usize), #krate::BytesError> {
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
    reject_enum_field_attrs(data)?;
    Ok(disc_ident)
}

/// Generate a read statement for a single struct field.
fn field_read(
    krate: &syn::Path,
    field: &syn::Field,
    var_name: &syn::Ident,
) -> syn::Result<TokenStream2> {
    let field_type = &field.ty;

    if has_skip_attr(field) {
        return Ok(quote! { let #var_name: #field_type = Default::default(); });
    }

    let ser_type = serializable_type(field)?;
    let read = quote! {
        let (val, consumed) = <#ser_type as #krate::FromBytes>::from_bytes(&buf[offset..])?;
        offset += consumed;
    };

    if has_boxed_attr(field) {
        Ok(quote! { #read let #var_name = Box::new(val); })
    } else {
        Ok(quote! { #read let #var_name = val; })
    }
}

fn generate_struct(
    krate: &syn::Path,
    name: &syn::Ident,
    fields: &Fields,
) -> syn::Result<(TokenStream2, TokenStream2)> {
    match fields {
        Fields::Named(named) => {
            let reads: Vec<_> = named
                .named
                .iter()
                .map(|f| {
                    let var = f.ident.clone().unwrap();
                    field_read(krate, f, &var)
                })
                .collect::<syn::Result<_>>()?;
            let field_names: Vec<_> = named.named.iter().map(|f| &f.ident).collect();
            Ok((
                quote! { #(#reads)* },
                quote! { #name { #(#field_names),* } },
            ))
        }
        Fields::Unnamed(unnamed) => {
            let var_names: Vec<_> = (0..unnamed.unnamed.len())
                .map(|i| syn::Ident::new(&format!("field_{i}"), proc_macro2::Span::call_site()))
                .collect();
            let reads: Vec<_> = unnamed
                .unnamed
                .iter()
                .zip(&var_names)
                .map(|(f, var)| field_read(krate, f, var))
                .collect::<syn::Result<_>>()?;
            Ok((quote! { #(#reads)* }, quote! { #name(#(#var_names),*) }))
        }
        Fields::Unit => Ok((quote! {}, quote! { #name })),
    }
}

fn generate_enum(
    krate: &syn::Path,
    name: &syn::Ident,
    data: &syn::DataEnum,
    disc_type: &syn::Ident,
    disc_values: &[i128],
) -> TokenStream2 {
    let match_arms: Vec<_> = data
        .variants
        .iter()
        .zip(disc_values)
        .map(|(variant, &disc_val)| {
            let variant_name = &variant.ident;
            let disc_lit = syn::LitInt::new(&disc_val.to_string(), proc_macro2::Span::call_site());

            match &variant.fields {
                Fields::Unit => quote! {
                    #disc_lit => Ok((#name::#variant_name, offset))
                },
                Fields::Unnamed(fields) => {
                    let names: Vec<_> = (0..fields.unnamed.len())
                        .map(|i| syn::Ident::new(&format!("field_{i}"), proc_macro2::Span::call_site()))
                        .collect();
                    let reads: Vec<_> = fields.unnamed.iter().zip(&names)
                        .map(|(f, var)| {
                            let ty = &f.ty;
                            quote! {
                                let (#var, consumed) = <#ty as #krate::FromBytes>::from_bytes(&buf[offset..])?;
                                offset += consumed;
                            }
                        })
                        .collect();
                    quote! {
                        #disc_lit => {
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
                                let (#field_name, consumed) = <#ty as #krate::FromBytes>::from_bytes(&buf[offset..])?;
                                offset += consumed;
                            }
                        })
                        .collect();
                    quote! {
                        #disc_lit => {
                            #(#reads)*
                            Ok((#name::#variant_name { #(#names),* }, offset))
                        }
                    }
                }
            }
        })
        .collect();

    quote! {
        let (discriminant, consumed) = <#disc_type as #krate::FromBytes>::from_bytes(&buf[offset..])?;
        offset += consumed;

        match discriminant {
            #(#match_arms,)*
            _ => Err(#krate::BytesError::InvalidData {
                message: "invalid enum discriminant"
            })
        }
    }
}
