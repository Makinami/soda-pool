use std::{collections::BinaryHeap, net::IpAddr, sync::Arc, time::Duration};

use chrono::{DateTime, TimeDelta, Utc};
use futures::{FutureExt, StreamExt, stream::FuturesUnordered};
use tokio::{
    sync::{RwLock, oneshot::channel},
    task::{AbortHandle, JoinHandle},
    time::interval,
};
use tonic::transport::Channel;

use tracing::{debug, trace};

use crate::{
    broken_endpoints::{BrokenEndpoints, DelayedAddress},
    dns::resolve_domain,
    endpoint_template::EndpointTemplate,
    ready_channels::ReadyChannels,
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
        let ready_clients = Arc::new(ReadyChannels::default());
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
                    check_dns(&endpoint, &ready_clients, &broken_endpoints).await;

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
            let endpoint = self.endpoint.clone();

            tokio::spawn(async move {
                loop {
                    // There is an asynchronous wait inside this function so we can run it in a tight loop here.
                    recheck_broken_endpoint(
                        broken_endpoints.next_broken_ip_address().await,
                        &endpoint,
                        &ready_clients,
                        &broken_endpoints,
                    )
                    .await;
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
            _doctor_task: Arc::new(doctor_task.into()),
        })
    }
}

async fn check_dns(
    endpoint_template: &EndpointTemplate,
    ready_clients: &ReadyChannels,
    broken_endpoints: &BrokenEndpoints,
) {
    // Resolve domain to IP addresses.
    let Ok(addresses) = resolve_domain(endpoint_template.domain()) else {
        todo!("This should never happen");
    };

    let mut ready = Vec::new();
    let mut broken = BinaryHeap::new();

    for address in addresses {
        // Skip if the address is already in ready_clients.
        if let Some(channel) = ready_clients.find(address).await {
            trace!("Skipping {:?} as already ready", address);
            ready.push((address, channel));
            continue;
        }

        // Skip if the address is already in broken_endpoints.
        if let Some(entry) = broken_endpoints.get_address(address).await {
            trace!("Skipping {:?} as already broken", address);
            broken.push(entry);
            continue;
        }

        debug!("Connecting to: {:?}", address);
        let channel = endpoint_template.build(address).connect().await;
        if let Ok(channel) = channel {
            ready.push((address, channel));
        } else {
            broken.push(address.into());
        }
    }

    // Replace a list of clients stored in `ready_clients`` with the new ones constructed in `ready`.
    ready_clients.replace_with(ready).await;
    broken_endpoints.replace_with(broken).await;
}

async fn recheck_broken_endpoint(
    address: DelayedAddress,
    endpoint: &EndpointTemplate,
    ready_clients: &ReadyChannels,
    broken_endpoints: &BrokenEndpoints,
) {
    let connection_test_result = endpoint.build(*address).connect().await;

    if let Ok(channel) = connection_test_result {
        debug!("Connection established to {:?}", *address);
        ready_clients.add(*address, channel).await;
    } else {
        debug!("Can't connect to {:?}", *address);
        broken_endpoints.readd_address(address).await;
    }
}

#[derive(Debug, Default)]
struct AbortOnDrop(Option<AbortHandle>);

impl<T> From<JoinHandle<T>> for AbortOnDrop {
    fn from(handle: JoinHandle<T>) -> Self {
        Self(Some(handle.abort_handle()))
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

// todo-performance: Need to change to INNER pattern to avoid cloning multiple Arcs.
#[derive(Clone, Debug)]
pub struct WrappedClient {
    template: Arc<EndpointTemplate>,
    ready_clients: Arc<ReadyChannels>,
    broken_endpoints: Arc<BrokenEndpoints>,

    _dns_lookup_task: Arc<AbortOnDrop>,
    _doctor_task: Arc<AbortOnDrop>,
}

impl WrappedClient {
    pub async fn get_channel(&self) -> Option<(IpAddr, Channel)> {
        static RECHECK_BROKEN_ENDPOINTS: RwLock<DateTime<Utc>> =
            RwLock::const_new(DateTime::<Utc>::MIN_UTC);
        const MIN_INTERVAL: TimeDelta = TimeDelta::milliseconds(500);

        if let Some(entry) = self.ready_clients.get_any().await {
            return Some(entry);
        }

        // todo: This entire function is a bit of a mess, but this part absolutely needs to be cleaned up.
        let _guard = match RECHECK_BROKEN_ENDPOINTS.try_read() {
            Ok(last_recheck_time)
                if Utc::now().signed_duration_since(*last_recheck_time) < MIN_INTERVAL =>
            {
                return None;
            }
            Ok(guard) => {
                drop(guard);
                let mut guard = RECHECK_BROKEN_ENDPOINTS.write().await;
                if let Some(entry) = self.ready_clients.get_any().await {
                    return Some(entry);
                }
                *guard = Utc::now();
                guard
            }
            Err(_) => {
                let _ = RECHECK_BROKEN_ENDPOINTS.write().await;
                return self.ready_clients.get_any().await;
            }
        };

        trace!("Force recheck of broken endpoints");

        let mut fut = FuturesUnordered::new();
        fut.push(
            async {
                check_dns(&self.template, &self.ready_clients, &self.broken_endpoints).await;
                self.ready_clients.get_any().await
            }
            .boxed(),
        );

        for address in self.broken_endpoints.addresses().await.iter().copied() {
            fut.push(
                async move {
                    recheck_broken_endpoint(
                        address,
                        &self.template,
                        &self.ready_clients,
                        &self.broken_endpoints,
                    )
                    .await;
                    self.ready_clients.get_any().await
                }
                .boxed(),
            );
        }

        fut.select_next_some().await
    }

    pub async fn report_broken(&self, ip_address: IpAddr) {
        self.ready_clients.remove(ip_address).await;
        self.broken_endpoints.add_address(ip_address).await;
    }
}
