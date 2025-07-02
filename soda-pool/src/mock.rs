//! Mock implementation of the `ChannelPool` trait for testing purposes.
//!
//! This implementation is useful for unit testing. It should not conceivably be used in production.

use std::net::{IpAddr, Ipv4Addr};

use tonic::{async_trait, transport::Channel};

use crate::ChannelPool;

/// Mock implementation of the `ChannelPool` trait for unit testing.
#[derive(Clone, Debug)]
pub struct MockChannelPool {
    channel: Channel,
}

impl MockChannelPool {
    /// Creates a new [`MockChannelPool`] with the given channel.
    ///
    /// The passed here channel will always be returned by the [`get_channel`](MockChannelPool::get_channel)
    /// method, together with IPv4 localhost address. While any channel will do,
    /// one way is to use a local stream as shown in the [tonic's
    /// example](https://github.com/hyperium/tonic/blob/master/examples/src/mock/mock.rs).
    #[must_use]
    pub fn new(channel: Channel) -> Self {
        Self { channel }
    }
}

#[async_trait]
impl ChannelPool for MockChannelPool {
    /// Returns a channel and an IP address (IPv4 localhost) for testing purposes.
    ///
    /// This method always returns the same channel that was provided during
    /// the creation of the [`MockChannelPool`], along with the IPv4 localhost
    /// address.
    async fn get_channel(&self) -> Option<(IpAddr, Channel)> {
        Some((IpAddr::V4(Ipv4Addr::LOCALHOST), self.channel.clone()))
    }

    /// Reports the given IP address as broken.
    ///
    /// This is a no-op in the mock implementation.
    async fn report_broken(&self, _ip_address: IpAddr) {
        // No-op
    }
}
