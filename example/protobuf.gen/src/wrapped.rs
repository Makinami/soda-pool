use soda_pool::{define_client, define_method, ChannelPool};

use super::health::health_client::HealthClient;

define_client!(
    WrappedClient,
    HealthClient,
    (is_alive, (), crate::health::IsAliveResponse),
);
