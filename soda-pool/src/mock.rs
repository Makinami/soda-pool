use std::net::{IpAddr, Ipv4Addr};

use tonic::{async_trait, transport::Channel};

use crate::ChannelPool;

#[derive(Clone, Debug)]
pub struct MockChannelPool {
    channel: Channel,
}

impl MockChannelPool {
    #[must_use]
    pub fn new(channel: Channel) -> Self {
        Self { channel }
    }
}

#[async_trait]
impl ChannelPool for MockChannelPool {
    async fn get_channel(&self) -> Option<(IpAddr, Channel)> {
        Some((IpAddr::V4(Ipv4Addr::LOCALHOST), self.channel.clone()))
    }

    async fn report_broken(&self, _ip_address: IpAddr) {
        // No-op
    }
}
