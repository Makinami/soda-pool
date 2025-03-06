use std::{
    collections::BinaryHeap,
    marker::PhantomData,
    mem::replace,
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use rand::Rng;
use tokio::{sync::RwLock, task::JoinHandle, time::interval};
use tonic::{transport::Channel, Status};
use tracing::{debug, error, info, warn};

use crate::{
    broken_endpoints::BrokenEndpoints, dns::ToSocketAddrs, endpoint_template::EndpointTemplate,
    health::health_client::HealthClient, request_generator::GeneratesRequest,
};

#[derive(Clone)]
pub struct WrappedHealthClient<Doc: Doctor = BasicDoctor> {
    // todo-performance: Consider using another data structure for ready_clients.
    // Vec was a first default choice because it's simple and easy to work with,
    // but I haven't thought about other options yet.
    ready_clients: Arc<RwLock<Vec<(IpAddr, Channel)>>>,
    broken_endpoints: Arc<BrokenEndpoints>,

    _dns_lookup_task: Arc<JoinHandle<()>>,
    _doctor_task: Arc<JoinHandle<()>>,

    _doctor: PhantomData<Arc<Doc>>,
}

pub trait Doctor {
    fn is_channel_alive(channel: &Channel) -> bool;
    // todo-interface: Maybe rename this method. Also, consider splitting Doctor into two separate traits. Or use Fn trait...
    fn is_alive_by_response(response: &Status) -> bool;
}

#[derive(Clone)]
pub struct BasicDoctor {}

impl Doctor for BasicDoctor {
    fn is_channel_alive(_channel: &Channel) -> bool {
        true
    }

    fn is_alive_by_response(_response: &Status) -> bool {
        // todo-correctness: check if source of the error is the transport layer.
        // E.g.:
        // Status {
        //     code: Unavailable,
        //     message: "error trying to connect: tcp connect error: Connection refused (os error 61)",
        //     source: Some(tonic::transport::Error(Transport, hyper::Error(Connect, ConnectError("tcp connect error", Os { code: 61, kind: ConnectionRefused, message: "Connection refused" }))))
        // }
        false
    }
}

impl<Doc: Doctor> WrappedHealthClient<Doc> {
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

                    // todo-performance: Look at the current endpoints to skip testing them again.
                    // Note: Changing implementation from Mutex to RwLock made it much nicer,
                    // but I still wonder about the performance implications of resting all endpoints each time we refresh DNS.
                    for address in addresses {
                        debug!("Connecting to: {:?}", address);
                        let endpoint = endpoint.build(address);
                        let channel = endpoint.connect().await;
                        if let Ok(channel) = channel
                            && Doc::is_channel_alive(&channel)
                        {
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
                let wait_duration = Duration::MAX;
                loop {
                    // todo-performance: block_in_place is not the best solution here. It will prevent further tasks from being scheduled on the current thread,
                    // but may block the ones already scheduled. It's ok for now for testing but should be avoided in production.
                    let Some(ip_address) = tokio::task::block_in_place(|| {
                        broken_endpoints.next_broken_ip_address(wait_duration)
                    }) else {
                        continue;
                    };

                    let connection_test_result = endpoint.build(ip_address).connect().await;

                    if let Ok(channel) = connection_test_result
                        && Doc::is_channel_alive(&channel)
                    {
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
            _doctor: PhantomData,
        }
    }
}

macro_rules! define_method {
    ($client:ident, $name:ident, $request:ty, $response:ty) => {
        impl<Doc: Doctor> WrappedHealthClient<Doc> {
            pub async fn $name(
                &mut self,
                request_generator: impl GeneratesRequest<$request>,
            ) -> Result<tonic::Response<$response>, WrappedStatus> {
                loop {
                    // Get channel of random index.
                    let (ip_address, channel) = {
                        let read_access = self.ready_clients.read().await;
                        if read_access.is_empty() {
                            // If there are no healthy channels, maybe we could trigger DNS lookup from here?
                            return Err(WrappedStatus::NoReadyChannels);
                        }
                        // If we keep track of what channels are currently being used, we could better load balance them.
                        let index = rand::rng().random_range(0..read_access.len());
                        read_access[index].clone()
                    };

                    info!("Sending request to {:?}", ip_address);
                    let request = request_generator.generate_request();
                    let result = $client::new(channel).$name(request).await;

                    match result {
                        Ok(response) => {
                            info!("Received response from {:?}", ip_address);
                            return Ok(response);
                        }
                        Err(e) => {
                            // If the error happened because the channel is dead (e.g. connection refused),
                            // add the address to broken endpoints and retry request thought another channel.
                            warn!("Error from {:?}: {:?}", ip_address, e);
                            if !Doc::is_alive_by_response(&e) {
                                let mut write_access = self.ready_clients.write().await;
                                let index = write_access.iter().position(|(e, _)| e == &ip_address);
                                if let Some(index) = index {
                                    write_access.remove(index);
                                }
                                self.broken_endpoints.add_address(ip_address);
                            } else {
                                // Otherwise, return the error to the caller.
                                error!("Return server error {:?} from {:?}", e, ip_address);
                                return Err(e.into());
                            }
                        }
                    }
                }
            }
        }
    };
}

define_method!(HealthClient, is_alive, (), crate::health::IsAliveResponse);

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
