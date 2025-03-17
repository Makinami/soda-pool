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
use rand::{Rng, random_range};
use tokio::time::sleep;

const PORT: u16 = 50001;

#[tokio::main]
async fn main() {
    let mut tasks = HashMap::new();
    let mut reliability = 0.9;
    let mut avg_wait = Duration::from_millis(500);

    macro_rules! start_server {
        ($address:expr) => {
            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            let server = tokio::spawn(async move {
                tonic::transport::Server::builder()
                    .add_service(HealthServer::new(HealthImpl {
                        address: $address,
                        reliability,
                        avg_wait,
                    }))
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

    macro_rules! restart_servers {
        () => {
            let addresses: Vec<_> = tasks.keys().copied().collect();
            for address in addresses {
                stop_server!(address);
                start_server!(address);
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
                println!("Reliability: {}", reliability);
                println!("Average wait: {} ms", avg_wait.as_millis());
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
                println!("  start v4               Start server on IPv4 localhost");
                println!("  start v6               Start server on IPv6 localhost");
                println!("  stop v4                Stop server on IPv4 localhost");
                println!("  stop v6                Stop server on IPv6 localhost");
                println!("  reliability <value>    Set server reliability (0.0-1.0)");
                println!("  avg_wait <value>       Set average wait time in milliseconds");
                println!("  list                   List running servers");
                println!("  help                   Display this help message");
                println!("  exit                   Exit the program");
            }
            cmd => {
                if cmd.starts_with("reliability ") {
                    if let Ok(value) = cmd.split_once(' ').unwrap().1.parse() {
                        reliability = value;
                        println!(
                            "Reliability set to {}. Restarting servers (if there are any)...",
                            reliability
                        );
                        restart_servers!();
                    } else {
                        println!("Invalid reliability value");
                    }
                } else if cmd.starts_with("avg_wait ") {
                    if let Ok(value) = cmd.split_once(' ').unwrap().1.parse() {
                        avg_wait = Duration::from_millis(value);
                        println!(
                            "Average wait set to {}. Restarting servers (if there are any)...",
                            avg_wait.as_millis()
                        );
                        restart_servers!();
                    } else {
                        println!("Invalid average wait value");
                    }
                } else {
                    println!("Unknown command: {}", cmd);
                }
            }
        }

        input.clear();
    }
}

struct HealthImpl {
    pub address: SocketAddr,
    pub reliability: f64,
    pub avg_wait: Duration,
}

#[tonic::async_trait]
impl Health for HealthImpl {
    async fn is_alive(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<tonic::Response<example_protobuf::IsAliveResponse>, tonic::Status> {
        if !self.avg_wait.is_zero() {
            let wait_range = (self.avg_wait / 2)..(self.avg_wait * 2);
            sleep(random_range(wait_range)).await;
        }
        if rand::rng().random_bool(self.reliability) {
            Ok(tonic::Response::new(example_protobuf::IsAliveResponse {
                message: format!("I'm alive! (from: {:?})", self.address),
            }))
        } else {
            Err(tonic::Status::internal("Server internal error"))
        }
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
