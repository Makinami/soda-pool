use super::health::*;
pub mod health_client {
    use super::super::health::health_client::*;
    #[derive(Clone)]
    pub struct HealthClientPool {
        pool: ::auto_discovery::ChannelPool,
    }
    impl HealthClientPool {
        pub async fn new(endpoint: ::auto_discovery::EndpointTemplate) -> Self {
            Self {
                pool: ::auto_discovery::ChannelPoolBuilder::new(endpoint)
                    .build()
                    .await
                    .unwrap(),
            }
        }
    }
    impl From<::auto_discovery::ChannelPool> for HealthClientPool {
        fn from(pool: ::auto_discovery::ChannelPool) -> Self {
            Self { pool }
        }
    }
    impl HealthClientPool {
        pub async fn is_alive(
            &self,
            request: impl tonic::IntoRequest<()>,
        ) -> std::result::Result<
            tonic::Response<super::IsAliveResponse>,
            tonic::Status,
        > {
            self.is_alive_with_retry::<::auto_discovery::DefaultRetryPolicy>(request)
                .await
        }
        pub async fn is_alive_with_retry<RP: ::auto_discovery::RetryPolicy>(
            &self,
            request: impl tonic::IntoRequest<()>,
        ) -> std::result::Result<
            tonic::Response<super::IsAliveResponse>,
            tonic::Status,
        > {
            let (metadata, extensions, message) = request.into_request().into_parts();
            let mut tries = 0;
            loop {
                tries += 1;
                let (ip_address, channel) = self
                    .pool
                    .get_channel()
                    .await
                    .ok_or_else(|| { tonic::Status::unavailable("No ready channels") })?;
                let request = tonic::Request::from_parts(
                    metadata.clone(),
                    extensions.clone(),
                    message.clone(),
                );
                let result = HealthClient::new(channel).is_alive(request).await;
                match result {
                    Ok(response) => {
                        return Ok(response);
                    }
                    Err(e) => {
                        let ::auto_discovery::RetryCheckResult(
                            server_status,
                            retry_time,
                        ) = RP::should_retry(&e, tries);
                        if matches!(
                            server_status, ::auto_discovery::ServerStatus::Dead
                        ) {
                            self.pool.report_broken(ip_address).await;
                        }
                        match retry_time {
                            ::auto_discovery::RetryTime::DoNotRetry => {
                                return Err(e);
                            }
                            ::auto_discovery::RetryTime::Immediately => {
                                continue;
                            }
                            ::auto_discovery::RetryTime::After(duration) => {
                                ::auto_discovery::deps::sleep(duration).await;
                            }
                        }
                    }
                }
            }
        }
    }
}
pub mod echo_client {
    use super::super::health::echo_client::*;
    #[derive(Clone)]
    pub struct EchoClientPool {
        pool: ::auto_discovery::ChannelPool,
    }
    impl EchoClientPool {
        pub async fn new(endpoint: ::auto_discovery::EndpointTemplate) -> Self {
            Self {
                pool: ::auto_discovery::ChannelPoolBuilder::new(endpoint)
                    .build()
                    .await
                    .unwrap(),
            }
        }
    }
    impl From<::auto_discovery::ChannelPool> for EchoClientPool {
        fn from(pool: ::auto_discovery::ChannelPool) -> Self {
            Self { pool }
        }
    }
    impl EchoClientPool {
        pub async fn echo_message(
            &self,
            request: impl tonic::IntoRequest<super::EchoRequest>,
        ) -> std::result::Result<tonic::Response<super::EchoResponse>, tonic::Status> {
            self.echo_message_with_retry::<::auto_discovery::DefaultRetryPolicy>(request)
                .await
        }
        pub async fn echo_message_with_retry<RP: ::auto_discovery::RetryPolicy>(
            &self,
            request: impl tonic::IntoRequest<super::EchoRequest>,
        ) -> std::result::Result<tonic::Response<super::EchoResponse>, tonic::Status> {
            let (metadata, extensions, message) = request.into_request().into_parts();
            let mut tries = 0;
            loop {
                tries += 1;
                let (ip_address, channel) = self
                    .pool
                    .get_channel()
                    .await
                    .ok_or_else(|| { tonic::Status::unavailable("No ready channels") })?;
                let request = tonic::Request::from_parts(
                    metadata.clone(),
                    extensions.clone(),
                    message.clone(),
                );
                let result = EchoClient::new(channel).echo_message(request).await;
                match result {
                    Ok(response) => {
                        return Ok(response);
                    }
                    Err(e) => {
                        let ::auto_discovery::RetryCheckResult(
                            server_status,
                            retry_time,
                        ) = RP::should_retry(&e, tries);
                        if matches!(
                            server_status, ::auto_discovery::ServerStatus::Dead
                        ) {
                            self.pool.report_broken(ip_address).await;
                        }
                        match retry_time {
                            ::auto_discovery::RetryTime::DoNotRetry => {
                                return Err(e);
                            }
                            ::auto_discovery::RetryTime::Immediately => {
                                continue;
                            }
                            ::auto_discovery::RetryTime::After(duration) => {
                                ::auto_discovery::deps::sleep(duration).await;
                            }
                        }
                    }
                }
            }
        }
    }
}
