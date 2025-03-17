use std::{
    collections::BinaryHeap,
    mem::replace,
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use rand::Rng;
use tokio::{sync::RwLock, task::JoinHandle, time::interval};
use tonic::{Status, transport::Channel};
use tracing::{debug, info, trace, warn};

use crate::{
    broken_endpoints::BrokenEndpoints, dns::ToSocketAddrs, endpoint_template::EndpointTemplate,
};

#[derive(Clone)]
pub struct WrappedClient {
    // todo-performance: Consider using another data structure for ready_clients.
    // Vec was a first default choice because it's simple and easy to work with,
    // but I haven't thought about other options yet.
    ready_clients: Arc<RwLock<Vec<(IpAddr, Channel)>>>,
    broken_endpoints: Arc<BrokenEndpoints>,

    _dns_lookup_task: Arc<JoinHandle<()>>,
    _doctor_task: Arc<JoinHandle<()>>,
}

impl WrappedClient {
    // todo-interface: Consider what parameters are necessary and how to best pass them.
    // todo-interface: Return a function that won't resolve until first DNS lookup finishes.
    //                 This will prevent NoReadeChannels error from happening when user tries to call an API immediately after creating a client.
    pub fn new(endpoint: EndpointTemplate, dns_interval: Duration) -> Self {
        let ready_clients = Arc::new(RwLock::new(Vec::new()));
        let broken_endpoints = Arc::new(BrokenEndpoints::default());

        let dns_lookup_task = {
            // Get shared ownership of the resources.
            let ready_clients = ready_clients.clone();
            let broken_endpoints = broken_endpoints.clone();
            let endpoint = endpoint.clone();

            tokio::spawn(async move {
                let mut interval = interval(dns_interval);
                loop {
                    interval.tick().await;

                    // Resolve domain to IP addresses.
                    let addresses = (endpoint.domain(), 0)
                        .to_socket_addrs()
                        .unwrap()
                        .map(|addr| addr.ip());

                    let mut ready = Vec::new();
                    let mut broken = BinaryHeap::new();

                    for address in addresses {
                        // Skip if the address is already in ready_clients.
                        if let Some(entry) = ready_clients
                            .read()
                            .await
                            .iter()
                            .find(|(e, _)| e == &address)
                            .cloned()
                        {
                            trace!("Skipping {:?} as already ready", address);
                            ready.push(entry);
                            continue;
                        }

                        // Skip if the address is already in broken_endpoints.
                        if let Some(entry) = broken_endpoints.get_entry(address) {
                            trace!("Skipping {:?} as already broken", address);
                            broken.push(entry);
                            continue;
                        }

                        debug!("Connecting to: {:?}", address);
                        let endpoint = endpoint.build(address);
                        let channel = endpoint.connect().await;
                        if let Ok(channel) = channel {
                            ready.push((address, channel));
                        } else {
                            broken.push((Instant::now() + Duration::from_secs(1), address));
                        }
                    }

                    // Replace a list of clients stored in `ready_clients`` with the new ones constructed in `ready`.
                    let _ = replace(&mut *ready_clients.write().await, ready);
                    broken_endpoints.replace_with(broken);
                }
            })
        };

        let doctor_task = {
            // Get shared ownership of the resources.
            let ready_clients = ready_clients.clone();
            let broken_endpoints = broken_endpoints.clone();

            tokio::spawn(async move {
                let wait_duration = Duration::from_secs(1);
                loop {
                    // todo-performance: block_in_place is not the best solution here. It will prevent further tasks from being scheduled on the current thread,
                    // but may block the ones already scheduled. It's ok for now for testing but should be avoided in production.
                    let Some(ip_address) = tokio::task::block_in_place(|| {
                        broken_endpoints.next_broken_ip_address(wait_duration)
                    }) else {
                        continue;
                    };

                    let connection_test_result = endpoint.build(ip_address).connect().await;

                    if let Ok(channel) = connection_test_result {
                        info!("Connection established to {:?}", ip_address);
                        // note: If only this task wouldn't use ready_clients, it would be trivial to move it to BrokenEndpoints itself.
                        // Maybe channel communication would be better here.
                        ready_clients.write().await.push((ip_address, channel));
                    } else {
                        // todo-performance: implement exponential backoff.
                        warn!("Can't connect to {:?}", ip_address);
                        broken_endpoints.add_address(ip_address);
                    }
                }
            })
        };

        Self {
            ready_clients,
            broken_endpoints,
            _dns_lookup_task: Arc::new(dns_lookup_task),
            _doctor_task: Arc::new(doctor_task),
        }
    }

    pub async fn get_channel(&self) -> Result<(IpAddr, Channel), WrappedStatus> {
        let read_access = self.ready_clients.read().await;
        if read_access.is_empty() {
            // If there are no healthy channels, maybe we could trigger DNS lookup from here?
            return Err(WrappedStatus::NoReadyChannels);
        }
        // If we keep track of what channels are currently being used, we could better load balance them.
        let index = rand::rng().random_range(0..read_access.len());
        Ok(read_access[index].clone())
    }

    pub async fn report_broken(&self, ip_address: IpAddr) {
        let mut write_access = self.ready_clients.write().await;
        let index = write_access.iter().position(|(e, _)| e == &ip_address);
        if let Some(index) = index {
            write_access.remove(index);
        }
        self.broken_endpoints.add_address(ip_address);
    }
}

#[macro_export]
macro_rules! define_method {
    ($client:ident, $name:ident, $request:ty, $response:ty) => {
        pub async fn $name(
            &self,
            request: impl tonic::IntoRequest<$request>,
        ) -> Result<tonic::Response<$response>, WrappedStatus> {
            let (metadata, extensions, message) = request.into_request().into_parts();
            loop {
                // Get channel of random index.
                let (ip_address, channel) = self.get_channel().await?;

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
                        // Initial tests suggest that source of the error is set only when it comes from the library (e.g. connection refused).
                        if std::error::Error::source(&e).is_some() {
                            // If the error happened because the channel is dead (e.g. connection refused),
                            // add the address to broken endpoints and retry request thought another channel.
                            self.report_broken(ip_address).await;
                        } else {
                            // All errors that come from the server are not errors in a sense of connection problems so they don't set source.
                            return Err(e.into());
                        }
                    }
                }
            }
        }
    };
}

// todo-interface: In general take care of error handling. This is just a quick draft.
#[derive(Debug)]
pub enum WrappedStatus {
    Status(tonic::Status),
    NoReadyChannels,
}

impl From<Status> for WrappedStatus {
    fn from(e: Status) -> Self {
        WrappedStatus::Status(e)
    }
}
