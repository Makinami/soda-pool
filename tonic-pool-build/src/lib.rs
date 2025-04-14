use std::{
    fs, path::{Path, PathBuf}
};

use proc_macro2::{Span, TokenStream};
use syn::{parse_file, parse_quote, Ident, Item, ItemMod, PathSegment, Stmt};
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
        let dir = self.dir.as_ref().ok_or(())?; // todo

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
                    // todo: don't hardcode this. Need a nicer way to get the path to the original client.
                    use super::super::health::#module_ident::*;
                    #(
                        #pooled_clients
                    )*
                }
            }
        });

    let gen_module_name = service_file.file_stem().unwrap().to_str().unwrap();
    let gen_module_ident = Ident::new(gen_module_name, Span::call_site());
println!("gen_module_name: {gen_module_name}");
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
            base: ::auto_discovery::WrappedClient,
        }

        impl #client_pool_ident {
            pub async fn new(endpoint: ::auto_discovery::EndpointTemplate) -> Self {
                Self {
                    base: ::auto_discovery::WrappedClientBuilder::new(endpoint).build().await.unwrap(),
                }
            }

            async fn get_channel(&self) -> Result<(std::net::IpAddr, tonic::transport::Channel), ::auto_discovery::WrappedClientError> {
                self.base.get_channel().await
            }

            async fn report_broken(&self, ip_address: std::net::IpAddr) -> Result<(), ::auto_discovery::WrappedClientError> {
                self.base.report_broken(ip_address).await
            }
        }

        impl From<::auto_discovery::WrappedClient> for #client_pool_ident {
            fn from(base: ::auto_discovery::WrappedClient) -> Self {
                Self { base }
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
    let fn_vis = &method.vis;
    let fn_sig = &method.sig;
    let fn_name = &fn_sig.ident;

    quote! {
        // todo: Remove mut from self
        #fn_vis #fn_sig {
            let (metadata, extensions, message) = request.into_request().into_parts();
            loop {
                // Get channel of random index.
                let (ip_address, channel) = self.get_channel().await?;

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
                        // Initial tests suggest that source of the error is set only when it comes from the library (e.g. connection refused).
                        if std::error::Error::source(&e).is_some() {
                            // If the error happened because the channel is dead (e.g. connection refused),
                            // add the address to broken endpoints and retry request thought another channel.
                            self.report_broken(ip_address).await;
                        } else {
                            // All errors that come from the server are not errors in a sense of connection problems so they don't set source.
                            return Err(e.into());
                        }
                    }
                }
            }
        }
    }
}
