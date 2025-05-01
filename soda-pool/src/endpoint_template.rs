use core::fmt;
use http::HeaderValue;
use std::error::Error;
use std::fmt::{Debug, Display};
use std::{net::IpAddr, str::FromStr, time::Duration};
#[cfg(feature = "_tls-any")]
use tonic::transport::ClientTlsConfig;
use tonic::transport::{Endpoint, Uri};
use url::Host;
use url::Url;

/// Template for creating [`Endpoint`]s.
///
/// This structure is used to store all the information necessary to create an [`Endpoint`].
/// It then creates an [`Endpoint`] to a specific IP address using the [`build`](EndpointTemplate::build) method.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct EndpointTemplate {
    url: Url,
    origin: Option<Uri>,
    user_agent: Option<HeaderValue>,
    timeout: Option<Duration>,
    concurrency_limit: Option<usize>,
    rate_limit: Option<(u64, Duration)>,
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
    http2_max_header_list_size: Option<u32>,
    connect_timeout: Option<Duration>,
    http2_adaptive_window: Option<bool>,
    local_address: Option<IpAddr>,
    // todo: If at all possible, support also setting the executor.
}

impl EndpointTemplate {
    /// Creates a new `EndpointTemplate` from the provided URL.
    ///
    /// # Errors
    /// - Will return [`EndpointTemplateError::HostMissing`] if the provided URL does not contain a host.
    /// - Will return [`EndpointTemplateError::AlreadyIpAddress`] if the provided URL already contains an IP address.
    /// - Will return [`EndpointTemplateError::Inconvertible`] if the provided URL cannot be converted to the tonic's internal representation.
    pub fn new(url: impl Into<Url>) -> Result<Self, EndpointTemplateError> {
        let url: Url = url.into();

        // Check if URL contains hostname that can be resolved with DNS
        match url.host() {
            Some(host) => match host {
                Host::Domain(_) => {}
                _ => return Err(EndpointTemplateError::AlreadyIpAddress),
            },
            None => return Err(EndpointTemplateError::HostMissing),
        }

        // Check if hostname in URL can be substituted by IP address
        if url.cannot_be_a_base() {
            // Since we have a host, I can't imagine an address that still
            // couldn't be a base. If there is one, let's treat it as
            // Inconvertible error for simplicity.
            return Err(EndpointTemplateError::Inconvertible);
        }

        // Check if tonic Uri can be build from Url.
        if Uri::from_str(url.as_str()).is_err() {
            // It's hard to prove that any url::Url will also be parsable as
            // tonic::transport::Uri, but in practice this error should never
            // happen.
            return Err(EndpointTemplateError::Inconvertible);
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
            http2_max_header_list_size: None,
            connect_timeout: None,
            http2_adaptive_window: None,
            local_address: None,
        })
    }

    /// Builds an [`Endpoint`] to the IP address.
    ///
    /// This will substitute the hostname in the URL with the provided IP
    /// address, create a new [`Endpoint`] from it, and apply all the settings
    /// set in the builder.
    #[allow(clippy::missing_panics_doc)]
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

        if let Some(size) = self.http2_max_header_list_size {
            endpoint = endpoint.http2_max_header_list_size(size);
        }

        endpoint = endpoint.local_address(self.local_address);

        endpoint
    }

    /// Returns the hostname of the URL held in the template.
    #[allow(clippy::missing_panics_doc)]
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

    /// r.f. [`Endpoint::user_agent`].
    ///
    /// # Errors
    ///
    /// Will return [`EndpointTemplateError::InvalidUserAgent`] if the provided value cannot be converted to a [`HeaderValue`] and would cause a failure when building an endpoint.
    pub fn user_agent(
        self,
        user_agent: impl TryInto<HeaderValue>,
    ) -> Result<Self, EndpointTemplateError> {
        user_agent
            .try_into()
            .map(|ua| Self {
                user_agent: Some(ua),
                ..self
            })
            .map_err(|_| EndpointTemplateError::InvalidUserAgent)
    }

    /// r.f. [`Endpoint::origin`].
    #[must_use]
    pub fn origin(self, origin: Uri) -> Self {
        Self {
            origin: Some(origin),
            ..self
        }
    }

    /// r.f. [`Endpoint::timeout`].
    #[must_use]
    pub fn timeout(self, dur: Duration) -> Self {
        Self {
            timeout: Some(dur),
            ..self
        }
    }

    /// r.f. [`Endpoint::connect_timeout`].
    #[must_use]
    pub fn connect_timeout(self, dur: Duration) -> Self {
        Self {
            connect_timeout: Some(dur),
            ..self
        }
    }

    /// r.f. [`Endpoint::tcp_keepalive`].
    #[must_use]
    pub fn tcp_keepalive(self, tcp_keepalive: Option<Duration>) -> Self {
        Self {
            tcp_keepalive,
            ..self
        }
    }

    /// r.f. [`Endpoint::concurrency_limit`]
    #[must_use]
    pub fn concurrency_limit(self, limit: usize) -> Self {
        Self {
            concurrency_limit: Some(limit),
            ..self
        }
    }

    /// r.f. [`Endpoint::rate_limit`].
    #[must_use]
    pub fn rate_limit(self, limit: u64, duration: Duration) -> Self {
        Self {
            rate_limit: Some((limit, duration)),
            ..self
        }
    }

    /// r.f. [`Endpoint::initial_stream_window_size`].
    #[must_use]
    pub fn initial_stream_window_size(self, sz: impl Into<Option<u32>>) -> Self {
        Self {
            init_stream_window_size: sz.into(),
            ..self
        }
    }

    /// r.f. [`Endpoint::initial_connection_window_size`].
    #[must_use]
    pub fn initial_connection_window_size(self, sz: impl Into<Option<u32>>) -> Self {
        Self {
            init_connection_window_size: sz.into(),
            ..self
        }
    }

    /// r.f. [`Endpoint::buffer_size`].
    #[must_use]
    pub fn buffer_size(self, sz: impl Into<Option<usize>>) -> Self {
        Self {
            buffer_size: sz.into(),
            ..self
        }
    }

    /// r.f. [`Endpoint::tls_config`].
    ///
    /// # Errors
    ///
    /// Will return [`EndpointTemplateError::InvalidTlsConfig`] if the provided config cannot be passed to an [`Endpoint`] and would cause a failure when building an endpoint.
    #[cfg(feature = "_tls-any")]
    pub fn tls_config(self, tls_config: ClientTlsConfig) -> Result<Self, EndpointTemplateError> {
        // Make sure we'll be able to build the Endpoint using this ClientTlsConfig
        let endpoint = self.build(std::net::Ipv4Addr::new(127, 0, 0, 1));
        let _ = endpoint
            .tls_config(tls_config.clone())
            .map_err(|_| EndpointTemplateError::InvalidTlsConfig)?;

        Ok(Self {
            tls_config: Some(tls_config),
            ..self
        })
    }

    /// r.f. [`Endpoint::tcp_nodelay`].
    #[must_use]
    pub fn tcp_nodelay(self, enabled: bool) -> Self {
        Self {
            tcp_nodelay: Some(enabled),
            ..self
        }
    }

    /// r.f. [`Endpoint::http2_keep_alive_interval`].
    #[must_use]
    pub fn http2_keep_alive_interval(self, interval: Duration) -> Self {
        Self {
            http2_keep_alive_interval: Some(interval),
            ..self
        }
    }

    /// r.f. [`Endpoint::keep_alive_timeout`].
    #[must_use]
    pub fn keep_alive_timeout(self, duration: Duration) -> Self {
        Self {
            http2_keep_alive_timeout: Some(duration),
            ..self
        }
    }

    /// r.f. [`Endpoint::keep_alive_while_idle`].
    #[must_use]
    pub fn keep_alive_while_idle(self, enabled: bool) -> Self {
        Self {
            http2_keep_alive_while_idle: Some(enabled),
            ..self
        }
    }

    /// r.f. [`Endpoint::http2_adaptive_window`].
    #[must_use]
    pub fn http2_adaptive_window(self, enabled: bool) -> Self {
        Self {
            http2_adaptive_window: Some(enabled),
            ..self
        }
    }

    /// r.f. [`Endpoint::http2_max_header_list_size`].
    #[must_use]
    pub fn http2_max_header_list_size(self, size: u32) -> Self {
        Self {
            http2_max_header_list_size: Some(size),
            ..self
        }
    }

    /// r.f. [`Endpoint::local_address`].
    #[must_use]
    pub fn local_address(self, ip: Option<IpAddr>) -> Self {
        Self {
            local_address: ip,
            ..self
        }
    }
}

impl Debug for EndpointTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EndpointTemplate")
            .field("url", &self.url.as_str())
            .finish_non_exhaustive()
    }
}

/// Errors that can occur when creating an [`EndpointTemplate`].
#[derive(Debug, PartialEq, Eq, Clone, Copy, PartialOrd, Ord, Hash)]
pub enum EndpointTemplateError {
    /// The URL does not contain a host.
    ///
    /// Provided URL does not contain a host that can be resolved with DNS.
    HostMissing,

    /// The URL is already an IP address.
    ///
    /// Provided URL is already an IP address, so it cannot be used as a template.
    AlreadyIpAddress,

    /// The URL cannot be converted to an internal type.
    ///
    /// tonic's [`Endpoint`](tonic::transport::Endpoint) uses its own
    /// [type](tonic::transport::Uri) for representing an address and provided
    /// URL (after substituting hostname for an IP address) could not be
    /// converted into it.
    Inconvertible,

    /// The provided user agent is invalid.
    ///
    /// Provided user agent cannot be converted to a [`HeaderValue`] and would
    /// cause a failure when building an endpoint.
    InvalidUserAgent,

    /// The provided TLS config is invalid.
    ///
    /// Provided TLS config would cause a failure when building an endpoint.
    #[cfg(feature = "_tls-any")]
    InvalidTlsConfig,
}

impl TryFrom<Url> for EndpointTemplate {
    type Error = EndpointTemplateError;

    fn try_from(url: Url) -> Result<Self, Self::Error> {
        Self::new(url)
    }
}

impl Display for EndpointTemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EndpointTemplateError::HostMissing => write!(f, "host missing"),
            EndpointTemplateError::AlreadyIpAddress => write!(f, "already an IP address"),
            EndpointTemplateError::Inconvertible => write!(f, "inconvertible URL"),
            EndpointTemplateError::InvalidUserAgent => write!(f, "invalid user agent"),
            #[cfg(feature = "_tls-any")]
            EndpointTemplateError::InvalidTlsConfig => write!(f, "invalid TLS config"),
        }
    }
}

impl Error for EndpointTemplateError {}

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
    #[case("http://127.0.0.1:50051", EndpointTemplateError::AlreadyIpAddress)]
    #[case("http://[::1]:50051", EndpointTemplateError::AlreadyIpAddress)]
    #[case("mailto:admin@example.com", EndpointTemplateError::HostMissing)]
    fn builder_error(#[case] input: &str, #[case] expected: EndpointTemplateError) {
        let result = EndpointTemplate::new(Url::parse(input).unwrap());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), expected);
    }

    #[rstest::rstest]
    #[case("http://example.com:50051/foo", Ok("example.com"))]
    #[case("http://127.0.0.1:50051", Err(EndpointTemplateError::AlreadyIpAddress))]
    #[case("http://[::1]:50051", Err(EndpointTemplateError::AlreadyIpAddress))]
    #[case("mailto:admin@example.com", Err(EndpointTemplateError::HostMissing))]
    fn from_trait(#[case] url: &str, #[case] expected: Result<&str, EndpointTemplateError>) {
        let url = Url::parse(url).unwrap();
        let result = EndpointTemplate::try_from(url.clone());
        let domain = result.as_ref().map(EndpointTemplate::domain);
        assert_eq!(domain, expected.as_deref());
    }

    #[test]
    fn setters() {
        let url = Url::parse("http://example.com:50051/foo").unwrap();
        let builder = EndpointTemplate::new(url.clone()).unwrap();

        let origin = Uri::from_str("http://example.net:50001").unwrap();
        let builder = builder.origin(origin.clone());
        assert_eq!(builder.origin, Some(origin));

        let user_agent = HeaderValue::from_str("my-user-agent").unwrap();
        let builder = builder.user_agent(user_agent.clone()).unwrap();
        assert_eq!(builder.user_agent, Some(user_agent));

        let duration = Duration::from_secs(10);
        let builder = builder.timeout(duration);
        assert_eq!(builder.timeout, Some(duration));

        let connect_timeout = Duration::from_secs(5);
        let builder = builder.connect_timeout(connect_timeout);
        assert_eq!(builder.connect_timeout, Some(connect_timeout));

        let tcp_keepalive = Some(Duration::from_secs(30));
        let builder = builder.tcp_keepalive(tcp_keepalive);
        assert_eq!(builder.tcp_keepalive, tcp_keepalive);

        let concurrency_limit = 10;
        let builder = builder.concurrency_limit(concurrency_limit);
        assert_eq!(builder.concurrency_limit, Some(concurrency_limit));

        let rate_limit = (100, Duration::from_secs(1));
        let builder = builder.rate_limit(rate_limit.0, rate_limit.1);
        assert_eq!(builder.rate_limit, Some(rate_limit));

        let init_stream_window_size = Some(64);
        let builder = builder.initial_stream_window_size(init_stream_window_size);
        assert_eq!(builder.init_stream_window_size, init_stream_window_size);

        let init_connection_window_size = Some(128);
        let builder = builder.initial_connection_window_size(init_connection_window_size);
        assert_eq!(
            builder.init_connection_window_size,
            init_connection_window_size
        );

        let buffer_size = Some(1024);
        let builder = builder.buffer_size(buffer_size);
        assert_eq!(builder.buffer_size, buffer_size);

        let tcp_nodelay = true;
        let builder = builder.tcp_nodelay(tcp_nodelay);
        assert_eq!(builder.tcp_nodelay, Some(tcp_nodelay));

        let http2_keep_alive_interval = Duration::from_secs(30);
        let builder = builder.http2_keep_alive_interval(http2_keep_alive_interval);
        assert_eq!(
            builder.http2_keep_alive_interval,
            Some(http2_keep_alive_interval)
        );

        let keep_alive_timeout = Duration::from_secs(60);
        let builder = builder.keep_alive_timeout(keep_alive_timeout);
        assert_eq!(builder.http2_keep_alive_timeout, Some(keep_alive_timeout));

        let keep_alive_while_idle = true;
        let builder = builder.keep_alive_while_idle(keep_alive_while_idle);
        assert_eq!(
            builder.http2_keep_alive_while_idle,
            Some(keep_alive_while_idle)
        );

        let http2_adaptive_window = true;
        let builder = builder.http2_adaptive_window(http2_adaptive_window);
        assert_eq!(builder.http2_adaptive_window, Some(http2_adaptive_window));

        let http2_max_header_list_size = 8192;
        let builder = builder.http2_max_header_list_size(http2_max_header_list_size);
        assert_eq!(
            builder.http2_max_header_list_size,
            Some(http2_max_header_list_size)
        );

        let local_address = Some(IpAddr::from([127, 0, 0, 2]));
        let builder = builder.local_address(local_address);
        assert_eq!(builder.local_address, local_address);

        let _ = builder.build([127, 0, 0, 1]);
    }

    #[test]
    fn debug_output() {
        let url = Url::parse("http://example.com:50051/foo").unwrap();
        let builder = EndpointTemplate::new(url.clone()).unwrap();

        let debug_output = format!("{builder:?}");
        assert_eq!(
            debug_output,
            "EndpointTemplate { url: \"http://example.com:50051/foo\", .. }"
        );
    }
}
