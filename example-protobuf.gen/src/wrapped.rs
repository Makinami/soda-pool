use auto_discovery::{define_client, define_method};

use crate::health_client::HealthClient;

define_client!(
    WrappedClient, HealthClient,
    (is_alive, (), crate::health::IsAliveResponse),
);
