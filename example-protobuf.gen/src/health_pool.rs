use super::health::*;
pub mod health_client {
    use super::super::health::health_client::*;
    #[derive(Clone)]
    pub struct HealthClientPool {
        base: ::auto_discovery::WrappedClient,
    }
    impl HealthClientPool {
        pub async fn new(endpoint: ::auto_discovery::EndpointTemplate) -> Self {
            Self {
                base: ::auto_discovery::WrappedClientBuilder::new(endpoint)
                    .build()
                    .await
                    .unwrap(),
            }
        }
        async fn get_channel(
            &self,
        ) -> Result<
            (std::net::IpAddr, tonic::transport::Channel),
            ::auto_discovery::WrappedClientError,
        > {
            self.base.get_channel().await
        }
        async fn report_broken(
            &self,
            ip_address: std::net::IpAddr,
        ) -> Result<(), ::auto_discovery::WrappedClientError> {
            self.base.report_broken(ip_address).await
        }
    }
    impl From<::auto_discovery::WrappedClient> for HealthClientPool {
        fn from(base: ::auto_discovery::WrappedClient) -> Self {
            Self { base }
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
            let (metadata, extensions, message) = request.into_request().into_parts();
            loop {
                let (ip_address, channel) = self.get_channel().await?;
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
                        if std::error::Error::source(&e).is_some() {
                            self.report_broken(ip_address).await;
                        } else {
                            return Err(e.into());
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
        base: ::auto_discovery::WrappedClient,
    }
    impl EchoClientPool {
        pub async fn new(endpoint: ::auto_discovery::EndpointTemplate) -> Self {
            Self {
                base: ::auto_discovery::WrappedClientBuilder::new(endpoint)
                    .build()
                    .await
                    .unwrap(),
            }
        }
        async fn get_channel(
            &self,
        ) -> Result<
            (std::net::IpAddr, tonic::transport::Channel),
            ::auto_discovery::WrappedClientError,
        > {
            self.base.get_channel().await
        }
        async fn report_broken(
            &self,
            ip_address: std::net::IpAddr,
        ) -> Result<(), ::auto_discovery::WrappedClientError> {
            self.base.report_broken(ip_address).await
        }
    }
    impl From<::auto_discovery::WrappedClient> for EchoClientPool {
        fn from(base: ::auto_discovery::WrappedClient) -> Self {
            Self { base }
        }
    }
    impl EchoClientPool {
        pub async fn echo_message(
            &mut self,
            request: impl tonic::IntoRequest<super::EchoRequest>,
        ) -> std::result::Result<tonic::Response<super::EchoResponse>, tonic::Status> {
            let (metadata, extensions, message) = request.into_request().into_parts();
            loop {
                let (ip_address, channel) = self.get_channel().await?;
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
                        if std::error::Error::source(&e).is_some() {
                            self.report_broken(ip_address).await;
                        } else {
                            return Err(e.into());
                        }
                    }
                }
            }
        }
    }
}
