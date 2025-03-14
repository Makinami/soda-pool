use std::{str::FromStr, sync::atomic::AtomicBool, time::Duration};

use auto_discovery::{CloneableRequest, EndpointTemplate};
use criterion::{black_box, BenchmarkId, Criterion, Throughput};
use std::sync::atomic::Ordering::Relaxed;
use example_protobuf::{WrappedClient, health_client::HealthClient};
use futures::future::try_join_all;
use tokio::{runtime::Runtime, time::sleep};
use tonic::{transport::Endpoint, IntoRequest, Status};
use url::Url;

pub fn grpc_client(c: &mut Criterion) {
    let runner = Runtime::new().unwrap();
    let mut group = c.benchmark_group("grpc_client");

    let address = std::env::var("ADDRESS").unwrap_or_else(|_| "http://localhost:50001".to_string());

    let template = EndpointTemplate::new(
        Url::parse(&address).unwrap(),
    )
    .unwrap();
    let endpoint = Endpoint::from_str(address.as_str()).unwrap();
    let client = runner.block_on(async {
        let client = WrappedClient::new(template, Duration::from_secs(1));
        sleep(Duration::from_secs(3)).await;
        client
    });

    let test_cases = [1, 2, 4, 8, 16, 32, 64];
    let prev_test_failed = AtomicBool::new(false);

    for i in test_cases.iter() {
        if prev_test_failed.load(Relaxed) {
            break;
        }
        group.throughput(Throughput::Elements(*i as u64));
        group.bench_with_input(BenchmarkId::new("wrapped", i), &i, |b, _i| {
            b.to_async(&runner).iter(|| async {
                let res = try_join_all(
                    (0..*i).map(|_| client.is_alive(CloneableRequest::new(()))),
                )
                .await;
                prev_test_failed.store(black_box(res).is_err(), Relaxed);
            })
        });
    }
    if prev_test_failed.load(Relaxed) {
        println!("Some tests failed.");
    }

    prev_test_failed.store(false, Relaxed);
    for i in test_cases.iter() {
        if prev_test_failed.load(Relaxed) {
            break;
        }
        group.throughput(Throughput::Elements(*i as u64));
        group.bench_with_input(BenchmarkId::new("reconnect", i), &i, |b, _i| {
            b.to_async(&runner).iter(|| async {
                let res = try_join_all((0..*i).map(|_| async {
                    let mut client = HealthClient::connect(endpoint.clone()).await.map_err(|_| Status::unknown(""))?;
                    client.is_alive(().into_request()).await
                }))
                .await;
                prev_test_failed.store(black_box(res).is_err(), Relaxed);
            })
        });
    }
    if prev_test_failed.load(Relaxed) {
        println!("Some tests failed.");
    }

    group.finish();

    let mut group = c.benchmark_group("grpc_connection");

    group.bench_function("connect", |b| {
        b.to_async(&runner).iter(|| async {
            let _ = black_box(HealthClient::connect(endpoint.clone()).await);
        });
    });

    group.finish();
}

pub fn benches() {
}

fn main() {
    let mut criterion = Criterion::default()
        .configure_from_args();

    grpc_client(&mut criterion);

    criterion.final_summary();
}
