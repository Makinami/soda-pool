use std::time::Duration;

use example_protobuf::{EndpointTemplate, RequestGenerator, WrappedHealthClient};
use tokio::{task::JoinSet, time::interval};
use tracing::info;
use url::Url;

#[tokio::main]
async fn main() {
    let mut set = JoinSet::new();
    colog::init();

    let template = EndpointTemplate::new(Url::parse("http://localhost:50000").unwrap()).unwrap();
    let client = WrappedHealthClient::new(template);

    for _ in 0..4 {
        let mut client = client.clone();
        set.spawn(async move {
            let mut interval = interval(Duration::from_millis(1000));
            let request = RequestGenerator::new(());
            loop {
                interval.tick().await;
                let response = client.is_alive(request.clone()).await.unwrap().into_inner();
                info!("Response: {:?}", response);
            }
        });
    }

    while let Some(_) = set.join_next().await {}
}
