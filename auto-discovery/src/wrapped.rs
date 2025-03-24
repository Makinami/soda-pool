use std::{collections::BinaryHeap, error::Error, mem::replace, net::IpAddr, sync::Arc, time::Duration};

use chrono::Utc;
use rand::Rng;
use tokio::{
    sync::{RwLock, oneshot::channel},
    task::JoinHandle,
    time::interval,
};
use tonic::{Status, transport::Channel};
use tracing::{debug, info, trace, warn};

use crate::{
    broken_endpoints::BrokenEndpoints, dns::ToSocketAddrs, endpoint_template::EndpointTemplate,
};

pub struct WrappedClientBuilder {
    endpoint: EndpointTemplate,
    dns_interval: Duration,
}

#[derive(Debug)]
pub enum WrapperClientBuilderError {
    FailedToInitiate,
}

impl WrappedClientBuilder {
    pub fn new(endpoint: EndpointTemplate) -> Self {
        Self {
            endpoint,
            // Note: Is this a good default?
            dns_interval: Duration::from_secs(5),
        }
    }

    pub fn dns_interval(mut self, dns_interval: Duration) -> Self {
        self.dns_interval = dns_interval;
        self
    }

    // todo-interface: Consider error type that will be returned when DNS lookup fails or times out.
    pub async fn build(self) -> Result<WrappedClient, WrapperClientBuilderError> {
        let ready_clients = Arc::new(RwLock::new(Vec::new()));
        let broken_endpoints = Arc::new(BrokenEndpoints::default());

        let (initiated_send, initiated_recv) = channel();
        let mut initiated_send = Some(initiated_send);

        let dns_lookup_task = {
            // Get shared ownership of the resources.
            let ready_clients = ready_clients.clone();
            let broken_endpoints = broken_endpoints.clone();
            let endpoint = self.endpoint.clone();

            tokio::spawn(async move {
                let mut interval = interval(self.dns_interval);
                loop {
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
                            broken.push((BackoffTracker::from_failed_times(1), address));
                        }
                    }

                    // Replace a list of clients stored in `ready_clients`` with the new ones constructed in `ready`.
                    let _ = replace(&mut *ready_clients.write().await, ready);
                    broken_endpoints.replace_with(broken);

                    if let Some(initiated_send) = initiated_send.take() {
                        let _ = initiated_send.send(());
                    }

                    interval.tick().await;
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
                    let Some((ip_address, backoff)) = tokio::task::block_in_place(|| {
                        broken_endpoints.next_broken_ip_address(wait_duration)
                    }) else {
                        continue;
                    };

                    let connection_test_result = self.endpoint.build(ip_address).connect().await;

                    if let Ok(channel) = connection_test_result {
                        info!("Connection established to {:?}", ip_address);
                        // note: If only this task wouldn't use ready_clients, it would be trivial to move it to BrokenEndpoints itself.
                        // Maybe channel communication would be better here.
                        ready_clients.write().await.push((ip_address, channel));
                    } else {
                        warn!("Can't connect to {:?}", ip_address);
                        broken_endpoints.add_address_with_backoff(ip_address, backoff);
                    }
                }
            })
        };

        match initiated_recv.await {
            Ok(_) => {}
            Err(_) => {
                return Err(WrapperClientBuilderError::FailedToInitiate);
            }
        }

        Ok(WrappedClient {
            ready_clients,
            broken_endpoints,
            _dns_lookup_task: Arc::new(dns_lookup_task),
            _doctor_task: Arc::new(doctor_task),
        })
    }
}

#[derive(Clone)]
pub struct WrappedClient {
    // Note: For current random load balances, Vec a perfect data structure.
    // However, depending on other algorithms we might want to support,
    // we might want to change it to something else.
    ready_clients: Arc<RwLock<Vec<(IpAddr, Channel)>>>,
    broken_endpoints: Arc<BrokenEndpoints>,

    _dns_lookup_task: Arc<JoinHandle<()>>,
    _doctor_task: Arc<JoinHandle<()>>,
}

impl WrappedClient {
    pub async fn get_channel(&self) -> Result<(IpAddr, Channel), WrappedClientError> {
        let read_access = self.ready_clients.read().await;
        if read_access.is_empty() {
            // If there are no healthy channels, maybe we could trigger DNS lookup from here?
            return Err(WrappedClientError::NoReadyChannels);
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
        ) -> Result<tonic::Response<$response>, tonic::Status> {
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

#[macro_export]
macro_rules! define_client {
    ($client_type:ident) => {
        #[derive(Clone)]
        pub struct $client_type {
            base: $crate::WrappedClient,
        }

        impl $client_type {
            pub async fn new(endpoint: $crate::EndpointTemplate) -> Self {
                Self {
                    base: $crate::WrappedClientBuilder::new(endpoint).build().await.unwrap(),
                }
            }

            async fn get_channel(&self) -> Result<(std::net::IpAddr, tonic::transport::Channel), $crate::WrappedClientError> {
                self.base.get_channel().await
            }

            async fn report_broken(&self, ip_address: std::net::IpAddr) {
                self.base.report_broken(ip_address).await
            }
        }

        impl From<$crate::WrappedClient> for $client_type {
            fn from(base: $crate::WrappedClient) -> Self {
                Self { base }
            }
        }
    };
    (
        $client_type:ident, $original_client:ident, $(($name:ident, $request:ty, $response:ty)),+ $(,)?) => {
        define_client!($client_type);
        impl $client_type {
            $(
                define_method!($original_client, $name, $request, $response);
            )+
        }
    };
    ($client_type:ident, $(($original_client:ident, $name:ident, $request:ty, $response:ty)),+) => {
        define_client!($client_type);
        impl $client_type {
            $(
                define_method!($original_client, $name, $request, $response);
            )+
        }
    };
}

#[derive(Debug)]
pub enum WrappedClientError {
    NoReadyChannels,
}

impl std::fmt::Display for WrappedClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WrappedClientError::NoReadyChannels => {
                std::fmt::Display::fmt("No ready channels", f)
            },
        }
    }
}

impl Error for WrappedClientError {}

impl From<WrappedClientError> for Status {
    fn from(e: WrappedClientError) -> Self {
        match e {
            WrappedClientError::NoReadyChannels => {
                let mut status = Status::new(tonic::Code::Unavailable, "No ready channels");
                status.set_source(Arc::new(e));
                status
            },
        }
    }
}
