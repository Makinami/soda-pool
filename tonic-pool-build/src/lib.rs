use std::{
    fs, path::{Path, PathBuf}
};

use proc_macro2::{Span, TokenStream};
use syn::{parse_file, Ident, Item, ItemMod, PathSegment, Receiver, };
use tracing::{debug, info};
use quote::quote;

pub fn configure() -> TonicPoolBuilder {
    TonicPoolBuilder { dir: None }
}

pub struct TonicPoolBuilder {
    dir: Option<PathBuf>,
}

type TonicPoolBuilderError = ();

impl TonicPoolBuilder {
    pub fn dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.dir = Some(dir.as_ref().to_path_buf());
        self
    }

    pub fn wrap_services(&self, services: &[impl AsRef<str>]) -> Result<(), TonicPoolBuilderError> {
        let dir = self.dir.as_ref().ok_or(())?;

        services.into_iter().try_for_each(|service| {
            let service_filename = if service.as_ref().ends_with(".rs") {
                service.as_ref().to_string()
            } else {
                format!("{}.rs", service.as_ref())
            };

            let service_file = dir.join(&service_filename);
            wrap_service(service_file)
        })
    }
}

fn wrap_service(service_file: PathBuf) -> Result<(), TonicPoolBuilderError> {
    if !service_file.exists() {
        return Err(());
    }

    let proto_module = Ident::new(service_file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or(())?, Span::call_site());

    let contents = fs::read_to_string(&service_file).map_err(|_| ())?;

    let file = parse_file(&contents).map_err(|_| ())?;

    let modules = find_client_modules(&file)
        .map(|client_module| {
            let module_ident = &client_module.ident;
            debug!("Found client module: {}", module_ident);
            let pooled_clients = find_client_interface_implementations(client_module)
                .map(|impl_item| {
                    let struct_name = match *impl_item.self_ty {
                        syn::Type::Path(ref path) => path.path.segments.last().unwrap().ident.to_string(),
                        _ => unreachable!(),
                    };
                    debug!("Found client struct implementation for: {struct_name}");
                    wrap_client(&impl_item)
                });
            quote! {
                pub mod #module_ident {
                    use super::super::#proto_module::#module_ident::*;
                    #(
                        #pooled_clients
                    )*
                }
            }
        });

    let gen_module_name = service_file.file_stem().unwrap().to_str().unwrap();
    let gen_module_ident = Ident::new(gen_module_name, Span::call_site());

    let wrapped_clients = quote! {
        use super::#gen_module_ident::*;

        #(
            #modules
        )*
    };

    let output = wrapped_clients.to_string();
    let file = syn::parse_file(&output).unwrap();
    let formatted = prettyplease::unparse(&file);

    let output_file = service_file.with_file_name(service_file.file_stem().unwrap().to_str().unwrap().to_owned() + "_pool.rs");

    fs::write(output_file, formatted).unwrap();

    Ok(())
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

fn find_grpc_methods(impl_item: &syn::ItemImpl) -> impl Iterator<Item = &syn::ImplItemFn> {
    impl_item.items.iter().filter_map(|item| match item {
        syn::ImplItem::Fn(impl_item_fn) => Some(impl_item_fn),
        _ => None,
    }).filter(|method| {
        is_grpc_method(method)
    })
}

fn is_grpc_method(method: &syn::ImplItemFn) -> bool {
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

fn wrap_client(impl_item: &syn::ItemImpl) -> TokenStream {
    let struct_ident = match *impl_item.self_ty {
        syn::Type::Path(ref path) => &path.path.segments.last().unwrap().ident,
        _ => return TokenStream::new(),
    };

    let struct_name = struct_ident.to_string();
    let service_name = struct_name.strip_suffix("Client").unwrap_or(&struct_name);
    let client_pool_ident = Ident::new(&format!("{service_name}ClientPool"), Span::call_site());

    info!("Generating {client_pool_ident} from {struct_ident} ({service_name} service)");

    let methods = find_grpc_methods(impl_item).map(|method| {
        generate_method(method, &struct_ident)
    });

    quote! {
        #[derive(Clone)]
        pub struct #client_pool_ident {
            pool: ::auto_discovery::ChannelPool,
        }

        impl #client_pool_ident {
            pub async fn new(endpoint: ::auto_discovery::EndpointTemplate) -> Self {
                Self {
                    pool: ::auto_discovery::ChannelPoolBuilder::new(endpoint).build().await.unwrap(),
                }
            }
        }

        impl From<::auto_discovery::ChannelPool> for #client_pool_ident {
            fn from(pool: ::auto_discovery::ChannelPool) -> Self {
                Self { pool }
            }
        }

        impl #client_pool_ident {
            #(
                #methods
            )*
        }
    }
}

fn generate_method(method: &syn::ImplItemFn, client_name: &Ident) -> TokenStream {
    let fn_sig = method.sig.clone();
    let mut fn_inputs = fn_sig.inputs.clone();

    let fn_name = &fn_sig.ident;
    let fn_retry_name = Ident::new(&format!("{}_with_retry", fn_name), Span::call_site());

    match fn_inputs.first_mut() {
        Some(syn::FnArg::Receiver(recv)) => {
            *recv = remove_ref_from_receiver(recv.clone());
        }
        _ => panic!("Expected first argument to be a receiver"),
    }
    let fn_return = &fn_sig.output;

    let mut retry_sig = fn_sig.clone();
    retry_sig.ident = Ident::new(&format!("{}_with_retry", fn_name), Span::call_site());
    retry_sig.generics = syn::Generics::default();

    quote! {
        pub async fn #fn_name(#fn_inputs) #fn_return {
            self. #fn_retry_name ::<::auto_discovery::DefaultRetryPolicy>(
                request,
            ).await
        }

        pub async fn #fn_retry_name <RP: ::auto_discovery::RetryPolicy>(#fn_inputs) #fn_return {
            let (metadata, extensions, message) = request.into_request().into_parts();
            let mut tries = 0;
            loop {
                tries += 1;

                // Get channel of random index.
                let (ip_address, channel) = self.pool.get_channel().await.ok_or_else(|| {
                    tonic::Status::unavailable("No ready channels")
                })?;

                let request = tonic::Request::from_parts(
                    metadata.clone(),
                    extensions.clone(),
                    message.clone(),
                );
                let result = #client_name::new(channel).#fn_name(request).await;

                match result {
                    Ok(response) => {
                        return Ok(response);
                    }
                    Err(e) => {
                        let ::auto_discovery::RetryCheckResult(server_status, retry_time) = RP::should_retry(&e, tries);
                        if matches!(server_status, ::auto_discovery::ServerStatus::Dead) {
                            // If the server is dead, we should report it.
                            self.pool.report_broken(ip_address).await;
                        }

                        match retry_time {
                            ::auto_discovery::RetryTime::DoNotRetry => {
                                // Do not retry and do not report broken endpoint.
                                return Err(e);
                            }
                            ::auto_discovery::RetryTime::Immediately => {
                                // Retry immediately.
                                continue;
                            }
                            ::auto_discovery::RetryTime::After(duration) => {
                                // Wait for the specified duration before retrying.
                                // todo-interface: Don't require client to have tokio dependency.
                                ::auto_discovery::deps::sleep(duration).await;
                            }
                        }
                    }
                }
            }
        }
    }
}

fn remove_ref_from_receiver(mut recv: Receiver) -> Receiver {
    recv.mutability = None;
    match *recv.ty {
        syn::Type::Reference(ref mut ref_type) => {
            ref_type.mutability = None;
        },
        _ => {}
    }
    recv
}
