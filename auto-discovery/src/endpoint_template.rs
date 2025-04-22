use http::HeaderValue;
use std::{net::IpAddr, str::FromStr, time::Duration};
#[cfg(feature = "_tls-any")]
use tonic::transport::ClientTlsConfig;
use tonic::transport::{Endpoint, Uri};
use url::Host;
pub use url::Url;

#[derive(Debug, Clone)]
pub struct EndpointTemplate {
    url: Url,
    origin: Option<Uri>,
    user_agent: Option<HeaderValue>,
    concurrency_limit: Option<usize>,
    rate_limit: Option<(u64, Duration)>,
    timeout: Option<Duration>,
    #[cfg(feature = "_tls-any")]
    tls_config: Option<ClientTlsConfig>,
    buffer_size: Option<usize>,
    init_stream_window_size: Option<u32>,
    init_connection_window_size: Option<u32>,
    tcp_keepalive: Option<Duration>,
    tcp_nodelay: Option<bool>,
    http2_keep_alive_interval: Option<Duration>,
    http2_keep_alive_timeout: Option<Duration>,
    http2_keep_alive_while_idle: Option<bool>,
    connect_timeout: Option<Duration>,
    http2_adaptive_window: Option<bool>,
}

impl EndpointTemplate {
    pub fn new(url: impl Into<Url>) -> Result<Self, Error> {
        let url: Url = url.into();

        // Check if URL contains hostname that can be resolved with DNS
        match url.host() {
            Some(host) => match host {
                Host::Domain(_) => {}
                _ => return Err(Error::AlreadyIpAddress),
            },
            None => return Err(Error::HostMissing),
        }

        // Check if hostname in URL can be substituted by IP address
        if url.cannot_be_a_base() {
            // Since we have a host, I can't imagine an address that still
            // couldn't be a base. If there is one, let's treat it as
            // Inconvertible error for simplicity.
            return Err(Error::Inconvertible);
        }

        // Check if tonic Uri can be build from Url.
        if Uri::from_str(url.as_str()).is_err() {
            // It's hard to prove that any url::Url will also be parsable as
            // tonic::transport::Uri, but in practice this error should never
            // happen.
            return Err(Error::Inconvertible);
        }

        Ok(Self {
            url,
            origin: None,
            user_agent: None,
            timeout: None,
            #[cfg(feature = "_tls-any")]
            tls_config: None,
            concurrency_limit: None,
            rate_limit: None,
            buffer_size: None,
            init_stream_window_size: None,
            init_connection_window_size: None,
            tcp_keepalive: None,
            tcp_nodelay: None,
            http2_keep_alive_interval: None,
            http2_keep_alive_timeout: None,
            http2_keep_alive_while_idle: None,
            connect_timeout: None,
            http2_adaptive_window: None,
        })
    }

    #[must_use]
    pub fn origin(self, origin: Uri) -> Self {
        Self {
            origin: Some(origin),
            ..self
        }
    }

    pub fn user_agent(self, user_agent: impl TryInto<HeaderValue>) -> Result<Self, Error> {
        user_agent
            .try_into()
            .map(|ua| Self {
                user_agent: Some(ua),
                ..self
            })
            .map_err(|_| Error::InvalidUserAgent)
    }

    #[must_use]
    pub fn timeout(self, dur: Duration) -> Self {
        Self {
            timeout: Some(dur),
            ..self
        }
    }

    #[cfg(feature = "_tls-any")]
    pub fn tls_config(self, tls_config: ClientTlsConfig) -> Result<Self, Error> {
        // Make sure we'll be able to build the Endpoint using this ClientTlsConfig
        let endpoint = self.build(std::net::Ipv4Addr::new(127, 0, 0, 1));
        let _ = endpoint
            .tls_config(tls_config.clone())
            .map_err(|_| Error::TlsInvalidUrl)?;

        Ok(Self {
            tls_config: Some(tls_config),
            ..self
        })
    }

    #[must_use]
    pub fn connect_timeout(self, dur: Duration) -> Self {
        Self {
            connect_timeout: Some(dur),
            ..self
        }
    }

    #[must_use]
    pub fn tcp_keepalive(self, tcp_keepalive: Option<Duration>) -> Self {
        Self {
            tcp_keepalive,
            ..self
        }
    }

    #[must_use]
    pub fn concurrency_limit(self, limit: usize) -> Self {
        Self {
            concurrency_limit: Some(limit),
            ..self
        }
    }

    #[must_use]
    pub fn rate_limit(self, limit: u64, duration: Duration) -> Self {
        Self {
            rate_limit: Some((limit, duration)),
            ..self
        }
    }

    #[must_use]
    pub fn initial_stream_window_size(self, sz: impl Into<Option<u32>>) -> Self {
        Self {
            init_stream_window_size: sz.into(),
            ..self
        }
    }

    #[must_use]
    pub fn initial_connection_window_size(self, sz: impl Into<Option<u32>>) -> Self {
        Self {
            init_connection_window_size: sz.into(),
            ..self
        }
    }

    #[must_use]
    pub fn buffer_size(self, sz: impl Into<Option<usize>>) -> Self {
        Self {
            buffer_size: sz.into(),
            ..self
        }
    }

    #[must_use]
    pub fn tcp_nodelay(self, enabled: bool) -> Self {
        Self {
            tcp_nodelay: Some(enabled),
            ..self
        }
    }

    #[must_use]
    pub fn http2_keep_alive_interval(self, interval: Duration) -> Self {
        Self {
            http2_keep_alive_interval: Some(interval),
            ..self
        }
    }

    #[must_use]
    pub fn keep_alive_timeout(self, duration: Duration) -> Self {
        Self {
            http2_keep_alive_timeout: Some(duration),
            ..self
        }
    }

    #[must_use]
    pub fn keep_alive_while_idle(self, enabled: bool) -> Self {
        Self {
            http2_keep_alive_while_idle: Some(enabled),
            ..self
        }
    }

    #[must_use]
    pub fn http2_adaptive_window(self, enabled: bool) -> Self {
        Self {
            http2_adaptive_window: Some(enabled),
            ..self
        }
    }

    pub fn build(&self, ip_address: impl Into<IpAddr>) -> Endpoint {
        let mut endpoint = Endpoint::from(self.build_uri(ip_address.into()));

        if let Some(origin) = self.origin.clone() {
            endpoint = endpoint.origin(origin);
        }

        if let Some(user_agent) = self.user_agent.clone() {
            endpoint = endpoint
                .user_agent(user_agent)
                .expect("already checked in the setter");
        }

        if let Some(timeout) = self.timeout {
            endpoint = endpoint.timeout(timeout);
        }

        #[cfg(feature = "_tls-any")]
        if let Some(tls_config) = self.tls_config.clone() {
            endpoint = endpoint
                .tls_config(tls_config)
                .expect("already checked in the setter");
        }

        if let Some(connect_timeout) = self.connect_timeout {
            endpoint = endpoint.connect_timeout(connect_timeout);
        }

        endpoint = endpoint.tcp_keepalive(self.tcp_keepalive);

        if let Some(limit) = self.concurrency_limit {
            endpoint = endpoint.concurrency_limit(limit);
        }

        if let Some((limit, duration)) = self.rate_limit {
            endpoint = endpoint.rate_limit(limit, duration);
        }

        if let Some(sz) = self.init_stream_window_size {
            endpoint = endpoint.initial_stream_window_size(sz);
        }

        if let Some(sz) = self.init_connection_window_size {
            endpoint = endpoint.initial_connection_window_size(sz);
        }

        endpoint = endpoint.buffer_size(self.buffer_size);

        if let Some(tcp_nodelay) = self.tcp_nodelay {
            endpoint = endpoint.tcp_nodelay(tcp_nodelay);
        }

        if let Some(interval) = self.http2_keep_alive_interval {
            endpoint = endpoint.http2_keep_alive_interval(interval);
        }

        if let Some(duration) = self.http2_keep_alive_timeout {
            endpoint = endpoint.keep_alive_timeout(duration);
        }

        if let Some(enabled) = self.http2_keep_alive_while_idle {
            endpoint = endpoint.keep_alive_while_idle(enabled);
        }

        if let Some(enabled) = self.http2_adaptive_window {
            endpoint = endpoint.http2_adaptive_window(enabled);
        }

        endpoint
    }

    pub fn domain(&self) -> &str {
        self.url
            .domain()
            .expect("already checked in the constructor")
    }

    fn build_uri(&self, ip_addr: IpAddr) -> Uri {
        // We make sure this conversion doesn't return any errors in Self::new
        // already so it's safe to unwrap here.
        let mut url = self.url.clone();
        url.set_ip_host(ip_addr)
            .expect("already checked in the constructor by trying cannot_be_a_base");
        Uri::from_str(url.as_str()).expect("starting from Url, this should always be a valid Uri")
    }
}

#[derive(Debug, PartialEq)]
pub enum Error {
    HostMissing,
    AlreadyIpAddress,
    Inconvertible,
    InvalidUserAgent,
    #[cfg(feature = "_tls-any")]
    TlsInvalidUrl,
}

impl TryFrom<Url> for EndpointTemplate {
    type Error = Error;

    fn try_from(url: Url) -> Result<Self, Self::Error> {
        Self::new(url)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::{net::IpAddr, str::FromStr};

    use http::Uri;
    use url::Url;

    use super::*;

    #[test]
    fn can_substitute_domain_fot_ipv4_address() {
        let builder =
            EndpointTemplate::new(Url::parse("http://example.com:50051/foo").unwrap()).unwrap();

        let endpoint = builder.build("203.0.113.6".parse::<IpAddr>().unwrap());
        assert_eq!(
            *endpoint.uri(),
            Uri::from_str("http://203.0.113.6:50051/foo").unwrap()
        );
    }

    #[test]
    fn can_substitute_domain_fot_ipv6_address() {
        let builder =
            EndpointTemplate::new(Url::parse("http://example.com:50051/foo").unwrap()).unwrap();

        let endpoint = builder.build("2001:db8::".parse::<IpAddr>().unwrap());
        assert_eq!(
            *endpoint.uri(),
            Uri::from_str("http://[2001:db8::]:50051/foo").unwrap()
        );
    }

    #[rstest::rstest]
    #[case("http://127.0.0.1:50051", Error::AlreadyIpAddress)]
    #[case("http://[::1]:50051", Error::AlreadyIpAddress)]
    #[case("mailto:admin@example.com", Error::HostMissing)]
    fn builder_error(#[case] input: &str, #[case] expected: Error) {
        let result = EndpointTemplate::new(Url::parse(input).unwrap());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), expected);
    }
}
