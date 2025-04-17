use std::{
    collections::BinaryHeap, error::Error, mem::replace, net::IpAddr, sync::Arc, time::Duration,
};

use rand::Rng;
use tokio::{
    sync::{RwLock, oneshot::channel},
    task::{AbortHandle, JoinHandle},
    time::interval,
};
use tonic::{Status, transport::Channel};
use tracing::{debug, info, trace, warn};

use crate::{
    broken_endpoints::{BackoffTracker, BrokenEndpoints, BrokenEndpointsError},
    dns::resolve_domain,
    endpoint_template::EndpointTemplate,
};

#[derive(Debug)]
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

        let dns_lookup_state = Arc::new(RwLock::new(None));
        let dns_lookup_task = {
            // Get shared ownership of the resources.
            let ready_clients = ready_clients.clone();
            let broken_endpoints = broken_endpoints.clone();
            let endpoint = self.endpoint.clone();
            let dns_lookup_state = dns_lookup_state.clone();

            tokio::spawn(async move {
                let mut interval = interval(self.dns_interval);
                loop {
                    let result = check_dns(&endpoint, &ready_clients, &broken_endpoints).await;
                    let _ = replace(&mut *dns_lookup_state.write().await, result.err());

                    if let Some(initiated_send) = initiated_send.take() {
                        let _ = initiated_send.send(());
                    }

                    interval.tick().await;
                }
            })
        };

        let doctor_state = Arc::new(RwLock::new(None));
        let doctor_task = {
            // Get shared ownership of the resources.
            let ready_clients = ready_clients.clone();
            let broken_endpoints = broken_endpoints.clone();
            let endpoint = self.endpoint.clone();
            let doctor_state = doctor_state.clone();

            tokio::spawn(async move {
                loop {
                    // There is an asynchronous wait inside this function so we can run it in a tight loop here.
                    let result =
                        recheck_broken_endpoint(&endpoint, &ready_clients, &broken_endpoints).await;
                    let _ = replace(&mut *doctor_state.write().await, result.err());
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
            _dns_lookup_task: Arc::new(dns_lookup_task.into()),
            _dns_lookup_state: dns_lookup_state,
            _doctor_task: Arc::new(doctor_task.into()),
        })
    }
}

async fn check_dns(
    endpoint: &EndpointTemplate,
    ready_clients: &RwLock<Vec<(IpAddr, Channel)>>,
    broken_endpoints: &BrokenEndpoints,
) -> Result<(), WrappedClientError> {
    // Resolve domain to IP addresses.
    let addresses =
        resolve_domain(endpoint.domain()).map_err(WrappedClientError::dns_resolution_error)?;

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
        if let Some(entry) = broken_endpoints.get_entry(address).await? {
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
    broken_endpoints.replace_with(broken).await?;

    Ok(())
}

async fn recheck_broken_endpoint(
    endpoint: &EndpointTemplate,
    ready_clients: &RwLock<Vec<(IpAddr, Channel)>>,
    broken_endpoints: &BrokenEndpoints,
) -> Result<(), WrappedClientError> {
    let (ip_address, backoff) = broken_endpoints.next_broken_ip_address().await?;

    let connection_test_result = endpoint.build(ip_address).connect().await;

    if let Ok(channel) = connection_test_result {
        info!("Connection established to {:?}", ip_address);
        ready_clients.write().await.push((ip_address, channel));
    } else {
        warn!("Can't connect to {:?}", ip_address);
        broken_endpoints
            .add_address_with_backoff(ip_address, backoff)
            .await?;
    }

    Ok(())
}

struct AbortOnDrop(AbortHandle);

impl<T> From<JoinHandle<T>> for AbortOnDrop {
    fn from(handle: JoinHandle<T>) -> Self {
        Self(handle.abort_handle())
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone)]
pub struct WrappedClient {
    // Note: For current random load balances, Vec a perfect data structure.
    // However, depending on other algorithms we might want to support,
    // we might want to change it to something else.
    ready_clients: Arc<RwLock<Vec<(IpAddr, Channel)>>>,
    broken_endpoints: Arc<BrokenEndpoints>,

    _dns_lookup_task: Arc<AbortOnDrop>,
    _dns_lookup_state: Arc<RwLock<Option<WrappedClientError>>>,
    _doctor_task: Arc<AbortOnDrop>,
}

impl WrappedClient {
    pub async fn get_channel(&self) -> Result<(IpAddr, Channel), WrappedClientError> {
        let read_access = self.ready_clients.read().await;
        if read_access.is_empty() {
            // If there are no healthy channels, maybe we could trigger DNS lookup from here?
            // todo- correctness: There is a chance, that due to a long backoff, there actually already exists a healthy channel,
            // but we are not aware of it. We should probably check if there are any broken endpoints and try to connect to them.
            return Err(WrappedClientError::NoReadyChannels);
        }
        // If we keep track of what channels are currently being used, we could better load balance them.
        let index = rand::rng().random_range(0..read_access.len());
        Ok(read_access[index].clone())
    }

    pub async fn report_broken(&self, ip_address: IpAddr) -> Result<(), WrappedClientError> {
        let mut write_access = self.ready_clients.write().await;
        let index = write_access.iter().position(|(e, _)| e == &ip_address);
        if let Some(index) = index {
            write_access.remove(index);
        }
        self.broken_endpoints
            .add_address(ip_address)
            .await
            .map_err(From::from)
    }
}

#[macro_export]
macro_rules! define_method {
    ($client:ident, $name:ident, $request:ty, $response:ty) => {
        paste::paste! {
            pub async fn $name(
                &self,
                request: impl tonic::IntoRequest<$request>,
            ) -> Result<tonic::Response<$response>, tonic::Status> {
                self.[<$name _with_retry>]::<$crate::DefaultRetryPolicy>(
                    request,
                ).await
            }

            pub async fn [<$name _with_retry>] <RP: $crate::RetryPolicy>(
                &self,
                request: impl tonic::IntoRequest<$request>,
            ) -> Result<tonic::Response<$response>, tonic::Status> {
                let (metadata, extensions, message) = request.into_request().into_parts();
                let mut tries = 0;
                loop {
                    tries += 1;

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
                            let $crate::RetryCheckResult(server_status, retry_time) = RP::should_retry(&e, tries);
                            if matches!(server_status, $crate::ServerStatus::Dead) {
                                // If the server is dead, we should report it.
                                self.report_broken(ip_address).await?;
                            }

                            match retry_time {
                                $crate::RetryTime::DoNotRetry => {
                                    // Do not retry and do not report broken endpoint.
                                    return Err(e);
                                }
                                $crate::RetryTime::Immediately => {
                                    // Retry immediately.
                                    continue;
                                }
                                $crate::RetryTime::After(duration) => {
                                    // Wait for the specified duration before retrying.
                                    // todo-interface: Don't require client to have tokio dependency.
                                    tokio::time::sleep(duration).await;
                                }
                            }
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
            pub async fn new(endpoint: $crate::EndpointTemplate) -> Result<Self, $crate::WrapperClientBuilderError> {
                Ok(Self {
                    base: $crate::WrappedClientBuilder::new(endpoint).build().await?,
                })
            }

            async fn get_channel(&self) -> Result<(std::net::IpAddr, tonic::transport::Channel), $crate::WrappedClientError> {
                self.base.get_channel().await
            }

            async fn report_broken(&self, ip_address: std::net::IpAddr) -> Result<(), $crate::WrappedClientError> {
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
    BrokenLock,
    DnsResolutionError(std::io::Error),
}

impl WrappedClientError {
    pub fn dns_resolution_error(e: std::io::Error) -> Self {
        WrappedClientError::DnsResolutionError(e)
    }
}

impl From<BrokenEndpointsError> for WrappedClientError {
    fn from(_: BrokenEndpointsError) -> Self {
        WrappedClientError::BrokenLock
    }
}

impl std::fmt::Display for WrappedClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WrappedClientError::NoReadyChannels => std::fmt::Display::fmt("No ready channels", f),
            WrappedClientError::BrokenLock => std::fmt::Display::fmt("Broken lock", f),
            WrappedClientError::DnsResolutionError(e) => {
                std::fmt::Display::fmt("DNS resolution error: ", f)?;
                std::fmt::Display::fmt(e, f)
            }
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
            }
            WrappedClientError::BrokenLock => {
                let mut status = Status::new(tonic::Code::Internal, "Broken lock");
                status.set_source(Arc::new(e));
                status
            }
            WrappedClientError::DnsResolutionError(e) => {
                let mut status = Status::new(tonic::Code::Unavailable, "DNS resolution error");
                status.set_source(Arc::new(e));
                status
            }
        }
    }
}
