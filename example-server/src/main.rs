use std::{
    collections::HashMap,
    io::Write,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

use example_protobuf::{
    echo_server::{Echo, EchoServer},
    health_server::{Health, HealthServer},
};
use rand::random_range;
use tokio::time::sleep;

const PORT: u16 = 50001;

#[tokio::main]
async fn main() -> () {
    let mut tasks = HashMap::new();

    macro_rules! start_server {
        ($address:expr) => {
            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            let server = tokio::spawn(async move {
                tonic::transport::Server::builder()
                    .add_service(HealthServer::new(HealthImpl { address: $address }))
                    .add_service(EchoServer::new(EchoImpl {}))
                    .serve_with_shutdown($address, async {
                        rx.await.ok();
                    })
                    .await
            });
            tasks.insert($address, (server, tx));
            println!("Started server on {}", $address);
        };
    }

    macro_rules! try_start_server {
        ($address:expr) => {
            if tasks.get(&$address).is_some() {
                println!("Server on {} already started", $address);
            } else {
                start_server!($address);
            }
        };
    }

    macro_rules! stop_server {
        ($address:expr) => {
            if let Some(task) = tasks.remove(&$address) {
                let _ = task.1.send(());
                println!("Server on {} scheduled to stop", $address);
            }
        };
    }

    println!("Type 'help' for a list of available commands");

    let mut input = String::new();
    while {
        print!("> ");
        let _ = std::io::stdout().flush();
        std::io::stdin().read_line(&mut input).is_ok()
    } {
        match input.trim() {
            "exit" => break,
            "start v4" => {
                let address = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), PORT);
                try_start_server!(address);
            }
            "start v6" => {
                let address = SocketAddr::new(Ipv6Addr::LOCALHOST.into(), PORT);
                try_start_server!(address);
            }
            "stop v4" => {
                let address = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), PORT);
                stop_server!(address);
            }
            "stop v6" => {
                let address = SocketAddr::new(Ipv6Addr::LOCALHOST.into(), PORT);
                stop_server!(address);
            }
            "list" => {
                for (address, task) in tasks.iter() {
                    println!(
                        "Server on socket {} is running: {}",
                        address,
                        !task.0.is_finished()
                    );
                }
                if tasks.is_empty() {
                    println!("No servers running");
                }
            }
            "help" => {
                println!("Available commands:");
                println!("  start v4   Start server on IPv4 localhost");
                println!("  start v6   Start server on IPv6 localhost");
                println!("  stop v4    Stop server on IPv4 localhost");
                println!("  stop v6    Stop server on IPv6 localhost");
                println!("  list       List running servers");
                println!("  exit       Exit the program");
            }
            cmd => println!("Unknown command: {}", cmd),
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
