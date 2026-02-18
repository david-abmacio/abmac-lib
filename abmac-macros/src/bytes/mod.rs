//! Derive macros for bytecast.

mod from_bytes;
mod to_bytes;

pub use from_bytes::derive_impl as derive_from_bytes_impl;
pub use to_bytes::derive_impl as derive_to_bytes_impl;

/// Extract the discriminant type from `#[repr(uN)]` on an enum.
/// Returns `None` if no repr or a non-integer repr is used (defaults to u8).
pub fn repr_int_type(attrs: &[syn::Attribute]) -> Option<syn::Ident> {
    for attr in attrs {
        if !attr.path().is_ident("repr") {
            continue;
        }
        let mut found = None;
        let _ = attr.parse_nested_meta(|meta| {
            let ident = meta.path.get_ident().map(|id| id.to_string());
            if let Some("u8" | "u16" | "u32" | "u64" | "i8" | "i16" | "i32" | "i64") =
                ident.as_deref()
            {
                found = Some(meta.path.get_ident().unwrap().clone());
            }
            Ok(())
        });
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Return the max number of variants a discriminant type can hold.
pub fn disc_capacity(disc_type: &str) -> usize {
    match disc_type {
        "u8" | "i8" => 256,
        "u16" | "i16" => 65536,
        _ => usize::MAX,
    }
}

/// Resolve discriminant values for all variants.
///
/// Supports integer literals (`= 10`) and auto-increment from the previous
/// value. Returns one `i128` per variant. Rejects non-literal discriminants
/// with a compile error.
pub fn resolve_discriminants(data: &syn::DataEnum) -> syn::Result<Vec<i128>> {
    let mut next: i128 = 0;
    let mut values = Vec::with_capacity(data.variants.len());
    for variant in &data.variants {
        if let Some((_, expr)) = &variant.discriminant {
            next = parse_int_expr(expr)?;
        }
        values.push(next);
        next = next.wrapping_add(1);
    }
    Ok(values)
}

/// Parse an integer literal from a discriminant expression.
/// Supports both positive (`10`) and negative (`-10`) literals.
fn parse_int_expr(expr: &syn::Expr) -> syn::Result<i128> {
    match expr {
        syn::Expr::Lit(lit) => parse_int_lit(&lit.lit),
        syn::Expr::Unary(syn::ExprUnary {
            op: syn::UnOp::Neg(_),
            expr,
            ..
        }) => {
            if let syn::Expr::Lit(lit) = expr.as_ref() {
                Ok(-parse_int_lit(&lit.lit)?)
            } else {
                Err(syn::Error::new_spanned(
                    expr,
                    "bytecast: enum discriminants must be integer literals",
                ))
            }
        }
        _ => Err(syn::Error::new_spanned(
            expr,
            "bytecast: enum discriminants must be integer literals",
        )),
    }
}

fn parse_int_lit(lit: &syn::Lit) -> syn::Result<i128> {
    match lit {
        syn::Lit::Int(int) => int
            .base10_parse::<i128>()
            .map_err(|e| syn::Error::new(int.span(), e)),
        _ => Err(syn::Error::new_spanned(
            lit,
            "bytecast: enum discriminants must be integer literals",
        )),
    }
}

/// Parsed `#[bytecast(...)]` attributes on a single field.
struct FieldAttrs {
    skip: bool,
    boxed: bool,
}

/// Parse all `#[bytecast(...)]` attributes on a field, rejecting unknown names.
fn parse_field_attrs(field: &syn::Field) -> syn::Result<FieldAttrs> {
    let mut attrs = FieldAttrs {
        skip: false,
        boxed: false,
    };
    for attr in &field.attrs {
        if !attr.path().is_ident("bytecast") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                attrs.skip = true;
            } else if meta.path.is_ident("boxed") {
                attrs.boxed = true;
            } else {
                return Err(meta.error("unknown bytecast attribute; expected `skip` or `boxed`"));
            }
            Ok(())
        })?;
    }
    Ok(attrs)
}

pub fn has_skip_attr(field: &syn::Field) -> bool {
    parse_field_attrs(field).is_ok_and(|a| a.skip) || is_phantom_data(&field.ty)
}

pub fn has_boxed_attr(field: &syn::Field) -> bool {
    parse_field_attrs(field).is_ok_and(|a| a.boxed)
}

/// Check if a type is `PhantomData` (with any generic args).
fn is_phantom_data(ty: &syn::Type) -> bool {
    let syn::Type::Path(type_path) = ty else {
        return false;
    };
    type_path
        .path
        .segments
        .last()
        .is_some_and(|seg| seg.ident == "PhantomData")
}

/// Validate all `#[bytecast(...)]` attributes on struct fields.
/// Rejects unknown attribute names at compile time.
pub fn validate_struct_field_attrs(fields: &syn::Fields) -> syn::Result<()> {
    for field in fields {
        parse_field_attrs(field)?;
    }
    Ok(())
}

/// Validate and reject `#[bytecast(...)]` on enum variant fields.
/// Enum fields don't support `skip` or `boxed`.
pub fn reject_enum_field_attrs(data: &syn::DataEnum) -> syn::Result<()> {
    for variant in &data.variants {
        for field in variant.fields.iter() {
            let attrs = parse_field_attrs(field)?;
            if attrs.skip {
                return Err(syn::Error::new_spanned(
                    field,
                    "#[bytecast(skip)] is not supported on enum variant fields",
                ));
            }
            if attrs.boxed {
                return Err(syn::Error::new_spanned(
                    field,
                    "#[bytecast(boxed)] is not supported on enum variant fields",
                ));
            }
        }
    }
    Ok(())
}

/// Extract the inner type `T` from `Box<T>`.
pub fn extract_box_inner(ty: &syn::Type) -> Option<&syn::Type> {
    let syn::Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Box" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    match args.args.first() {
        Some(syn::GenericArgument::Type(inner)) => Some(inner),
        _ => None,
    }
}

/// Collect where-clause predicates for non-skipped field types.
///
/// For each non-skipped field, produces a `FieldType: Bound` predicate using the
/// serializable type (inner type for `#[bytecast(boxed)]` fields). This is the
/// serde-style approach: bounds are placed on field types, not type parameters,
/// so generic parameters that only appear in skipped fields (e.g. `PhantomData<T>`)
/// don't require the bound.
pub fn field_type_bounds(
    input: &syn::DeriveInput,
    bound: syn::TypeParamBound,
) -> syn::Result<Vec<syn::WherePredicate>> {
    // Only add bounds if there are type parameters.
    if input.generics.type_params().next().is_none() {
        return Ok(Vec::new());
    }

    let fields: Vec<&syn::Field> = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Named(f) => f.named.iter().collect(),
            syn::Fields::Unnamed(f) => f.unnamed.iter().collect(),
            syn::Fields::Unit => vec![],
        },
        syn::Data::Enum(data) => data.variants.iter().flat_map(|v| v.fields.iter()).collect(),
        syn::Data::Union(_) => vec![],
    };

    let mut predicates = Vec::new();
    for field in fields {
        if has_skip_attr(field) {
            continue;
        }
        let ty = serializable_type(field)?;
        predicates.push(syn::parse_quote!(#ty: #bound));
    }
    Ok(predicates)
}

/// Resolve the serializable type for a field, accounting for `#[bytecast(boxed)]`.
pub fn serializable_type(field: &syn::Field) -> syn::Result<&syn::Type> {
    if has_boxed_attr(field) {
        extract_box_inner(&field.ty).ok_or_else(|| {
            syn::Error::new_spanned(
                &field.ty,
                "#[bytecast(boxed)] requires field type to be Box<T>",
            )
        })
    } else {
        Ok(&field.ty)
    }
}
