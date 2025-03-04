use std::{
    collections::{BinaryHeap, VecDeque}, mem::replace, sync::Arc, time::{Duration, Instant}
};

use tokio::{task::JoinHandle, time::interval};
use tonic::{
    transport::{Channel, Endpoint},
    IntoRequest,
};
use tracing::{debug, info, warn};

use crate::{
    endpoint_template::EndpointTemplate,
    health::health_client::HealthClient, request_generator::GeneratesRequest,
    dns::ToSocketAddrs,
};

#[derive(Clone)]
pub struct WrappedHealthClient {
    pub ready_clients: Arc<std::sync::Mutex<VecDeque<(Endpoint, HealthClient<Channel>)>>>,
    pub ready_clients_condvar: Arc<std::sync::Condvar>,

    pub broken_endpoints: Arc<std::sync::Mutex<BinaryHeap<InstantEndpoint>>>,
    pub broken_endpoints_condvar: Arc<std::sync::Condvar>,

    pub dns_lookup_task: Arc<JoinHandle<()>>,
    pub doctor_task: Arc<JoinHandle<()>>,
}

impl WrappedHealthClient {
    pub fn new(endpoint: EndpointTemplate) -> Self {
        let ready_clients = Arc::new(std::sync::Mutex::new(VecDeque::new()));
        let ready_clients_condvar = Arc::new(std::sync::Condvar::new());

        let broken_endpoints = Arc::new(std::sync::Mutex::new(BinaryHeap::new()));
        let broken_endpoints_condvar = Arc::new(std::sync::Condvar::new());

        let dns_lookup_task = {
            let ready_clients = ready_clients.clone();
            let ready_clients_condvar = ready_clients_condvar.clone();

            let broken_endpoints = broken_endpoints.clone();
            let broken_endpoints_condvar = broken_endpoints_condvar.clone();

            tokio::spawn(async move {
                let mut interval = interval(Duration::from_secs(1));
                loop {
                    interval.tick().await;

                    let addresses = (endpoint.domain(), 0)
                        .to_socket_addrs()
                        .unwrap()
                        .map(|addr| addr.ip());

                    let mut ready = VecDeque::new();
                    let mut broken = BinaryHeap::new();


                    for address in addresses {
                        debug!("Connecting to: {:?}", address);
                        let endpoint = endpoint.build(address);
                        let client = HealthClient::connect(endpoint.clone()).await;
                        match client {
                            Ok(client) => {
                                ready.push_back((endpoint, client));
                            }
                            Err(_e) => {
                                broken.push(InstantEndpoint(
                                    Instant::now() + Duration::from_secs(1),
                                    endpoint,
                                ));
                            }
                        }
                    }

                    // todo: Wrap this nicely.
                    match ready_clients.lock() {
                        Ok(mut guard) => {
                            let healthy = !ready.is_empty();
                            let _ = replace(&mut *guard, ready);
                            if healthy {
                                ready_clients_condvar.notify_one();
                            }
                        }
                        Err(_) => todo!(),
                    };
                    match broken_endpoints.lock() {
                        Ok(mut guard) => {
                            let is_broken = !broken.is_empty();
                            let _ = replace(&mut *guard, broken);
                            if is_broken {
                                broken_endpoints_condvar.notify_one();
                            }
                        }
                        Err(_) => todo!(),
                    };
                }
            })
        };

        let doctor_task = {
            let ready_clients = ready_clients.clone();
            let ready_clients_condvar = ready_clients_condvar.clone();

            let broken_endpoints = broken_endpoints.clone();
            let broken_endpoints_condvar = broken_endpoints_condvar.clone();

            tokio::spawn(async move {
                let mut wait_duration = Duration::MAX;
                loop {
                    let endpoint = {
                        let mut result = broken_endpoints_condvar
                            .wait_timeout_while(
                                broken_endpoints.lock().unwrap(),
                                wait_duration,
                                |endpoints| endpoints.is_empty(),
                            )
                            .unwrap();

                        #[allow(unused_variables)]
                        match result.0.peek() {
                            Some(InstantEndpoint(instant, _)) => {
                                let now = Instant::now();
                                if now < *instant {
                                    wait_duration = *instant - now;
                                    continue;
                                } else {
                                    result.0.pop().unwrap().1
                                }
                            }
                            None => {
                                wait_duration = Duration::MAX;
                                continue;
                            }
                        }
                    };

                    let result = HealthClient::connect(endpoint.clone()).await;
                    match result {
                        Ok(mut client) => {
                            match client.is_alive(().into_request()).await {
                                Ok(_) => {
                                    info!("Health check passed for {:?}", endpoint);
                                    ready_clients
                                        .lock()
                                        .unwrap()
                                        .push_back((endpoint, client));
                                    ready_clients_condvar.notify_one();
                                },
                                Err(_) => {
                                    warn!("Failed health check for {:?}", endpoint);
                                    broken_endpoints
                                        .lock()
                                        .unwrap()
                                        .push(InstantEndpoint(
                                            Instant::now() + Duration::from_secs(1),
                                            endpoint,
                                        ));
                                    broken_endpoints_condvar.notify_one();},
                            }

                        }
                        Err(_e) => {
                            warn!("Can't connect to {:?}", endpoint);
                            broken_endpoints
                                .lock()
                                .unwrap()
                                .push(InstantEndpoint(
                                    Instant::now() + Duration::from_secs(1),
                                    endpoint,
                                ));
                            broken_endpoints_condvar.notify_one();
                        }
                    }
                }
            })
        };

        Self {
            ready_clients,
            ready_clients_condvar,
            broken_endpoints,
            broken_endpoints_condvar,
            dns_lookup_task: Arc::new(dns_lookup_task),
            doctor_task: Arc::new(doctor_task),
        }
    }
}

pub struct InstantEndpoint(Instant, Endpoint);

impl Eq for InstantEndpoint {}

impl PartialEq for InstantEndpoint {
    fn eq(&self, other: &Self) -> bool {
        // todo: Double check this. Maybe not for binary heap, but I should
        // probably compare also the endpoint.
        PartialEq::eq(&self.0, &other.0)
    }
}

impl PartialOrd for InstantEndpoint {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        PartialOrd::partial_cmp(&self.0, &other.0).map(|o| o.reverse())
    }
}

impl Ord for InstantEndpoint {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        Ord::cmp(&self.0, &other.0)
    }
}

#[allow(dead_code)]
async fn wtf(wrapper: &mut WrappedHealthClient) {
    let mut wait_duration = Duration::MAX;
    loop {
        let mut result = wrapper
            .broken_endpoints_condvar
            .wait_timeout_while(
                wrapper.broken_endpoints.lock().unwrap(),
                wait_duration,
                |endpoints| endpoints.is_empty(),
            )
            .unwrap();

        let endpoint = match result.0.peek() {
            Some(InstantEndpoint(instant, _)) => {
                let now = Instant::now();
                if now < *instant {
                    wait_duration = *instant - now;
                    continue;
                } else {
                    result.0.pop().unwrap().1
                }
            }
            None => {
                wait_duration = Duration::MAX;
                continue;
            }
        };

        let result = HealthClient::connect(endpoint.clone()).await;
        match result {
            Ok(client) => {
                wrapper
                    .ready_clients
                    .lock()
                    .unwrap()
                    .push_back((endpoint, client));
                wrapper.ready_clients_condvar.notify_one();
            }
            Err(_e) => {
                wrapper
                    .broken_endpoints
                    .lock()
                    .unwrap()
                    .push(InstantEndpoint(
                        Instant::now() + Duration::from_secs(1),
                        endpoint,
                    ));
                wrapper.broken_endpoints_condvar.notify_one();
            }
        }
    }
}

impl WrappedHealthClient {
    pub async fn is_alive(
        &mut self,
        request_generator: impl GeneratesRequest<()>,
    ) -> Result<tonic::Response<crate::health::IsAliveResponse>, tonic::Status> {
        loop {
            let request = request_generator.generate_request();

            let (endpoint, mut client) = self
                .ready_clients_condvar
                .wait_while(self.ready_clients.lock().unwrap(), |clients| {
                    clients.is_empty()
                })
                .unwrap()
                .pop_front()
                .unwrap();

            info!("Sending request to {:?}", endpoint);
            let result = client.is_alive(request).await;

            match result {
                Ok(response) => {
                    info!("Received response from {:?}, {:?}", endpoint, response);
                    self.ready_clients
                        .lock()
                        .unwrap()
                        .push_back((endpoint, client));
                    self.ready_clients_condvar.notify_one();
                    return Ok(response);
                }
                Err(e) => {
                    // todo: Determine if the endpoint is dead.
                    warn!("Error from {:?}: {:?}", endpoint, e);
                    if true {
                        self.broken_endpoints.lock().unwrap().push(InstantEndpoint(
                            Instant::now() + Duration::from_secs(1),
                            endpoint,
                        ));
                        self.broken_endpoints_condvar.notify_one();
                    }
                    return Err(e);
                }
            }
        }
    }
}

#[derive(Debug)]
enum WrappedError {
    Transport(tonic::transport::Error),
    Discover(Box<dyn std::error::Error + Send + Sync>),
    Generic(Box<dyn std::error::Error + Send + Sync>),
    #[allow(dead_code)]
    Pool,
}

impl From<tower::balance::error::Discover> for WrappedError {
    fn from(e: tower::balance::error::Discover) -> Self {
        WrappedError::Discover(e.into())
    }
}

impl From<tonic::transport::Error> for WrappedError {
    fn from(e: tonic::transport::Error) -> Self {
        WrappedError::Transport(e)
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for WrappedError {
    fn from(e: Box<dyn std::error::Error + Send + Sync>) -> Self {
        WrappedError::Generic(e)
    }
}

impl std::fmt::Display for WrappedError {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl std::error::Error for WrappedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WrappedError::Transport(e) => Some(e),
            WrappedError::Discover(e) => e.source(),
            WrappedError::Generic(e) => Some(&**e),
            WrappedError::Pool => None,
        }
    }
}
