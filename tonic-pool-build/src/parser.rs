use std::{fs, path::PathBuf};

use proc_macro2::Span;
use syn::{Ident, ImplItem, ImplItemFn, Item, ItemImpl, ItemMod, PathSegment, Type};

use crate::{error::{BuilderError, BuilderResult}, model::{GrpcClientFile, GrpcClientImpl, GrpcClientMethod, GrpcClientModule}};

pub(crate) fn parse_grpc_client_file(client_file: &PathBuf) -> BuilderResult<GrpcClientFile> {
    if !client_file.exists() {
        return Err(BuilderError::TODO_TotallyUnexpected);
    }

    let file_module = Ident::new(client_file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or(BuilderError::TODO_TotallyUnexpected)?, Span::call_site());

    let raw_contents = fs::read_to_string(&client_file).map_err(|_| BuilderError::TODO_TotallyUnexpected)?;
    let syn_parsed_contents = syn::parse_file(&raw_contents).map_err(|_| BuilderError::TODO_TotallyUnexpected)?;

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

    Ok(GrpcClientModule {
        name,
        clients,
    })
}

fn find_client_interface_implementations(module: &syn::ItemMod) -> impl Iterator<Item = &syn::ItemImpl> {
    module.content.iter().map(|(_, items)| {
        items.iter().filter_map(|impl_item| match impl_item {
            syn::Item::Impl(impl_item) => Some(impl_item),
            _ => None,
        }).filter(|impl_item| {
            match *impl_item.self_ty {
                syn::Type::Path(ref path) => path.path.segments.last().map_or(false, |segment| {
                    segment.ident.to_string().ends_with("Client")
                }),
                _ => false,
            }
        }).filter(|impl_item| {
            impl_item
                .items
                .iter()
                .any(|item| match item {
                    syn::ImplItem::Fn(impl_item_fn) => {
                        impl_item_fn.sig.ident.to_string() == "new"
                    },
                    _ => false,
                })
        })
    }).flatten()
}

fn parse_grpc_client_implementation(impl_item: &ItemImpl) -> BuilderResult<GrpcClientImpl> {
    let name = match impl_item.self_ty.as_ref() {
        Type::Path(type_path) => {
            match type_path.path.segments.last() {
                Some(PathSegment { ident, .. }) => ident.clone(),
                None => return Err(BuilderError::UnexpectedStructure),
            }
        },
        _ => return Err(BuilderError::UnexpectedStructure),
    };
    let methods = find_grpc_methods(impl_item).map(parse_grpc_method).collect::<BuilderResult<Vec<_>>>()?;

    Ok(GrpcClientImpl {
        name,
        methods,
    })
}

fn find_grpc_methods(impl_item: &ItemImpl) -> impl Iterator<Item = &ImplItemFn> {
    impl_item.items.iter().filter_map(|item| match item {
        ImplItem::Fn(impl_item_fn) => Some(impl_item_fn),
        _ => None,
    }).filter(|method| {
        is_grpc_method(method)
    })
}

// todo-cleanup: Reimplement it more cleanly.
fn is_grpc_method(method: &ImplItemFn) -> bool {
    let mut inputs = method.sig.inputs.iter();
    match (inputs.next(), inputs.next()) {
        (Some(syn::FnArg::Receiver(_)), Some(syn::FnArg::Typed(pat))) => {
            match &*pat.ty {
                syn::Type::ImplTrait(type_impl_trait) => {
                    type_impl_trait.bounds.iter().any(|bound| {
                        match bound {
                            syn::TypeParamBound::Trait(trait_bound) => {
                                let mut segments = trait_bound.path.segments.iter();
                                match (segments.next(), segments.next()) {
                                    (Some(PathSegment { ident: ident1, .. }), Some(PathSegment { ident: ident2, .. })) => {
                                        ident1 == "tonic" && ident2 == "IntoRequest"
                                    },
                                    _ => false,
                                }
                            },
                            _ => false,
                        }
                    })
                },
                _ => false,
            }
        },
        _ => false,
    }
}

fn parse_grpc_method(method: &ImplItemFn) -> BuilderResult<GrpcClientMethod> {
    let inputs = method.sig.inputs.iter();
    let request_type = match inputs.skip(1).next() {
        Some(syn::FnArg::Typed(pat)) => pat, // todo: really want just the internal type of the request itself; not the entire parameter
        _ => return Err(BuilderError::UnexpectedStructure),
    };
    let response_type = match method.sig.output {
        syn::ReturnType::Type(_, ref ty) => ty, // todo: really want just the internal type of the response itself; not the entire return type
        _ => return Err(BuilderError::UnexpectedStructure),
    };

    Ok(GrpcClientMethod {
        name: method.sig.ident.clone(),
        request_type: request_type.clone(),
        response_type: (**response_type).clone(),
    })
}
