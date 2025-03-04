use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddrV4},
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
    //let mut set = JoinSet::new();

    let mut tasks = HashMap::new();

    for i in 0..4 {
        let port = 50000 + i;
        tasks.insert(
            port,
            tokio::spawn(async move {
                let address = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), port);
                tonic::transport::Server::builder()
                    .add_service(HealthServer::new(HealthImpl {
                        port: address.port(),
                    }))
                    .add_service(EchoServer::new(EchoImpl {}))
                    .serve(address.into())
                    .await
            }),
        );
    }

    let mut input = String::new();
    while std::io::stdin().read_line(&mut input).is_ok() {
        if input.trim() == "exit" {
            break;
        }

        if input.starts_with("start") {
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() != 2 {
                println!("Invalid command");
                input.clear();
                continue;
            }

            let port = parts[1].parse::<u16>().unwrap();
            if tasks.get(&port).is_some() {
                println!("Task on port {} already started", port);
            } else {
                tasks.insert(
                    port,
                    tokio::spawn(async move {
                        let address = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), port);
                        tonic::transport::Server::builder()
                            .add_service(HealthServer::new(HealthImpl {
                                port: address.port(),
                            }))
                            .add_service(EchoServer::new(EchoImpl {}))
                            .serve(address.into())
                            .await
                    }),
                );
                println!("Task on port {} started", port);
            }
        }

        if input.starts_with("stop") {
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() != 2 {
                println!("Invalid command");
                input.clear();
                continue;
            }

            let port = parts[1].parse::<u16>().unwrap();
            if let Some(task) = tasks.get(&port) {
                task.abort();
                println!("Task on port {} stopped", port);
            }
        }

        if input.starts_with("list") {
            for (port, task) in tasks.iter() {
                println!("Task on port {} is running: {}", port, !task.is_finished());
            }
        }

        input.clear();
    }
}

struct HealthImpl {
    pub port: u16,
}

#[tonic::async_trait]
impl Health for HealthImpl {
    async fn is_alive(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<tonic::Response<example_protobuf::IsAliveResponse>, tonic::Status> {
        sleep(Duration::from_millis(random_range(200..1000))).await;
        Ok(tonic::Response::new(example_protobuf::IsAliveResponse {
            message: format!("I'm alive! (from port: {})", self.port),
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
