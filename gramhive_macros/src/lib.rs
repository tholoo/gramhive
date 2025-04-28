use deluxe::ExtractAttributes;
use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, parse_quote, Expr, FnArg, GenericArgument, ItemFn, LitInt, Pat, PatIdent,
    PathArguments, Type, TypePath,
};

#[derive(ExtractAttributes)]
#[deluxe(attributes(argument))]
struct Argument {
    extractor: Expr,
}

#[derive(ExtractAttributes)]
#[deluxe(attributes(arg))]
struct Arg(LitInt);

fn is_option(ty: &Type) -> bool {
    if let Type::Path(tp) = ty {
        let segs = &tp.path.segments;
        if segs.len() == 1 && segs[0].ident == "Option" {
            return matches!(segs[0].arguments, PathArguments::AngleBracketed(_));
        }
    }
    false
}

fn get_inner_type(ty: &Type) -> &Type {
    if let Type::Path(TypePath { path, .. }) = ty {
        if let Some(seg) = path.segments.last() {
            if seg.ident == "Option" {
                if let PathArguments::AngleBracketed(args) = &seg.arguments {
                    if let Some(GenericArgument::Type(inner)) = args.args.first() {
                        return inner;
                    }
                }
            }
        }
    }
    ty
}

#[proc_macro_attribute]
pub fn command(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut func = parse_macro_input!(item as ItemFn);
    let sig = &mut func.sig;
    let mut extract_stmts = Vec::new();
    let mut new_inputs = syn::punctuated::Punctuated::new();

    for arg in sig.inputs.iter_mut() {
        match arg {
            FnArg::Typed(pat_type) => {
                let extractor =
                    if let Ok(Argument { extractor }) = deluxe::extract_attributes(pat_type) {
                        Some(quote! {#extractor})
                    } else if let Ok(Arg(idx)) = deluxe::extract_attributes(pat_type) {
                        Some(quote! {
                            gramhive::extractors::ArgumentExtractor::new(#idx)
                        })
                    } else if pat_type.attrs.iter().any(|a| a.path().is_ident("reply")) {
                        Some(quote! {
                            gramhive::extractors::ReplyExtractor{}
                        })
                    } else if pat_type.attrs.iter().any(|a| a.path().is_ident("input")) {
                        Some(quote! {
                            gramhive::extractors::InputExtractor{}
                        })
                    } else {
                        None
                    };

                if let Some(extractor) = extractor {
                    if let Pat::Ident(PatIdent { ident, .. }) = &*pat_type.pat {
                        let ty = &*pat_type.ty;
                        let inner_type = get_inner_type(ty);
                        let ident_str = ident.to_string();

                        let ts = if is_option(ty) {
                            quote! {
                                let #ident: Option<#inner_type> = match gramhive::extractors::Extractor::extract(
                                    &#extractor,
                                    std::sync::Arc::clone(&client),
                                    &message,
                                    &command_input
                                ).await {
                                    Ok(val) => Some(val),
                                    Err(gramhive::errors::ExtractionError::Missing) => {
                                        None
                                    }
                                    Err(err) => { return Err(err.with_context(#ident_str, command_input.clone()).into()); },
                                };
                            }
                        } else {
                            quote! {
                                let #ident: #inner_type = match gramhive::extractors::Extractor::extract(
                                    &#extractor,
                                    std::sync::Arc::clone(&client),
                                    &message,
                                    &command_input
                                ).await {
                                    Ok(val) => val,
                                    Err(err) => { return Err(err.with_context(#ident_str, command_input.clone()).into()); },
                                };
                            }
                        };

                        extract_stmts.push(ts);
                        continue;
                    }
                }

                new_inputs.push(arg.clone());
            }

            other => new_inputs.push(other.clone()),
        }
    }

    let mut has_message = false;
    let mut has_cmd_input = false;
    let mut has_client = false;
    for input in new_inputs.iter() {
        if let FnArg::Typed(pat_type) = input {
            if let Pat::Ident(p) = &*pat_type.pat {
                match p.ident.to_string().as_str() {
                    "message" => has_message = true,
                    "command_input" => has_cmd_input = true,
                    "client" => has_client = true,
                    _ => {}
                }
            }
        }
    }

    if !has_message {
        new_inputs.insert(0, parse_quote! { message: grammers_client::types::Message });
    }
    if !has_cmd_input {
        new_inputs.insert(
            0,
            parse_quote! { command_input: gramhive::commands::CommandInput },
        );
    }
    if !has_client {
        new_inputs.insert(
            0,
            parse_quote! { client: std::sync::Arc<grammers_client::Client> },
        );
    }

    sig.inputs = new_inputs;

    let old_stmts = std::mem::take(&mut func.block.stmts);
    let mut new_stmts = Vec::new();
    for stmt in extract_stmts {
        new_stmts.push(syn::parse2(stmt).expect("quoted code must parse"));
    }
    new_stmts.extend(old_stmts);
    func.block.stmts = new_stmts;

    TokenStream::from(quote! { #func })
}
