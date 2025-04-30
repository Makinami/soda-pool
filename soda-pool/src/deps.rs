// Re-export dependencies for convenience and to avoid users having to add them
// to their own Cargo.toml files.

// memo: We could maybe support also other async libraries through feature flags.
pub use tokio::time::sleep;

pub use paste::paste;
