use std::sync::Arc;

use tonic::Status;

use crate::broken_endpoints::BrokenEndpointsError;

#[derive(Debug)]
pub enum WrappedClientError {
    NoReadyChannels,
    BrokenLock,
    DnsResolutionError(std::io::Error),
}

impl WrappedClientError {
    #[must_use]
    pub fn dns_resolution_error(e: std::io::Error) -> Self {
        WrappedClientError::DnsResolutionError(e)
    }
}

impl From<BrokenEndpointsError> for WrappedClientError {
    fn from(_: BrokenEndpointsError) -> Self {
        WrappedClientError::BrokenLock
    }
}

impl std::fmt::Display for WrappedClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WrappedClientError::NoReadyChannels => std::fmt::Display::fmt("No ready channels", f),
            WrappedClientError::BrokenLock => std::fmt::Display::fmt("Broken lock", f),
            WrappedClientError::DnsResolutionError(e) => {
                std::fmt::Display::fmt("DNS resolution error: ", f)?;
                std::fmt::Display::fmt(e, f)
            }
        }
    }
}

impl std::error::Error for WrappedClientError {}

impl From<WrappedClientError> for Status {
    fn from(e: WrappedClientError) -> Self {
        match e {
            WrappedClientError::NoReadyChannels => {
                let mut status = Status::new(tonic::Code::Unavailable, "No ready channels");
                status.set_source(Arc::new(e));
                status
            }
            WrappedClientError::BrokenLock => {
                let mut status = Status::new(tonic::Code::Internal, "Broken lock");
                status.set_source(Arc::new(e));
                status
            }
            WrappedClientError::DnsResolutionError(e) => {
                let mut status = Status::new(tonic::Code::Unavailable, "DNS resolution error");
                status.set_source(Arc::new(e));
                status
            }
        }
    }
}
