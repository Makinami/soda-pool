use auto_discovery::{EndpointTemplate, WrappedStatus, define_method};
use auto_discovery::{WrappedClient as Base, WrappedClientBuilder as BaseBuilder};
use std::net::IpAddr;
use tonic::transport::Channel;

use crate::health_client::HealthClient;

#[derive(Clone)]
pub struct WrappedClient {
    base: Base,
}

impl WrappedClient {
    pub async fn new(endpoint: EndpointTemplate) -> Self {
        Self {
            base: BaseBuilder::new(endpoint).build().await.unwrap(),
        }
    }

    async fn get_channel(&self) -> Result<(IpAddr, Channel), WrappedStatus> {
        self.base.get_channel().await
    }

    async fn report_broken(&self, ip_address: IpAddr) {
        self.base.report_broken(ip_address).await
    }
}

impl WrappedClient {
    define_method!(HealthClient, is_alive, (), crate::health::IsAliveResponse);
}
