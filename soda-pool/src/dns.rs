use std::{io::Result, net::IpAddr};

#[cfg(not(any(test, feature = "mock-dns")))]
pub use std::net::ToSocketAddrs;

#[cfg(any(test, feature = "mock-dns"))]
pub use mock_net::ToSocketAddrs;

pub fn resolve_domain(domain: &str) -> Result<impl Iterator<Item = IpAddr>> {
    Ok((domain, 0).to_socket_addrs()?.map(|addr| addr.ip()))
}

#[cfg(any(test, feature = "mock-dns"))]
#[cfg_attr(coverage_nightly, coverage(off))]
pub mod mock_net {
    use std::{io, net::SocketAddr, vec};

    use std::sync::{LazyLock, RwLock};

    type ToSocketAddrsFn = dyn Fn(&str, u16) -> io::Result<Vec<SocketAddr>> + Send + Sync;

    static DNS_RESULT: LazyLock<RwLock<Box<ToSocketAddrsFn>>> =
        LazyLock::new(|| RwLock::new(Box::new(|_, _| Ok(vec![]))));

    pub trait ToSocketAddrs {
        type Iter: Iterator<Item = SocketAddr>;

        fn to_socket_addrs(&self) -> io::Result<Self::Iter>;
    }

    impl ToSocketAddrs for (&str, u16) {
        type Iter = vec::IntoIter<SocketAddr>;
        fn to_socket_addrs(&self) -> io::Result<vec::IntoIter<SocketAddr>> {
            (*DNS_RESULT
                .read()
                .expect("failed to acquire read lock on DNS_RESULT"))(self.0, self.1)
            .map(IntoIterator::into_iter)
        }
    }

    #[allow(dead_code)]
    pub fn set_socket_addrs(func: Box<ToSocketAddrsFn>) {
        *DNS_RESULT
            .write()
            .expect("failed to acquire write lock on DNS_RESULT") = func;
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use serial_test::serial;

    use super::*;
    use std::{io, net::SocketAddr, str::FromStr};

    #[test]
    #[serial]
    fn can_mock_address_resolution() {
        let addresses = vec![
            IpAddr::from_str("128.0.0.1").unwrap(),
            IpAddr::from_str("129.0.0.1").unwrap(),
            IpAddr::from_str("::2").unwrap(),
            IpAddr::from_str("::3").unwrap(),
        ];

        {
            let sockets = addresses
                .iter()
                .map(|ip| SocketAddr::new(*ip, 0))
                .collect::<Vec<_>>();
            mock_net::set_socket_addrs(Box::new(move |_, _| Ok(sockets.clone())));
        }

        assert_eq!(
            resolve_domain("localhost").unwrap().collect::<Vec<_>>(),
            addresses
        );
    }

    #[test]
    #[serial]
    fn forwards_errors() {
        mock_net::set_socket_addrs(Box::new(|_, _| {
            Err(io::Error::new(io::ErrorKind::Other, "mock error"))
        }));

        let result = resolve_domain("localhost");
        assert!(result.is_err());
        let Err(error) = result else { unreachable!() };
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(error.to_string(), "mock error");
    }
}
