use std::{
    collections::{BinaryHeap, VecDeque}, mem::replace, sync::Arc, time::{Duration, Instant}
};

use tokio::{task::JoinHandle, time::interval};
use tonic::{
    metadata::MetadataMap,
    transport::{Channel, Endpoint},
    Extensions, IntoRequest,
};
use tracing::{debug, trace, warn};

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
    pub fn new(_endpoint: EndpointTemplate) -> Self {
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

                    // let addresses = (endpoint.domain(), 0)
                    //     .to_socket_addrs()
                    //     .unwrap()
                    //     .map(|addr| addr.ip());

                    let mut ready = VecDeque::new();
                    let mut broken = BinaryHeap::new();

                    for port in 50000..50005 {
                        let endpoint  = Endpoint::new(format!("http://localhost:{}", port)).unwrap();
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

                    // for address in addresses {
                    //     let endpoint = endpoint.build(address);
                    //     let client = HealthClient::connect(endpoint.clone()).await;
                    //     match client {
                    //         Ok(client) => {
                    //             ready.push_back((endpoint, client));
                    //         }
                    //         Err(_e) => {
                    //             broken.push(InstantEndpoint(
                    //                 Instant::now() + Duration::from_secs(1),
                    //                 endpoint,
                    //             ));
                    //         }
                    //     }
                    // }

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
                                    debug!("Health check passed for {:?}", endpoint);
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

            trace!("Sending request to {:?}", endpoint);
            let result = client.is_alive(request).await;

            match result {
                Ok(response) => {
                    trace!("Received response from {:?}", endpoint);
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

// struct BalancedGrpcService {
//     discover: EndpointToChannelStream<IpAddr>,
//     live_services:
//         tower::ready_cache::ReadyCache<IpAddr, Channel, http::Request<tonic::body::BoxBody>>,
//     ready_index: Option<usize>,
//     rng: rand::rngs::SmallRng,

//     #[allow(dead_code)]
//     dead_addresses: std::collections::BinaryHeap<(std::cmp::Reverse<std::time::Instant>, IpAddr)>,
// }

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

// impl BalancedGrpcService {
//     /// Polls `discover` for updates, adding new items to `not_ready`.
//     ///
//     /// Removals may alter the order of either `ready` or `not_ready`.
//     fn update_pending_from_discover(
//         &mut self,
//         cx: &mut Context<'_>,
//     ) -> Poll<Option<Result<(), WrappedError>>> {
//         debug!("updating from discover");
//         loop {
//             match ready!(Pin::new(&mut self.discover).poll_discover(cx))
//                 .transpose()
//                 .map_err(|e| WrappedError::Discover(e.into()))?
//             {
//                 None => return Poll::Ready(None),
//                 Some(Change::Remove(key)) => {
//                     trace!("remove");
//                     self.live_services.evict(&key);
//                 }
//                 Some(Change::Insert(key, svc)) => {
//                     trace!("insert");
//                     // If this service already existed in the set, it will be
//                     // replaced as the new one becomes ready.
//                     self.live_services.push(key, svc);
//                 }
//             }
//         }
//     }

//     fn promote_pending_to_ready(&mut self, cx: &mut Context<'_>) {
//         loop {
//             match self.live_services.poll_pending(cx) {
//                 Poll::Ready(Ok(())) => {
//                     // There are no remaining pending services.
//                     debug_assert_eq!(self.live_services.pending_len(), 0);
//                     break;
//                 }
//                 Poll::Pending => {
//                     // None of the pending services are ready.
//                     debug_assert!(self.live_services.pending_len() > 0);
//                     break;
//                 }
//                 Poll::Ready(Err(error)) => {
//                     // An individual service was lost; continue processing
//                     // pending services.
//                     debug!(%error, "dropping failed endpoint");
//                 }
//             }
//         }
//         trace!(
//             ready = %self.live_services.ready_len(),
//             pending = %self.live_services.pending_len(),
//             "poll_unready"
//         );
//     }

//     /// Performs P2C on inner services to find a suitable endpoint.
//     fn ready_index(&mut self) -> Option<usize> {
//         match self.live_services.ready_len() {
//             0 => None,
//             1 => Some(0),
//             len => Some((0..len).sample_single(&mut self.rng)),
//         }
//     }

//     #[allow(dead_code)]
//     fn mark_failed(&self, _key: IpAddr) {}
// }

// impl tower_service::Service<http::Request<tonic::body::BoxBody>> for BalancedGrpcService {
//     type Response = http::Response<hyper::body::Body>;
//     type Error = WrappedError;
//     type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;
//     // Box<Pin<
//     //     <Channel as tower_service::Service<http::Request<tonic::body::BoxBody>>>::Future,
//     // >>;

//     // From tonic v0.11.0: src/balance/p2c/service.rs
//     fn poll_ready(
//         &mut self,
//         cx: &mut std::task::Context<'_>,
//     ) -> std::task::Poll<Result<(), Self::Error>> {
//         // `ready_index` may have already been set by a prior invocation. These
//         // updates cannot disturb the order of existing ready services.
//         let _ = self.update_pending_from_discover(cx)?;
//         self.promote_pending_to_ready(cx);

//         loop {
//             // If a service has already been selected, ensure that it is ready.
//             // This ensures that the underlying service is ready immediately
//             // before a request is dispatched to it (i.e. in the same task
//             // invocation). If, e.g., a failure detector has changed the state
//             // of the service, it may be evicted from the ready set so that
//             // another service can be selected.
//             if let Some(index) = self.ready_index.take() {
//                 match self.live_services.check_ready_index(cx, index) {
//                     Ok(true) => {
//                         // The service remains ready.
//                         self.ready_index = Some(index);
//                         return Poll::Ready(Ok(()));
//                     }
//                     Ok(false) => {
//                         // The service is no longer ready. Try to find a new one.
//                         trace!("ready service became unavailable");
//                     }
//                     Err(Failed(_, error)) => {
//                         // The ready endpoint failed, so log the error and try
//                         // to find a new one.
//                         debug!(%error, "endpoint failed");
//                     }
//                 }
//             }

//             // Select a new service by comparing two at random and using the
//             // lesser-loaded service.
//             self.ready_index = self.ready_index();
//             if self.ready_index.is_none() {
//                 debug_assert_eq!(self.live_services.ready_len(), 0);
//                 // We have previously registered interest in updates from
//                 // discover and pending services.
//                 return Poll::Pending;
//             }
//         }
//     }

//     fn call(&mut self, request: http::Request<tonic::body::BoxBody>) -> Self::Future {
//         let index = self.ready_index.take().expect("called before ready");
//         let _key = self
//             .live_services
//             .get_ready_index(index)
//             .expect("index out of bounds")
//             .0
//             .clone();

//         let fut = self.live_services.call_ready_index(index, request);

//         Box::pin(async move {
//             let result = fut.await;
//             match result {
//                 Ok(response) => Ok(response),
//                 Err(error) => {
//                     // todo: Mark the ip for eviction.
//                     // How the hell do I do that without global variables?
//                     Err(WrappedError::Transport(error))
//                 }
//             }
//         })
//     }
// }
