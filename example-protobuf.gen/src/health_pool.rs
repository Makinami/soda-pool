#![allow(clippy::unit_arg, clippy::clone_on_copy)]
use super::health::*;
pub mod health_client {
    use super::super::health::health_client::*;
    #[derive(Clone)]
    pub struct HealthClientPool {
        pool: soda_pool::ChannelPool,
    }
    impl HealthClientPool {
        pub async fn new(pool: soda_pool::ChannelPool) -> Self {
            Self { pool }
        }
        pub fn new_from_endpoint(endpoint: soda_pool::EndpointTemplate) -> Self {
            Self {
                pool: soda_pool::ChannelPoolBuilder::new(endpoint).build(),
            }
        }
    }
    impl From<soda_pool::ChannelPool> for HealthClientPool {
        fn from(pool: soda_pool::ChannelPool) -> Self {
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
            self.is_alive_with_retry::<soda_pool::DefaultRetryPolicy>(request).await
        }
        pub async fn is_alive_with_retry<RP: soda_pool::RetryPolicy>(
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
                        let (server_status, retry_time) = RP::should_retry(&e, tries);
                        if matches!(server_status, soda_pool::ServerStatus::Dead) {
                            self.pool.report_broken(ip_address).await;
                        }
                        match retry_time {
                            soda_pool::RetryTime::DoNotRetry => {
                                return Err(e);
                            }
                            soda_pool::RetryTime::Immediately => {
                                continue;
                            }
                            soda_pool::RetryTime::After(duration) => {
                                soda_pool::deps::sleep(duration).await;
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
        pool: soda_pool::ChannelPool,
    }
    impl EchoClientPool {
        pub async fn new(pool: soda_pool::ChannelPool) -> Self {
            Self { pool }
        }
        pub fn new_from_endpoint(endpoint: soda_pool::EndpointTemplate) -> Self {
            Self {
                pool: soda_pool::ChannelPoolBuilder::new(endpoint).build(),
            }
        }
    }
    impl From<soda_pool::ChannelPool> for EchoClientPool {
        fn from(pool: soda_pool::ChannelPool) -> Self {
            Self { pool }
        }
    }
    impl EchoClientPool {
        pub async fn echo_message(
            &self,
            request: impl tonic::IntoRequest<super::EchoRequest>,
        ) -> std::result::Result<tonic::Response<super::EchoResponse>, tonic::Status> {
            self.echo_message_with_retry::<soda_pool::DefaultRetryPolicy>(request).await
        }
        pub async fn echo_message_with_retry<RP: soda_pool::RetryPolicy>(
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
                        let (server_status, retry_time) = RP::should_retry(&e, tries);
                        if matches!(server_status, soda_pool::ServerStatus::Dead) {
                            self.pool.report_broken(ip_address).await;
                        }
                        match retry_time {
                            soda_pool::RetryTime::DoNotRetry => {
                                return Err(e);
                            }
                            soda_pool::RetryTime::Immediately => {
                                continue;
                            }
                            soda_pool::RetryTime::After(duration) => {
                                soda_pool::deps::sleep(duration).await;
                            }
                        }
                    }
                }
            }
        }
    }
}
