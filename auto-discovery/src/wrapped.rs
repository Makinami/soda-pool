use std::{collections::BinaryHeap, mem::replace, net::IpAddr, sync::Arc, time::Duration};

use chrono::{DateTime, TimeDelta, Utc};
use futures::{FutureExt, StreamExt, stream::FuturesUnordered};
use rand::Rng;
use tokio::{
    sync::{RwLock, oneshot::channel},
    task::{AbortHandle, JoinHandle},
    time::interval,
};
use tonic::transport::Channel;

use tracing::{debug, trace};

use crate::{
    broken_endpoints::{BackoffTracker, BrokenEndpoints},
    dns::resolve_domain,
    endpoint_template::EndpointTemplate,
    error::WrappedClientError,
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

    #[must_use]
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
                    let result = async {
                        let (ip_address, backoff) =
                            broken_endpoints.next_broken_ip_address().await?;
                        recheck_broken_endpoint(
                            ip_address,
                            backoff,
                            &endpoint,
                            &ready_clients,
                            &broken_endpoints,
                        )
                        .await
                    }
                    .await;
                    let _ = replace(&mut *doctor_state.write().await, result.err());
                }
            })
        };

        match initiated_recv.await {
            Ok(()) => {}
            Err(_) => {
                return Err(WrapperClientBuilderError::FailedToInitiate);
            }
        }

        Ok(WrappedClient {
            template: Arc::new(self.endpoint),
            ready_clients,
            broken_endpoints,
            _dns_lookup_task: Arc::new(dns_lookup_task.into()),
            _dns_lookup_state: dns_lookup_state,
            _doctor_task: Arc::new(doctor_task.into()),
            _doctor_state: doctor_state,
        })
    }
}

async fn check_dns(
    endpoint_template: &EndpointTemplate,
    ready_clients: &RwLock<Vec<(IpAddr, Channel)>>,
    broken_endpoints: &BrokenEndpoints,
) -> Result<(), WrappedClientError> {
    // Resolve domain to IP addresses.
    let addresses = resolve_domain(endpoint_template.domain())
        .map_err(WrappedClientError::dns_resolution_error)?;

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
        let channel = endpoint_template.build(address).connect().await;
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
    ip_address: IpAddr,
    backoff: BackoffTracker,
    endpoint: &EndpointTemplate,
    ready_clients: &RwLock<Vec<(IpAddr, Channel)>>,
    broken_endpoints: &BrokenEndpoints,
) -> Result<(), WrappedClientError> {
    let connection_test_result = endpoint.build(ip_address).connect().await;

    if let Ok(channel) = connection_test_result {
        debug!("Connection established to {:?}", ip_address);
        ready_clients.write().await.push((ip_address, channel));
    } else {
        debug!("Can't connect to {:?}", ip_address);
        broken_endpoints
            .add_address_with_backoff(ip_address, backoff)
            .await?;
    }

    Ok(())
}

#[derive(Debug)]
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

// todo-performance: Need to change to INNER pattern to avoid cloning multiple Arcs.
#[derive(Clone, Debug)]
pub struct WrappedClient {
    template: Arc<EndpointTemplate>,
    // Note: For current random load balances, Vec a perfect data structure.
    // However, depending on other algorithms we might want to support,
    // we might want to change it to something else.
    ready_clients: Arc<RwLock<Vec<(IpAddr, Channel)>>>,
    broken_endpoints: Arc<BrokenEndpoints>,

    _dns_lookup_task: Arc<AbortOnDrop>,
    _dns_lookup_state: Arc<RwLock<Option<WrappedClientError>>>,
    _doctor_task: Arc<AbortOnDrop>,
    _doctor_state: Arc<RwLock<Option<WrappedClientError>>>,
}

impl WrappedClient {
    pub async fn get_channel(&self) -> Result<(IpAddr, Channel), WrappedClientError> {
        static RECHECK_BROKEN_ENDPOINTS: RwLock<DateTime<Utc>> =
            RwLock::const_new(DateTime::<Utc>::MIN_UTC);
        const MIN_INTERVAL: TimeDelta = TimeDelta::milliseconds(500);

        if let Some(entry) = self.get_channel_inner().await {
            return Ok(entry);
        }

        // todo: This entire function is a bit of a mess, but this part absolutely needs to be cleaned up.
        let _guard = match RECHECK_BROKEN_ENDPOINTS.try_read() {
            Ok(last_recheck_time)
                if Utc::now().signed_duration_since(*last_recheck_time) < MIN_INTERVAL =>
            {
                return Err(WrappedClientError::NoReadyChannels);
            }
            Ok(guard) => {
                drop(guard);
                let mut guard = RECHECK_BROKEN_ENDPOINTS.write().await;
                if let Some(entry) = self.get_channel_inner().await {
                    return Ok(entry);
                }
                *guard = Utc::now();
                guard
            }
            Err(_) => {
                let _ = RECHECK_BROKEN_ENDPOINTS.write().await;
                return self
                    .get_channel_inner()
                    .await
                    .ok_or(WrappedClientError::NoReadyChannels);
            }
        };

        trace!("Force recheck of broken endpoints");

        let mut fut = FuturesUnordered::new();
        fut.push(
            async {
                check_dns(&self.template, &self.ready_clients, &self.broken_endpoints)
                    .await
                    .ok()?;
                self.get_channel_inner().await
            }
            .boxed(),
        );

        for (backoff, ip_address) in self.broken_endpoints.addresses().await.iter().copied() {
            fut.push(
                async move {
                    recheck_broken_endpoint(
                        ip_address,
                        backoff,
                        &self.template,
                        &self.ready_clients,
                        &self.broken_endpoints,
                    )
                    .await
                    .ok()?;
                    self.get_channel_inner().await
                }
                .boxed(),
            );
        }

        fut.select_next_some()
            .await
            .ok_or(WrappedClientError::NoReadyChannels)
    }

    async fn get_channel_inner(&self) -> Option<(IpAddr, Channel)> {
        let read_access = self.ready_clients.read().await;
        if read_access.is_empty() {
            return None;
        }
        // If we keep track of what channels are currently being used, we could better load balance them.
        let index = rand::rng().random_range(0..read_access.len());
        Some(read_access[index].clone())
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
