//! Rust types to JSON Schema, at compile time.
//!
//! A tool's arguments are a struct the author already wrote; the schema the
//! model reads is derived from it rather than repeated by hand in a string
//! literal that nothing checks. Field doc comments become descriptions, and
//! `Option<T>` is what makes a field optional.

use crate::docs;
use syn::{Fields, GenericArgument, ItemStruct, PathArguments, Type};

/// Build the `parameters` object for a tool from its argument struct.
pub fn object(item: &ItemStruct) -> syn::Result<String> {
    let Fields::Named(fields) = &item.fields else {
        return Err(syn::Error::new_spanned(
            &item.ident,
            "a tool's arguments must be a struct with named fields — the names are what the model fills in",
        ));
    };

    let mut properties = Vec::new();
    let mut required = Vec::new();

    for field in &fields.named {
        let name = field
            .ident
            .as_ref()
            .expect("named fields have idents")
            .to_string();
        let (ty, optional) = unwrap_option(&field.ty);
        let description = docs(&field.attrs);

        let mut property = type_of(ty)?;
        if !description.is_empty() {
            // Exactly one brace: a nested type like `Vec<T>` or a map ends in
            // two, and trimming both puts the description inside the inner
            // object and leaves the JSON one brace short.
            let open = property
                .strip_suffix('}')
                .ok_or_else(|| unsupported(&field.ty))?;
            property = format!(
                r#"{open},"description":"{}"}}"#,
                crate::escape(&description)
            );
        }

        properties.push(format!(r#""{}":{}"#, crate::escape(&name), property));
        if !optional {
            required.push(format!(r#""{}""#, crate::escape(&name)));
        }
    }

    let properties = properties.join(",");
    let required = required.join(",");
    Ok(format!(
        r#"{{"type":"object","properties":{{{properties}}},"required":[{required}]}}"#
    ))
}

/// `Option<T>` means the model may omit the field; everything else is required.
fn unwrap_option(ty: &Type) -> (&Type, bool) {
    if let Type::Path(path) = ty
        && let Some(segment) = path.path.segments.last()
        && segment.ident == "Option"
        && let PathArguments::AngleBracketed(args) = &segment.arguments
        && let Some(GenericArgument::Type(inner)) = args.args.first()
    {
        return (inner, true);
    }
    (ty, false)
}

/// The JSON Schema fragment for one Rust type.
fn type_of(ty: &Type) -> syn::Result<String> {
    if let Type::Reference(reference) = ty {
        return type_of(&reference.elem);
    }

    let Type::Path(path) = ty else {
        return Err(unsupported(ty));
    };
    let Some(segment) = path.path.segments.last() else {
        return Err(unsupported(ty));
    };

    let name = segment.ident.to_string();
    if name == "Vec" {
        let PathArguments::AngleBracketed(args) = &segment.arguments else {
            return Err(unsupported(ty));
        };
        let Some(GenericArgument::Type(inner)) = args.args.first() else {
            return Err(unsupported(ty));
        };
        return Ok(format!(r#"{{"type":"array","items":{}}}"#, type_of(inner)?));
    }

    // A map is an open object: the keys are the caller's, so only the values
    // have a schema. Keys are strings because that is what JSON objects have.
    if name == "BTreeMap" || name == "HashMap" {
        let PathArguments::AngleBracketed(args) = &segment.arguments else {
            return Err(unsupported(ty));
        };
        let mut types = args.args.iter().filter_map(|arg| match arg {
            GenericArgument::Type(ty) => Some(ty),
            _ => None,
        });
        let (Some(key), Some(value)) = (types.next(), types.next()) else {
            return Err(unsupported(ty));
        };
        if type_of(key)? != r#"{"type":"string"}"# {
            return Err(syn::Error::new_spanned(
                key,
                "a map's keys become JSON object keys, so they have to be strings",
            ));
        }
        return Ok(format!(
            r#"{{"type":"object","additionalProperties":{}}}"#,
            type_of(value)?
        ));
    }

    let primitive = match name.as_str() {
        "String" | "str" => "string",
        "bool" => "boolean",
        "f32" | "f64" => "number",
        "u8" | "u16" | "u32" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64" | "isize" => {
            "integer"
        }
        _ => return Err(unsupported(ty)),
    };
    Ok(format!(r#"{{"type":"{primitive}"}}"#))
}

fn unsupported(ty: &Type) -> syn::Error {
    syn::Error::new_spanned(
        ty,
        "a tool argument must be a string, number, boolean, Vec, or Option of those — \
         the model fills these in from a JSON Schema, and there is no schema for this type",
    )
}
