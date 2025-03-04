use std::{
    collections::{BinaryHeap, VecDeque},
    mem::replace,
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use rand::Rng;
use tokio::{sync::RwLock, task::JoinHandle, time::interval};
use tonic::{transport::Channel, IntoRequest, Status};
use tracing::{debug, error, info, warn};

use crate::{
    broken_endpoints::BrokenEndpoints, dns::ToSocketAddrs, endpoint_template::EndpointTemplate, health::health_client::HealthClient, request_generator::GeneratesRequest
};


#[derive(Clone)]
pub struct WrappedHealthClient {
    ready_clients: Arc<RwLock<VecDeque<(IpAddr, Channel)>>>,
    broken_endpoints: Arc<BrokenEndpoints>,

    _dns_lookup_task: Arc<JoinHandle<()>>,
    _doctor_task: Arc<JoinHandle<()>>,
}

impl WrappedHealthClient {
    pub fn new(endpoint: EndpointTemplate, dns_interval: Duration) -> Self {
        let ready_clients = Arc::new(RwLock::new(VecDeque::new()));
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

                    let addresses = (endpoint.domain(), 0)
                        .to_socket_addrs()
                        .unwrap()
                        .map(|addr| addr.ip());

                    let mut ready = VecDeque::new();
                    let mut broken = BinaryHeap::new();

                    // todo: Look at the current endpoints to skip testing them again.
                    // Note: Changing implementation from Mutex to RwLock made it much nicer,
                    // but I still wonder about the performance implications of resting all endpoints each time we refresh DNS.
                    for address in addresses {
                        debug!("Connecting to: {:?}", address);
                        let endpoint = endpoint.build(address);
                        let channel = endpoint.connect().await;
                        if let Ok(channel) = channel {
                            ready.push_back((address, channel));
                        } else {
                            broken.push((
                                Instant::now() + Duration::from_secs(1),
                                address,
                            ));
                        }
                    }

                    // todo: Wrap this nicely.

                    // Replace a list of clients stored in `ready_clients`` with the new constructed in `ready`.
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
                let wait_duration = Duration::MAX;
                loop {
                    // todo: block_in_place is not the best solution here. It will prevent further tasks from being scheduled on the current thread,
                    // but may block the ones scheduled. It's ok for now for testing but should be avoided in production.
                    let Some(ip_address) = tokio::task::block_in_place(|| broken_endpoints.next_broken_ip_address(wait_duration))
                    else {
                        continue;
                    };

                    let connection_test_result = endpoint.build(ip_address).connect().await;

                    if let Ok(channel) = connection_test_result
                        && HealthClient::new(channel.clone()).is_alive(().into_request()).await.is_ok()
                    {
                        info!("Health check passed for {:?}", ip_address);
                        ready_clients.write().await.push_back((ip_address, channel));
                    } else {
                        // todo: implement exponential backoff.
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
}

impl WrappedHealthClient {
    pub async fn is_alive(
        &mut self,
        request_generator: impl GeneratesRequest<()>,
    ) -> Result<tonic::Response<crate::health::IsAliveResponse>, WrappedError> {
        loop {
            let request = request_generator.generate_request();

            let read_access = self.ready_clients.read().await;
            if read_access.is_empty() {
                return Err(WrappedError::NoReadyChannels);
            }
            let index = rand::rng().random_range(0..read_access.len());
            let (ip_address, channel) = read_access[index].clone();
            let mut client = HealthClient::new(channel);

            info!("Sending request to {:?}", ip_address);
            let result = client.is_alive(request).await;

            match result {
                Ok(response) => {
                    info!("Received response from {:?}", ip_address);
                    return Ok(response);
                }
                Err(e) => {
                    // todo: Determine if the endpoint is dead.
                    // If it is, add it to broken endpoints and retry request to another server.
                    // If it isn't, just return an error response to the caller.
                    warn!("Error from {:?}: {:?}", ip_address, e);
                    if true {
                        let mut write_access = self.ready_clients.write().await;
                        let index = write_access.iter().position(|(e, _)| e == &ip_address);
                        if let Some(index) = index {
                            write_access.remove(index);
                        }
                        self.broken_endpoints.add_address(ip_address);
                    }
                    error!("Return server error {:?} from {:?}", e, ip_address);
                    return Err(e.into());
                }
            }
        }
    }
}

#[derive(Debug)]
pub enum WrappedError {
    Status(tonic::Status),
    //Transport(tonic::transport::Error),
    //Discover(Box<dyn std::error::Error + Send + Sync>),
    //Generic(Box<dyn std::error::Error + Send + Sync>),
    NoReadyChannels,
}

impl From<Status> for WrappedError {
    fn from(e: Status) -> Self {
        WrappedError::Status(e)
    }
}

// impl From<tower::balance::error::Discover> for WrappedError {
//     fn from(e: tower::balance::error::Discover) -> Self {
//         WrappedError::Discover(e.into())
//     }
// }

// impl From<tonic::transport::Error> for WrappedError {
//     fn from(e: tonic::transport::Error) -> Self {
//         WrappedError::Transport(e)
//     }
// }

// impl From<Box<dyn std::error::Error + Send + Sync>> for WrappedError {
//     fn from(e: Box<dyn std::error::Error + Send + Sync>) -> Self {
//         WrappedError::Generic(e)
//     }
// }

// impl std::fmt::Display for WrappedError {
//     fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         todo!()
//     }
// }

// impl std::error::Error for WrappedError {
//     fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
//         match self {
//             WrappedError::Transport(e) => Some(e),
//             WrappedError::Discover(e) => e.source(),
//             WrappedError::Generic(e) => Some(&**e),
//             WrappedError::Pool => None,
//         }
//     }
// }
