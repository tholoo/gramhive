//! Conservative data derives. Routing and handler arguments are ordinary Rust.
#![forbid(unsafe_code)]
use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::quote;
use syn::{Data, DeriveInput, Fields, LitStr, Type, spanned::Spanned};

fn core() -> Tokens {
    match proc_macro_crate::crate_name("gramhive") {
        Ok(proc_macro_crate::FoundCrate::Itself) => quote!(crate::__private),
        Ok(proc_macro_crate::FoundCrate::Name(name)) => {
            let name = syn::Ident::new(&name, proc_macro2::Span::call_site());
            quote!(::#name::__private)
        }
        Err(_) => quote!(::gramhive_core),
    }
}
fn metadata(input: &DeriveInput, attr: &str, keys: &[&str]) -> syn::Result<Vec<Option<LitStr>>> {
    let mut values = vec![None; keys.len()];
    for attribute in input.attrs.iter().filter(|a| a.path().is_ident(attr)) {
        attribute.parse_nested_meta(|meta| {
            let Some(index) = keys.iter().position(|key| meta.path.is_ident(key)) else {
                return Err(meta.error("unknown metadata key"));
            };
            if values[index].is_some() {
                return Err(meta.error("duplicate metadata"));
            }
            values[index] = Some(meta.value()?.parse::<LitStr>()?);
            Ok(())
        })?;
    }
    Ok(values)
}
fn no_generics(input: &DeriveInput) -> syn::Result<()> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "data derives do not support generic declarations",
        ));
    }
    Ok(())
}
fn named_or_unit(fields: &Fields) -> syn::Result<Vec<&syn::Field>> {
    match fields {
        Fields::Named(f) => Ok(f.named.iter().collect()),
        Fields::Unit => Ok(vec![]),
        _ => Err(syn::Error::new_spanned(
            fields,
            "use a unit or named-field declaration",
        )),
    }
}
fn plain_path(ty: &Type) -> bool {
    matches!(ty, Type::Path(p) if p.qself.is_none() && p.path.segments.iter().all(|s| matches!(s.arguments, syn::PathArguments::None)))
}
fn string(ty: &Type) -> bool {
    matches!(ty, Type::Path(p) if plain_path(ty) && p.path.segments.last().is_some_and(|s| s.ident == "String"))
}
fn optional_string(ty: &Type) -> bool {
    let Type::Path(p) = ty else {
        return false;
    };
    let Some(s) = p.path.segments.last() else {
        return false;
    };
    let syn::PathArguments::AngleBracketed(args) = &s.arguments else {
        return false;
    };
    s.ident == "Option"
        && args.args.len() == 1
        && matches!(args.args.first(), Some(syn::GenericArgument::Type(t)) if string(t))
}

#[proc_macro_derive(Command, attributes(command, rest))]
pub fn command(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    derive_command(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
fn derive_command(input: &DeriveInput) -> syn::Result<Tokens> {
    no_generics(input)?;
    let core = core();
    let name = &input.ident;
    let meta = metadata(input, "command", &["name", "description"])?;
    let command = meta[0]
        .as_ref()
        .ok_or_else(|| syn::Error::new(input.span(), "missing #[command(name = \"...\")]"))?;
    let value = command.value();
    if value.is_empty()
        || value.len() > 32
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(syn::Error::new_spanned(
            command,
            "command name must contain 1..=32 lowercase ASCII letters, digits, or underscores",
        ));
    }
    let description = match &meta[1] {
        Some(v) => quote!(Some(#v)),
        None => quote!(None),
    };
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(input.span(), "Command requires a struct"));
    };
    let fields = named_or_unit(&data.fields)?;
    let rests: Vec<_> = fields
        .iter()
        .enumerate()
        .filter(|(_, f)| f.attrs.iter().any(|a| a.path().is_ident("rest")))
        .collect();
    if rests.len() > 1 {
        return Err(syn::Error::new_spanned(
            rests[1].1,
            "at most one #[rest] field is allowed",
        ));
    }
    if let Some((index, field)) = rests.first()
        && index + 1 != fields.len()
    {
        return Err(syn::Error::new_spanned(
            field,
            "#[rest] must be the final field",
        ));
    }
    let mut parsing = Vec::new();
    let mut names = Vec::new();
    for (index, field) in fields.into_iter().enumerate() {
        let ident = field.ident.as_ref().unwrap();
        let local = quote::format_ident!("__gramhive_field_{index}");
        names.push(quote!(#ident: #local));
        let ty = &field.ty;
        let attrs: Vec<_> = field
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("rest"))
            .collect();
        if attrs.len() > 1 {
            return Err(syn::Error::new_spanned(
                field,
                "duplicate #[rest] attribute",
            ));
        }
        let expression = if let Some(attr) = attrs.first() {
            if !matches!(attr.meta, syn::Meta::Path(_)) {
                return Err(syn::Error::new_spanned(
                    attr,
                    "use #[rest] without arguments",
                ));
            }
            if string(ty) {
                quote!(args.rest().ok_or_else(|| #core::CommandError(concat!("missing argument `", stringify!(#ident), "`").into()))?)
            } else if optional_string(ty) {
                quote!(args.rest())
            } else {
                return Err(syn::Error::new_spanned(
                    ty,
                    "#[rest] supports only String or Option<String>",
                ));
            }
        } else {
            if !plain_path(ty) {
                return Err(syn::Error::new_spanned(
                    ty,
                    "positional fields require a concrete FromStr type; optional input uses #[rest] Option<String>",
                ));
            }
            quote!(args.take::<#ty>(stringify!(#ident))?)
        };
        parsing.push(quote!(let #local = #expression;));
    }
    let construct = if matches!(data.fields, Fields::Unit) {
        quote!(Self)
    } else {
        quote!(Self { #(#names),* })
    };
    Ok(quote! {
        impl #core::CommandSpec for #name {
            const NAME: &'static str = #command;
            const DESCRIPTION: ::core::option::Option<&'static str> = #description;
            fn parse(input: &str) -> ::core::result::Result<Self, #core::CommandError> {
                let mut args = #core::Arguments::new(input);
                #(#parsing)*
                args.finish()?;
                Ok(#construct)
            }
        }
    })
}

#[proc_macro_derive(CallbackData, attributes(callback))]
pub fn callback(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    derive_callback(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
fn derive_callback(input: &DeriveInput) -> syn::Result<Tokens> {
    no_generics(input)?;
    let core = core();
    let name = &input.ident;
    let meta = metadata(input, "callback", &["prefix"])?;
    let prefix = meta[0]
        .as_ref()
        .ok_or_else(|| syn::Error::new(input.span(), "missing #[callback(prefix = \"...\")]"))?;
    if prefix.value().is_empty() || prefix.value().len() > 32 {
        return Err(syn::Error::new_spanned(
            prefix,
            "callback prefix must contain 1..=32 UTF-8 bytes",
        ));
    }
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new(
            input.span(),
            "CallbackData requires an enum",
        ));
    };
    if data.variants.is_empty() {
        return Err(syn::Error::new(
            input.span(),
            "CallbackData requires at least one variant",
        ));
    }
    let mut encode = Vec::new();
    let mut decode = Vec::new();
    for variant in &data.variants {
        let ident = &variant.ident;
        if variant.discriminant.is_some() {
            return Err(syn::Error::new_spanned(
                variant,
                "callback variants cannot have explicit discriminants",
            ));
        }
        let label = ident.to_string();
        if prefix.value().len() + label.len() + 3 > 64 {
            return Err(syn::Error::new_spanned(
                ident,
                "callback header exceeds Telegram's 64-byte limit",
            ));
        }
        let fields = named_or_unit(&variant.fields)?;
        let mut names = Vec::new();
        let mut reads = Vec::new();
        let mut locals = Vec::new();
        for (index, field) in fields.into_iter().enumerate() {
            let field_name = field.ident.as_ref().unwrap();
            let ty = &field.ty;
            if !plain_path(ty) {
                return Err(syn::Error::new_spanned(
                    ty,
                    "callback fields require concrete Display + FromStr types",
                ));
            }
            let local = quote::format_ident!("__gramhive_field_{index}");
            names.push(quote!(#field_name: #local));
            locals.push(local.clone());
            reads.push(quote!(let #local = #core::codec::take(&mut input)?.parse::<#ty>().map_err(|_| #core::CallbackDataError::Malformed(concat!("invalid ", stringify!(#field_name))))?;));
        }
        let construct = if matches!(variant.fields, Fields::Unit) {
            quote!(Self::#ident)
        } else {
            quote!(Self::#ident { #(#names),* })
        };
        encode.push(quote! { #construct => {
            let mut out = #core::codec::header(#prefix, #label)?;
            #(#core::codec::push(&mut out, &::std::string::ToString::to_string(#locals))?;)*
            Ok(out)
        }});
        decode.push(quote! { #label => { #(#reads)* #construct } });
    }
    Ok(quote! {
        impl #core::CallbackData for #name {
            const PREFIX: &'static str = #prefix;
            fn encode(&self) -> ::core::result::Result<::std::vec::Vec<u8>, #core::CallbackDataError> { match self { #(#encode),* } }
            fn decode(data: &[u8]) -> ::core::result::Result<Self, #core::CallbackDataError> {
                let (variant, mut input) = #core::codec::open(data, #prefix)?;
                let result = match variant { #(#decode),*, _ => return Err(#core::CallbackDataError::Malformed("unknown variant")) };
                if !input.is_empty() { return Err(#core::CallbackDataError::Malformed("trailing bytes")); }
                Ok(result)
            }
        }
    })
}
