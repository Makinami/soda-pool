#![allow(unexpected_cfgs)]
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

#[rustfmt::skip]
mod health;
pub use health::*;

mod wrapped;
pub use wrapped::*;
