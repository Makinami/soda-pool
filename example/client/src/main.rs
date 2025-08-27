#![allow(unexpected_cfgs)]
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

use std::time::Duration;

use example_protobuf::health_pool::health_client::HealthClientPool;
use soda_pool::{EndpointTemplate, RetryPolicyResult, RetryTime, ServerStatus};
use tokio::{task::JoinSet, time::interval};
use tracing::info;
use tracing_subscriber::EnvFilter;
use url::Url;

struct AlwaysRetry;
impl soda_pool::RetryPolicy for AlwaysRetry {
    fn should_retry(_error: &tonic::Status, _tries: u8) -> RetryPolicyResult {
        (ServerStatus::Dead, RetryTime::Immediately)
    }
}

#[tokio::main]
async fn main() {
    let mut set = JoinSet::new();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let template = EndpointTemplate::new(Url::parse("http://localhost:50001").unwrap()).unwrap();
    let client = HealthClientPool::new_from_endpoint(template);

    for _ in 0..4 {
        let client = client.clone();
        set.spawn(async move {
            let mut interval = interval(Duration::from_millis(100));
            loop {
                interval.tick().await;
                #[allow(deprecated)]
                let response = client.is_alive_with_retry::<AlwaysRetry>(()).await;
                match response {
                    Ok(r) => info!("Request successful: {:?}", r.into_inner().message),
                    Err(e) => info!("Request failed: {:?}", e),
                }
            }
        });
    }

    while set.join_next().await.is_some() {}
}
