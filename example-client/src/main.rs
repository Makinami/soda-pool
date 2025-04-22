use std::time::Duration;

use auto_discovery::{EndpointTemplate, RetryCheckResult, RetryTime, ServerStatus};
use example_protobuf::WrappedClient;
use tokio::{task::JoinSet, time::interval};
use tracing::info;
use tracing_subscriber::EnvFilter;
use url::Url;

struct AlwaysRetry;
impl auto_discovery::RetryPolicy for AlwaysRetry {
    fn should_retry(_error: &tonic::Status, _tries: usize) -> RetryCheckResult {
        RetryCheckResult(ServerStatus::Dead, RetryTime::Immediately)
    }
}

#[tokio::main]
async fn main() {
    let mut set = JoinSet::new();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let template = EndpointTemplate::new(Url::parse("http://localhost:50001").unwrap()).unwrap();
    let client = WrappedClient::new(template).await.unwrap();

    for _ in 0..4 {
        let client = client.clone();
        set.spawn(async move {
            let mut interval = interval(Duration::from_millis(1000));
            loop {
                interval.tick().await;
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
