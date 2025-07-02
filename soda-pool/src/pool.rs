use std::{collections::BinaryHeap, net::IpAddr, sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use futures::{stream::FuturesUnordered, FutureExt, StreamExt};
use tokio::{
    sync::RwLock,
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

/// Builder for creating a [`ChannelPool`].
#[derive(Debug, Clone)]
pub struct ManagedChannelPoolBuilder {
    endpoint: EndpointTemplate,
    dns_interval: Duration,
}

impl ManagedChannelPoolBuilder {
    /// Create a new `ChannelPoolBuilder` from the given endpoint template.
    #[must_use]
    pub fn new(endpoint: impl Into<EndpointTemplate>) -> Self {
        Self {
            endpoint: endpoint.into(),
            // Note: Is this a good default?
            dns_interval: Duration::from_secs(5),
        }
    }

    /// Set the DNS check interval.
    ///
    /// Set how often the resulting pool will check the DNS for new IP
    /// addresses. Default is 5 seconds.
    #[must_use]
    pub fn dns_interval(&mut self, dns_interval: impl Into<Duration>) -> &mut Self {
        self.dns_interval = dns_interval.into();
        self
    }

    /// Build the [`ChannelPool`].
    ///
    /// This function will create a new channel pool from the given endpoint
    /// template and settings. This includes starting channel pool's background
    /// tasks.
    #[must_use]
    pub fn build(self) -> ManagedChannelPool {
        let ready_clients = Arc::new(ReadyChannels::default());
        let broken_endpoints = Arc::new(BrokenEndpoints::default());

        let dns_lookup_task = {
            // Get shared ownership of the resources.
            let ready_clients = ready_clients.clone();
            let broken_endpoints = broken_endpoints.clone();
            let endpoint = self.endpoint.clone();

            tokio::spawn(async move {
                let mut interval = interval(self.dns_interval);
                loop {
                    check_dns(&endpoint, &ready_clients, &broken_endpoints).await;

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

        ManagedChannelPool {
            template: Arc::new(self.endpoint),
            ready_clients,
            broken_endpoints,
            _dns_lookup_task: Arc::new(dns_lookup_task.into()),
            _doctor_task: Arc::new(doctor_task.into()),
        }
    }
}

async fn check_dns(
    endpoint_template: &EndpointTemplate,
    ready_clients: &ReadyChannels,
    broken_endpoints: &BrokenEndpoints,
) {
    // Resolve domain to IP addresses.
    let Ok(addresses) = resolve_domain(endpoint_template.domain()) else {
        // todo-interface: DNS resolution would mainly fail if domain does not
        // resolve to any IP address, but it could also fail for other reasons.
        // In the future version, we should record this error and allow user to
        // see it.
        return;
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
        broken_endpoints.re_add_address(address).await;
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

/// Trait implemented by channel pools.
///
/// Note: The trait definition is using [`mod@async_trait`] so it is possible
/// (or even suggested) for implementations to do the same.
#[async_trait]
pub trait ChannelPool {
    /// Get a channel from the pool.
    ///
    /// Returns a channel if one is available, or `None` if no channels are available.
    async fn get_channel(&self) -> Option<(IpAddr, Channel)>;

    /// Report a broken endpoint to the pool.
    ///
    /// Removes the endpoint from the pool and add it to the list of currently dead servers.
    async fn report_broken(&self, ip_address: IpAddr);
}

/// Self-managed pool of tonic's [`Channel`]s.
// todo-performance: Probably better to change to INNER pattern to avoid cloning multiple Arcs.
#[derive(Debug)]
pub struct ManagedChannelPool {
    template: Arc<EndpointTemplate>,
    ready_clients: Arc<ReadyChannels>,
    broken_endpoints: Arc<BrokenEndpoints>,

    _dns_lookup_task: Arc<AbortOnDrop>,
    _doctor_task: Arc<AbortOnDrop>,
}

#[async_trait]
impl ChannelPool for ManagedChannelPool {
    /// Get a channel from the pool.
    ///
    /// This function will return a channel if one is available, or `None` if no
    /// channels are available.
    ///
    /// ## Selection algorithm
    ///
    /// Currently, the channel is selected randomly from the pool of available
    /// channels. However, this behavior may change in the future.
    ///
    /// ## Additional DNS and broken connection checks
    ///
    /// If no channels are available, the function will check the DNS and recheck connections to all
    /// servers currently marked as dead. To avoid spamming the DNS and other
    /// servers, this will be performed no more than once every 500ms.
    ///
    /// If the above check is running while this function is called, the function
    /// will wait for the check to finish and return the result.
    ///
    /// If the check is not running, but the last check was performed less than 500ms ago,
    /// the function will return `None` immediately.
    ///
    /// The specifics of this behavior are not set in stone and may change in the future.
    async fn get_channel(&self) -> Option<(IpAddr, Channel)> {
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
                // RECHECK_BROKEN_ENDPOINTS used here to wait until ready channels and broken endpoints are checked.
                // Thus, there is no need to hold the lock after acquiring it.
                // (Some other implementation might be worth considering, but this is a good start.)
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

    /// Report a broken endpoint to the pool.
    ///
    /// This function will remove the endpoint from the pool and add it to the list of currently dead servers.
    async fn report_broken(&self, ip_address: IpAddr) {
        self.ready_clients.remove(ip_address).await;
        self.broken_endpoints.add_address(ip_address).await;
    }
}

/// This is a shallow clone, meaning that the new pool will reference the same
/// resources as the original pool.
impl Clone for ManagedChannelPool {
    fn clone(&self) -> Self {
        #[allow(clippy::used_underscore_binding)]
        Self {
            template: self.template.clone(),
            ready_clients: self.ready_clients.clone(),
            broken_endpoints: self.broken_endpoints.clone(),
            _dns_lookup_task: self._dns_lookup_task.clone(),
            _doctor_task: self._doctor_task.clone(),
        }
    }
}
