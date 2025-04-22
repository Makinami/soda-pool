#[macro_export]
macro_rules! define_client {
    ($client_type:ident) => {
        #[derive(Clone)]
        pub struct $client_type {
            base: $crate::WrappedClient,
        }

        impl $client_type {
            pub async fn new(endpoint: $crate::EndpointTemplate) -> Result<Self, $crate::WrapperClientBuilderError> {
                Ok(Self {
                    base: $crate::WrappedClientBuilder::new(endpoint).build().await?,
                })
            }

            async fn get_channel(&self) -> Option<(std::net::IpAddr, tonic::transport::Channel)> {
                self.base.get_channel().await
            }

            async fn report_broken(&self, ip_address: std::net::IpAddr) {
                self.base.report_broken(ip_address).await
            }
        }

        impl From<$crate::WrappedClient> for $client_type {
            fn from(base: $crate::WrappedClient) -> Self {
                Self { base }
            }
        }
    };
    (
        $client_type:ident, $original_client:ident, $(($name:ident, $request:ty, $response:ty)),+ $(,)?) => {
        define_client!($client_type);
        impl $client_type {
            $(
                define_method!($original_client, $name, $request, $response);
            )+
        }
    };
    ($client_type:ident, $(($original_client:ident, $name:ident, $request:ty, $response:ty)),+) => {
        define_client!($client_type);
        impl $client_type {
            $(
                define_method!($original_client, $name, $request, $response);
            )+
        }
    };
}

#[macro_export]
macro_rules! define_method {
    ($client:ident, $name:ident, $request:ty, $response:ty) => {
        $crate::deps::paste! {
            pub async fn $name(
                &self,
                request: impl tonic::IntoRequest<$request>,
            ) -> Result<tonic::Response<$response>, tonic::Status> {
                self.[<$name _with_retry>]::<$crate::DefaultRetryPolicy>(
                    request,
                ).await
            }

            pub async fn [<$name _with_retry>] <RP: $crate::RetryPolicy>(
                &self,
                request: impl tonic::IntoRequest<$request>,
            ) -> Result<tonic::Response<$response>, tonic::Status> {
                let (metadata, extensions, message) = request.into_request().into_parts();
                let mut tries = 0;
                loop {
                    tries += 1;

                    // Get channel of random index.
                    let (ip_address, channel) = self.get_channel().await.ok_or_else(|| {
                        tonic::Status::unavailable("No ready channels")
                    })?;

                    let request = tonic::Request::from_parts(
                        metadata.clone(),
                        extensions.clone(),
                        message.clone(),
                    );
                    let result = $client::new(channel).$name(request).await;

                    match result {
                        Ok(response) => {
                            return Ok(response);
                        }
                        Err(e) => {
                            let $crate::RetryCheckResult(server_status, retry_time) = RP::should_retry(&e, tries);
                            if matches!(server_status, $crate::ServerStatus::Dead) {
                                // If the server is dead, we should report it.
                                self.report_broken(ip_address).await;
                            }

                            match retry_time {
                                $crate::RetryTime::DoNotRetry => {
                                    // Do not retry and do not report broken endpoint.
                                    return Err(e);
                                }
                                $crate::RetryTime::Immediately => {
                                    // Retry immediately.
                                    continue;
                                }
                                $crate::RetryTime::After(duration) => {
                                    // Wait for the specified duration before retrying.
                                    // todo-interface: Don't require client to have tokio dependency.
                                    $crate::deps::sleep(duration).await;
                                }
                            }
                        }
                    }
                }
            }
        }
    };
}
