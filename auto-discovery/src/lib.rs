#![warn(clippy::unwrap_used)]
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

mod wrapped;
pub use wrapped::*;

mod dns;

mod broken_endpoints;

mod endpoint_template;
pub use endpoint_template::*;
