use std::{
    fs, path::{Path, PathBuf}
};

use syn::{parse_file, Item, ItemMod, PathSegment};
use tracing::{info, debug};

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

    find_client_modules(&file)
        .for_each(|client_module| {
            debug!("Found client module: {}", client_module.ident);
            find_client_interface_implementations(client_module)
                .for_each(|impl_item| {
                    let struct_name = match *impl_item.self_ty {
                        syn::Type::Path(ref path) => path.path.segments.last().unwrap().ident.to_string(),
                        _ => unreachable!(),
                    };
                    debug!("Found client struct implementation for: {struct_name}");
                    wrap_client(&impl_item);
                });
        });
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

fn wrap_client(impl_item: &syn::ItemImpl) {
    let struct_name = match *impl_item.self_ty {
        syn::Type::Path(ref path) => path.path.segments.last().unwrap().ident.to_string(),
        _ => return,
    };

    let service_name = struct_name.strip_suffix("Client").unwrap_or(&struct_name);
    let client_pool_name = format!("{service_name}ClientPool");

    info!("Generating {client_pool_name} from {struct_name}");

    find_grpc_methods(impl_item).for_each(|method| {
        debug!("Found gRPC method: {}", method.sig.ident);
    });
}
