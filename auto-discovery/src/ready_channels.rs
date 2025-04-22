use std::net::IpAddr;

use rand::Rng;

use tokio::sync::RwLock;
use tonic::transport::Channel;

#[derive(Debug, Default)]
pub(crate) struct ReadyChannels {
    channels: RwLock<Vec<(IpAddr, Channel)>>,
}

impl ReadyChannels {
    pub(crate) async fn find(&self, ip: IpAddr) -> Option<Channel> {
        self.channels
            .read()
            .await
            .iter()
            .find_map(|(addr, channel)| {
                if *addr == ip {
                    Some(channel.clone())
                } else {
                    None
                }
            })
    }

    pub(crate) async fn get_any(&self) -> Option<(IpAddr, Channel)> {
        let read_access = self.channels.read().await;
        if read_access.is_empty() {
            return None;
        }
        // If we keep track of what channels are currently being used, we could better load balance them.
        let index = rand::rng().random_range(0..read_access.len());
        Some(read_access[index].clone())
    }

    pub(crate) async fn add(&self, ip: IpAddr, channel: Channel) {
        self.channels.write().await.push((ip, channel));
    }

    pub(crate) async fn remove(&self, ip: IpAddr) {
        let mut write_access = self.channels.write().await;
        if let Some(index) = write_access.iter().position(|(addr, _)| *addr == ip) {
            write_access.swap_remove(index);
        }
    }

    pub(crate) async fn replace_with(&self, new: Vec<(IpAddr, Channel)>) {
        *self.channels.write().await = new;
    }
}
