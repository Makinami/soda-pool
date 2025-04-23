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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use tonic::transport::Endpoint;

    use super::*;

    fn default_channel() -> Channel {
        Endpoint::from_static("http://localhost:8080").connect_lazy()
    }

    #[tokio::test]
    async fn find() {
        let ready_channels = ReadyChannels::default();
        assert!(ready_channels.find(Ipv4Addr::LOCALHOST.into()).await.is_none());

        ready_channels.add(Ipv6Addr::LOCALHOST.into(), default_channel()).await;
        assert!(ready_channels.find(Ipv4Addr::LOCALHOST.into()).await.is_none());

        ready_channels.add(Ipv4Addr::LOCALHOST.into(), default_channel()).await;
        assert!(ready_channels.find(Ipv4Addr::LOCALHOST.into()).await.is_some());
    }

    #[tokio::test]
    async fn get_any() {
        let ready_channels = ReadyChannels::default();
        assert!(ready_channels.get_any().await.is_none());

        for i in 0..128 {
            ready_channels
                .add(Ipv4Addr::new(127, 0, 0, i).into(), default_channel())
                .await;
        }

        let mut found = vec![];
        for _ in 0..10 {
            if let Some((ip, _)) = ready_channels.get_any().await {
                found.push(ip);
            } else {
                panic!("No channels found");
            }
        }
        found.sort();
        found.dedup();

        assert!(found.len() > 1);
    }

    #[tokio::test]
    async fn add() {
        let ready_channels = ReadyChannels::default();
        assert!(ready_channels.channels.read().await.is_empty());

        ready_channels.add(Ipv4Addr::LOCALHOST.into(), default_channel()).await;
        assert_eq!(ready_channels.channels.read().await.len(), 1);

        ready_channels.add(Ipv6Addr::LOCALHOST.into(), default_channel()).await;
        assert_eq!(ready_channels.channels.read().await.len(), 2);
    }

    #[tokio::test]
    async fn remove() {
        let ready_channels = ReadyChannels::default();
        ready_channels.add(Ipv4Addr::LOCALHOST.into(), default_channel()).await;
        ready_channels.add(Ipv6Addr::LOCALHOST.into(), default_channel()).await;

        assert_eq!(ready_channels.channels.read().await.len(), 2);

        ready_channels.remove(Ipv4Addr::new(127, 0, 0, 2).into()).await;
        assert_eq!(ready_channels.channels.read().await.len(), 2);

        ready_channels.remove(Ipv4Addr::LOCALHOST.into()).await;
        assert_eq!(ready_channels.channels.read().await.len(), 1);
        assert!(ready_channels.find(Ipv4Addr::LOCALHOST.into()).await.is_none());

        ready_channels.remove(Ipv6Addr::LOCALHOST.into()).await;
        assert!(ready_channels.channels.read().await.is_empty());
        assert!(ready_channels.find(Ipv6Addr::LOCALHOST.into()).await.is_none());
    }

    #[tokio::test]
    async fn replace_with() {
        let ready_channels = ReadyChannels::default();
        ready_channels.add(Ipv4Addr::LOCALHOST.into(), default_channel()).await;
        ready_channels.add(Ipv6Addr::LOCALHOST.into(), default_channel()).await;

        assert_eq!(ready_channels.channels.read().await.len(), 2);

        let new = vec![
            (Ipv4Addr::new(127, 0, 0, 2).into(), default_channel()),
            (Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 2).into(), default_channel()),
        ];
        ready_channels.replace_with(new.clone()).await;

        {
            let guard = ready_channels.channels.read().await;
            assert_eq!(
                guard.iter().map(|(ip, _)| *ip).collect::<Vec<_>>(),
                new.iter().map(|(ip, _)| *ip).collect::<Vec<_>>(),
            );
        }
    }
}
