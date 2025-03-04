use std::{
    collections::HashMap,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    time::Duration,
};

use example_protobuf::{
    echo_server::{Echo, EchoServer},
    health_server::{Health, HealthServer},
};
use rand::random_range;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> () {
    let mut tasks = HashMap::new();

    let mut input = String::new();
    while std::io::stdin().read_line(&mut input).is_ok() {
        if input.trim() == "exit" {
            break;
        }

        if input.starts_with("start v4") {
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() != 3 {
                println!("Invalid command");
                input.clear();
                continue;
            }

            let port = parts[2].parse::<u16>().unwrap();
            if tasks.get(&port).is_some() {
                println!("Task on port {} already started", port);
            } else {
                let (tx, rx) = tokio::sync::oneshot::channel::<()>();
                let server = tokio::spawn(async move {
                    let address = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), port).into();
                    tonic::transport::Server::builder()
                        .add_service(HealthServer::new(HealthImpl {
                            address,
                        }))
                        .add_service(EchoServer::new(EchoImpl {}))
                        .serve_with_shutdown(address, async {
                            rx.await.ok();
                        })
                        .await
                });
                tasks.insert(
                    port + 4000,
                    (server, tx),
                );
                println!("Task on port {} started", port);
            }
        }

        if input.starts_with("start v6") {
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() != 3 {
                println!("Invalid command");
                input.clear();
                continue;
            }

            let port = parts[2].parse::<u16>().unwrap();
            if tasks.get(&port).is_some() {
                println!("Task on port {} already started", port);
            } else {
                let (tx, rx) = tokio::sync::oneshot::channel::<()>();
                let server = tokio::spawn(async move {
                    let address = SocketAddrV6::new(Ipv6Addr::LOCALHOST, port, 0, 0).into();
                    tonic::transport::Server::builder()
                        .add_service(HealthServer::new(HealthImpl {
                            address,
                        }))
                        .add_service(EchoServer::new(EchoImpl {}))
                        .serve_with_shutdown(address, async {
                            rx.await.ok();
                        })
                        .await
                });
                tasks.insert(
                    port + 6000,
                    (server, tx),
                );
                println!("Task on port {} started", port);
            }
        }

        if input.starts_with("stop v4") {
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() != 3 {
                println!("Invalid command");
                input.clear();
                continue;
            }

            let port = parts[2].parse::<u16>().unwrap();
            if let Some(task) = tasks.remove(&(port + 4000)) {
                let _ = task.1.send(());
                println!("Task on port {} stopped", port);
            }
        }

        if input.starts_with("stop v6") {
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() != 3 {
                println!("Invalid command");
                input.clear();
                continue;
            }

            let port = parts[2].parse::<u16>().unwrap();
            if let Some(task) = tasks.remove(&(port + 6000)) {
                let _ = task.1.send(());
                println!("Task on port {} stopped", port);
            }
        }

        if input.starts_with("list") {
            for (port, task) in tasks.iter() {
                println!("Task on port {} is running: {}", port, !task.0.is_finished());
            }
        }

        input.clear();
    }
}

struct HealthImpl {
    pub address: SocketAddr,
}

#[tonic::async_trait]
impl Health for HealthImpl {
    async fn is_alive(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<tonic::Response<example_protobuf::IsAliveResponse>, tonic::Status> {
        sleep(Duration::from_millis(random_range(200..1000))).await;
        Ok(tonic::Response::new(example_protobuf::IsAliveResponse {
            message: format!("I'm alive! (from: {:?})", self.address),
        }))
    }
}

struct EchoImpl {}

#[tonic::async_trait]
impl Echo for EchoImpl {
    type EchoStreamStream = tonic::codec::Streaming<example_protobuf::EchoResponse>;

    async fn echo_message(
        &self,
        request: tonic::Request<example_protobuf::EchoRequest>,
    ) -> Result<tonic::Response<example_protobuf::EchoResponse>, tonic::Status> {
        Ok(tonic::Response::new(example_protobuf::EchoResponse {
            message: request.into_inner().message,
        }))
    }

    async fn echo_stream(
        &self,
        _request: tonic::Request<tonic::Streaming<example_protobuf::EchoRequest>>,
    ) -> Result<tonic::Response<tonic::Streaming<example_protobuf::EchoResponse>>, tonic::Status>
    {
        todo!("Implement Echo::echo_stream")
    }
}
