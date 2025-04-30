use std::{fs, io, path::PathBuf};

use proc_macro2::Span;
use syn::{
    FnArg, Ident, ImplItem, ImplItemFn, Item, ItemImpl, ItemMod, PatType, PathSegment, ReturnType,
    Type, TypeImplTrait,
};
use tracing::warn;

use crate::{
    error::{BuilderError, BuilderResult},
    model::{GrpcClientFile, GrpcClientImpl, GrpcClientMethod, GrpcClientModule},
};

pub(crate) fn parse_grpc_client_file(client_file: &PathBuf) -> BuilderResult<GrpcClientFile> {
    if !(client_file.exists() && client_file.is_file()) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("File not found: {client_file:?}",),
        )
        .into());
    }

    let file_module = Ident::new(
        client_file
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("input must be a file"),
        Span::call_site(),
    );

    let raw_contents = fs::read_to_string(client_file)?;
    let syn_parsed_contents = syn::parse_file(&raw_contents)?;

    let client_modules = find_client_modules(&syn_parsed_contents)
        .map(parse_grpc_client_module)
        .collect::<BuilderResult<Vec<_>>>()?;

    Ok(GrpcClientFile {
        file_module,
        client_modules,
    })
}

fn find_client_modules(file: &syn::File) -> impl Iterator<Item = &ItemMod> {
    file.items
        .iter()
        .filter_map(|item| match item {
            Item::Mod(module) => Some(module),
            _ => None,
        })
        .filter(|module| module.ident.to_string().ends_with("_client"))
}

fn parse_grpc_client_module(client_module: &ItemMod) -> BuilderResult<GrpcClientModule> {
    let name = client_module.ident.clone();
    let clients = find_client_interface_implementations(client_module)
        .map(parse_grpc_client_implementation)
        .collect::<BuilderResult<Vec<_>>>()?;

    Ok(GrpcClientModule { name, clients })
}

fn find_client_interface_implementations(
    module: &syn::ItemMod,
) -> impl Iterator<Item = &syn::ItemImpl> {
    module.content.iter().flat_map(|(_, items)| {
        items
            .iter()
            .filter_map(|impl_item| match impl_item {
                syn::Item::Impl(impl_item) => Some(impl_item),
                _ => None,
            })
            .filter(|impl_item| match *impl_item.self_ty {
                syn::Type::Path(ref path) => path
                    .path
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident.to_string().ends_with("Client")),
                _ => false,
            })
            .filter(|impl_item| {
                impl_item.items.iter().any(|item| match item {
                    syn::ImplItem::Fn(impl_item_fn) => impl_item_fn.sig.ident == "new",
                    _ => false,
                })
            })
    })
}

fn parse_grpc_client_implementation(impl_item: &ItemImpl) -> BuilderResult<GrpcClientImpl> {
    let name = match impl_item.self_ty.as_ref() {
        Type::Path(type_path) => match type_path.path.segments.last() {
            Some(PathSegment { ident, .. }) => ident.clone(),
            None => return Err(BuilderError::UnexpectedStructure),
        },
        _ => return Err(BuilderError::UnexpectedStructure),
    };
    let methods = find_grpc_methods(impl_item)
        .map(parse_grpc_method)
        .collect::<BuilderResult<Vec<_>>>()?;

    Ok(GrpcClientImpl { name, methods })
}

fn find_grpc_methods(impl_item: &ItemImpl) -> impl Iterator<Item = &ImplItemFn> {
    impl_item
        .items
        .iter()
        .filter_map(|item| match item {
            ImplItem::Fn(impl_item_fn) => Some(impl_item_fn),
            _ => None,
        })
        .filter(|method| is_grpc_method(method))
}

fn is_grpc_method(method: &ImplItemFn) -> bool {
    let mut inputs = method.sig.inputs.iter();
    let first_arg_type = match (inputs.next(), inputs.next()) {
        (Some(syn::FnArg::Receiver(_)), Some(syn::FnArg::Typed(pat))) => &*pat.ty,
        _ => return false,
    };

    let trait_bound = match first_arg_type {
        syn::Type::ImplTrait(type_impl_trait) => {
            match type_impl_trait.bounds.iter().find_map(|bound| match bound {
                syn::TypeParamBound::Trait(trait_bound) => Some(trait_bound),
                _ => None,
            }) {
                Some(trait_bound) => trait_bound,
                None => return false,
            }
        }
        _ => return false,
    };

    let mut segments = trait_bound.path.segments.iter();
    let tonic_into_type = match (segments.next(), segments.next()) {
        (Some(PathSegment { ident: ident1, .. }), Some(PathSegment { ident: ident2, .. }))
            if ident1 == "tonic" =>
        {
            ident2
        }
        _ => return false,
    };

    if tonic_into_type == "IntoRequest" {
        true
    } else if tonic_into_type == "IntoStreamingRequest" {
        // Note: I would like to support this, but don't have enough experience
        // with streaming to even begin to imagine how to support retrying.
        // Maybe I could implement only non-retrying version for now?
        warn!("Streaming request is not supported yet");
        false
    } else {
        false
    }
}

fn parse_grpc_method(method: &ImplItemFn) -> BuilderResult<GrpcClientMethod> {
    let mut inputs = method.sig.inputs.iter();

    // todo: really want just the internal type of the request itself; not the entire parameter
    let Some(FnArg::Typed(PatType {
        ty: request_arg_type,
        ..
    })) = inputs.nth(1)
    else {
        return Err(BuilderError::UnexpectedStructure);
    };
    let request_type = get_into_request_type(request_arg_type)?.to_owned();

    let response_success_type = get_result_success_type(&method.sig.output)?;
    let response_type = get_tonic_response_type(response_success_type)?.to_owned();

    Ok(GrpcClientMethod {
        name: method.sig.ident.clone(),
        request_type,
        response_type,
    })
}

fn get_result_success_type(return_type: &ReturnType) -> BuilderResult<&Type> {
    match return_type {
        ReturnType::Type(_, ty) => {
            if let Type::Path(ref type_path) = **ty {
                if let Some(segment) = type_path.path.segments.last() {
                    if segment.ident == "Result" {
                        if let syn::PathArguments::AngleBracketed(ref args) = segment.arguments {
                            if let Some(syn::GenericArgument::Type(ty)) = args.args.first() {
                                return Ok(ty);
                            }
                        }
                    }
                }
            }
        }
        ReturnType::Default => {}
    }

    Err(BuilderError::UnexpectedStructure)
}

fn get_tonic_response_type(response_type: &Type) -> BuilderResult<&Type> {
    if let syn::Type::Path(type_path) = response_type {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Response" {
                if let syn::PathArguments::AngleBracketed(ref args) = segment.arguments {
                    if let Some(syn::GenericArgument::Type(ty)) = args.args.first() {
                        return Ok(ty);
                    }
                }
            }
        }
    }

    Err(BuilderError::UnexpectedStructure)
}

fn get_into_request_type(request_type: &Type) -> BuilderResult<&Type> {
    match request_type {
        syn::Type::Path(type_path) => {
            if let Some(segment) = type_path.path.segments.last() {
                if segment.ident == "IntoRequest" {
                    if let syn::PathArguments::AngleBracketed(ref args) = segment.arguments {
                        if let Some(syn::GenericArgument::Type(ty)) = args.args.first() {
                            return Ok(ty);
                        }
                    }
                }
            }
        }
        syn::Type::ImplTrait(TypeImplTrait {
            bounds: type_impl_trait,
            ..
        }) => {
            if let Some(syn::TypeParamBound::Trait(trait_bound)) = type_impl_trait.first() {
                if let Some(segment) = trait_bound.path.segments.last() {
                    if segment.ident == "IntoRequest" {
                        if let syn::PathArguments::AngleBracketed(ref args) = segment.arguments {
                            if let Some(syn::GenericArgument::Type(ty)) = args.args.first() {
                                return Ok(ty);
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }

    Err(BuilderError::UnexpectedStructure)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use syn::File;

    use super::*;

    #[test]
    fn ok() {
        let syntax: File = syn::parse_str(include_str!("test_cases/parser_success.rs")).unwrap();
        let actual = find_client_modules(&syntax)
            .map(parse_grpc_client_module)
            .collect::<BuilderResult<Vec<_>>>()
            .unwrap();
        let expected = vec![
            GrpcClientModule {
                name: Ident::new("health_client", Span::call_site()),
                clients: vec![GrpcClientImpl {
                    name: Ident::new("HealthClient", Span::call_site()),
                    methods: vec![GrpcClientMethod {
                        name: Ident::new("is_alive", Span::call_site()),
                        request_type: syn::parse_str("()").unwrap(),
                        response_type: syn::parse_str("super::IsAliveResponse").unwrap(),
                    }],
                }],
            },
            GrpcClientModule {
                name: Ident::new("echo_client", Span::call_site()),
                clients: vec![GrpcClientImpl {
                    name: Ident::new("EchoClient", Span::call_site()),
                    methods: vec![GrpcClientMethod {
                        name: Ident::new("echo_message", Span::call_site()),
                        request_type: syn::parse_str("super::EchoRequest").unwrap(),
                        response_type: syn::parse_str("super::EchoResponse").unwrap(),
                    }],
                }],
            },
        ];
        assert_eq!(actual, expected);
    }
}
