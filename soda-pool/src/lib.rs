#![warn(clippy::unwrap_used)]
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

mod pool;
pub use pool::*;

mod dns;

mod broken_endpoints;

mod endpoint_template;
pub use endpoint_template::*;

mod retry;
pub use retry::*;

#[doc(hidden)]
pub mod deps;

mod macros;
mod ready_channels;
