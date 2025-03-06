use std::time::Duration;

use auto_discovery::{EndpointTemplate, CloneableRequest};
use example_protobuf::WrappedClient;
use tokio::{task::JoinSet, time::interval};
use tracing::info;
use url::Url;

#[tokio::main]
async fn main() {
    let mut set = JoinSet::new();
    colog::init();

    let template = EndpointTemplate::new(Url::parse("http://localhost:50001").unwrap()).unwrap();
    let client = WrappedClient::new(template, Duration::from_secs(1));

    for _ in 0..4 {
        let mut client = client.clone();
        set.spawn(async move {
            let mut interval = interval(Duration::from_millis(10000));
            let request = CloneableRequest::new(());
            loop {
                interval.tick().await;
                let response = client.is_alive(request.clone()).await;
                match response {
                    Ok(r) => info!("Request successful: {:?}", r.into_inner().message),
                    Err(e) => info!("Request failed: {:?}", e),
                }
            }
        });
    }

    while set.join_next().await.is_some() {}
}
