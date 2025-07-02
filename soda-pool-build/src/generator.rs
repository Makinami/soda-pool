use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

use crate::model::{GrpcClientFile, GrpcClientImpl, GrpcClientMethod, GrpcClientModule};

impl GrpcClientFile {
    pub(crate) fn generate_pooled_version(&self) -> TokenStream {
        let original_file_module = &self.file_module;
        let client_modules = self
            .client_modules
            .iter()
            .map(|client_module| client_module.generate_pooled_version(&self.file_module));

        quote! {
            #![allow(
                clippy::unit_arg,
                clippy::clone_on_copy,
            )]

            use super::#original_file_module::*;

            #(#client_modules)*
        }
    }
}

impl GrpcClientModule {
    pub(crate) fn generate_pooled_version(&self, original_file_module: &Ident) -> TokenStream {
        let module_name = &self.name;
        let clients = self
            .clients
            .iter()
            .map(GrpcClientImpl::generate_pooled_version);

        quote! {
            pub mod #module_name {
                use super::super::#original_file_module::#module_name::*;

                #(#clients)*
            }
        }
    }
}

impl GrpcClientImpl {
    pub(crate) fn generate_pooled_version(&self) -> TokenStream {
        let client_pool_ident = Ident::new(&format!("{}Pool", self.name), self.name.span());
        let methods = self
            .methods
            .iter()
            .map(|method| method.generate_pooled_version(&self.name));

        quote! {
            #[derive(Clone)]
            pub struct #client_pool_ident {
                pool: std::sync::Arc<dyn soda_pool::ChannelPool + Send + Sync>,
            }

            impl #client_pool_ident {
                pub fn new(pool: impl soda_pool::ChannelPool + Send + Sync + 'static) -> Self {
                    Self { pool: std::sync::Arc::new(pool) }
                }

                pub fn new_from_endpoint(endpoint: soda_pool::EndpointTemplate) -> Self {
                    Self {
                        pool: std::sync::Arc::new(soda_pool::ManagedChannelPoolBuilder::new(endpoint).build()),
                    }
                }
            }

            impl #client_pool_ident {
                #(#methods)*
            }
        }
    }
}

impl GrpcClientMethod {
    pub(crate) fn generate_pooled_version(&self, original_client_name: &Ident) -> TokenStream {
        let method_name = &self.name;
        let method_name_with_retry =
            Ident::new(&format!("{method_name}_with_retry"), method_name.span());
        let request_param = &self.request_type;
        let response_type = &self.response_type;

        quote! {
            pub async fn #method_name(
                &self,
                request: impl tonic::IntoRequest<#request_param>,
            ) -> std::result::Result<tonic::Response<#response_type>, tonic::Status> {
                self.#method_name_with_retry::<soda_pool::DefaultRetryPolicy>(request).await
            }

            pub async fn #method_name_with_retry<RP: soda_pool::RetryPolicy>(
                &self,
                request: impl tonic::IntoRequest<#request_param>,
            ) -> std::result::Result<tonic::Response<#response_type>, tonic::Status> {
                let (metadata, extensions, message) = request.into_request().into_parts();
                let mut tries = 0;
                loop {
                    tries += 1;

                    let (ip_address, channel) = self.pool.get_channel().await.ok_or_else(|| {
                        tonic::Status::unavailable("No ready channels")
                    })?;

                    let request = tonic::Request::from_parts(
                        metadata.clone(),
                        extensions.clone(),
                        message.clone(),
                    );
                    let result = #original_client_name::new(channel).#method_name(request).await;

                    match result {
                        Ok(response) => {
                            return Ok(response);
                        }
                        Err(e) => {
                            let (server_status, retry_time) = RP::should_retry(&e, tries);
                            if matches!(server_status, soda_pool::ServerStatus::Dead) {
                                // If the server is dead, we should report it.
                                self.pool.report_broken(ip_address).await;
                            }

                            match retry_time {
                                soda_pool::RetryTime::DoNotRetry => {
                                    // Do not retry and do not report broken endpoint.
                                    return Err(e);
                                }
                                soda_pool::RetryTime::Immediately => {
                                    // Retry immediately.
                                    continue;
                                }
                                soda_pool::RetryTime::After(duration) => {
                                    // Wait for the specified duration before retrying.
                                    soda_pool::deps::sleep(duration).await;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use proc_macro2::Span;

    use super::*;

    #[test]
    fn ok() {
        let structure = GrpcClientFile {
            file_module: Ident::new("health", Span::call_site()),
            client_modules: vec![
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
            ],
        };
        let actual = structure.generate_pooled_version();
        let expected: TokenStream =
            syn::parse_str(include_str!("test_cases/generator_success.rs")).unwrap();
        assert_eq!(actual.to_string(), expected.to_string());
    }
}
