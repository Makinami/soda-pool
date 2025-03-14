use auto_discovery::WrappedClient as Base;
use auto_discovery::{EndpointTemplate, WrappedStatus, define_method};
use std::{net::IpAddr, time::Duration};
use tonic::transport::Channel;

use crate::health_client::HealthClient;

#[derive(Clone)]
pub struct WrappedClient {
    base: Base,
}

impl WrappedClient {
    pub fn new(endpoint: EndpointTemplate, dns_interval: Duration) -> Self {
        Self {
            base: Base::new(endpoint, dns_interval),
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
