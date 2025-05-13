use std::{cmp::min, collections::BinaryHeap, net::IpAddr, ops::Deref, time::Duration};

use chrono::{DateTime, Timelike, Utc};
use tokio::{
    select,
    sync::{Mutex, Notify},
    time::sleep,
};

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
pub(crate) struct DelayedAddress(BackoffTracker, IpAddr);

impl DelayedAddress {
    fn failed_times(&self) -> u8 {
        self.0.failed_times()
    }
}

impl From<IpAddr> for DelayedAddress {
    fn from(ip_address: IpAddr) -> Self {
        DelayedAddress(BackoffTracker::from_failed_times(1), ip_address)
    }
}

impl Deref for DelayedAddress {
    type Target = IpAddr;

    fn deref(&self) -> &Self::Target {
        &self.1
    }
}

// Implementation using RwLock should be much nicer, but RwLock doesn't support CondVar.
// Although untested, my current assumption is that broken endpoints will be rare enough
// that using Mutex+CondVar will be faster than RwLock with some other strange synchronization.
#[derive(Default, Debug)] // todo: Maybe implement a custom Debug trait to avoid printing all the details.
pub(crate) struct BrokenEndpoints {
    addresses: Mutex<BinaryHeap<DelayedAddress>>,
    notifier: Notify,
}

impl BrokenEndpoints {
    /// Replaces the current list of broken endpoints with a new one.
    ///
    /// If the new list is not empty, it will notify the waiting threads.
    ///
    /// Note: While calling replace or swap on the `BrokenEndpoints` itself won't cause an error,
    /// it also won't notify the waiting threads about the change.
    pub(crate) async fn replace_with(&self, new: BinaryHeap<DelayedAddress>) {
        let mut guard = self.addresses.lock().await;
        *guard = new;
        if !guard.is_empty() {
            self.notifier.notify_one();
        }
    }

    pub(crate) async fn get_address(&self, address: IpAddr) -> Option<DelayedAddress> {
        self.addresses
            .lock()
            .await
            .iter()
            .find(|DelayedAddress(_, addr)| *addr == address)
            .copied()
    }

    pub(crate) async fn add_address(&self, address: IpAddr) {
        self.add_address_with_current_fail_count(address, 1).await;
    }

    pub(crate) async fn re_add_address(&self, address: DelayedAddress) {
        self.add_address_with_current_fail_count(*address, address.failed_times() + 1)
            .await;
    }

    async fn add_address_with_current_fail_count(&self, address: IpAddr, current_fail_count: u8) {
        let next_test_time = BackoffTracker::from_failed_times(current_fail_count);
        let mut guard = self.addresses.lock().await;
        guard.retain(|DelayedAddress(_, addr)| *addr != address);
        guard.push(DelayedAddress(next_test_time, address));
        self.notifier.notify_one();
    }

    /// Returns the next broken IP address that should be tested.
    ///
    /// Warning: This function will block until the next broken IP address is available or `max_wait_duration` has passed.
    pub(crate) async fn next_broken_ip_address(&self) -> DelayedAddress {
        // let max_end_wait = Utc::now() + max_wait_duration;
        loop {
            let mut guard = self.addresses.lock().await;

            if let Some(DelayedAddress(instant, _)) = guard.peek() {
                let now = Utc::now();
                if now < instant.next_test_time() {
                    let durr = (instant.next_test_time() - now)
                        .to_std()
                        .expect("behind an if check, so cannot fail");
                    drop(guard);
                    select! {
                        () = sleep(durr) => {}
                        () = self.notifier.notified() => {}
                    }
                } else {
                    let entry = guard.pop().expect(
                        "peeked an element while holding the same mutex guard, so pop cannot fail",
                    );
                    return entry;
                }
            } else {
                drop(guard);
                self.notifier.notified().await;
            }
        }
    }

    pub(crate) async fn addresses(
        &self,
    ) -> impl Deref<Target = BinaryHeap<DelayedAddress>> + Send + '_ {
        self.addresses.lock().await
    }
}

// todo: Maybe implement a custom Debug trait to avoid printing all the details.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
pub(crate) struct BackoffTracker(DateTime<Utc>);

impl BackoffTracker {
    pub fn from_failed_times(failed_times: u8) -> Self {
        let timestamp = Utc::now() + calculate_backoff(failed_times);
        BackoffTracker(set_retires(timestamp, failed_times))
    }

    pub fn next_test_time(&self) -> DateTime<Utc> {
        self.0
    }

    pub fn failed_times(&self) -> u8 {
        get_retires(self.0)
    }
}

// Note: The backoff strategy is very simple and might be inefficient.
// It's just a placeholder for a more sophisticated strategy.
// The current strategy is to wait 2^(failed_times-1) seconds before retrying.
// The maximum wait time is 2^6 = 64 seconds (~1 minutes).
fn calculate_backoff(failed_times: u8) -> Duration {
    let failed_times = min(failed_times.saturating_sub(1), 6);
    Duration::from_secs(2u64.pow(u32::from(failed_times)))
}

fn set_retires(timestamp: DateTime<Utc>, failed_times: u8) -> DateTime<Utc> {
    let failed_times = min(failed_times, 0xFF);
    // The last byte of the nanoseconds field is used to store the number of
    // failed times.
    //
    // note: The maximum valid value for nanoseconds is 1_999_999_999 witch is
    // 0x7735_93FF in hex. Since the last byte is already saturated, we can
    // overwrite it with the number of failed times without ever going out of
    // bounds.
    let nanos = timestamp.nanosecond() & 0xFFFF_FF00 | u32::from(failed_times);
    timestamp
        .with_nanosecond(nanos)
        .expect("couldn't failed to set nanos")
}

fn get_retires(timestamp: DateTime<Utc>) -> u8 {
    (timestamp.nanosecond() & 0xFF) as u8
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use chrono::TimeDelta;

    use super::*;

    #[rstest::rstest]
    fn retries(#[values(0, 1, 2, 4, 16, 255)] i: u8) {
        let datetime = Utc::now();
        let actual = set_retires(datetime, i);
        let time_diff = (actual - datetime).abs();

        assert_eq!(get_retires(actual), i);
        assert!(time_diff.num_nanoseconds().unwrap() <= 1000);
    }

    #[tokio::test]
    #[rstest::rstest]
    #[case(0, Duration::from_secs(1))]
    #[case(1, Duration::from_secs(1))]
    #[case(2, Duration::from_secs(2))]
    #[case(4, Duration::from_secs(8))]
    #[case(7, Duration::from_secs(64))]
    #[case(8, Duration::from_secs(64))]
    #[case(16, Duration::from_secs(64))]
    #[case(255, Duration::from_secs(64))]
    async fn backoff(#[case] fail_count: u8, #[case] expected: Duration) {
        let actual = calculate_backoff(fail_count);
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    #[rstest::rstest]
    #[case(0, TimeDelta::seconds(1))]
    #[case(1, TimeDelta::seconds(1))]
    #[case(2, TimeDelta::seconds(2))]
    #[case(4, TimeDelta::seconds(8))]
    #[case(7, TimeDelta::seconds(64))]
    #[case(8, TimeDelta::seconds(64))]
    #[case(16, TimeDelta::seconds(64))]
    #[case(255, TimeDelta::seconds(64))]
    async fn tracker(#[case] fail_count: u8, #[case] expected_delay: TimeDelta) {
        let start = Utc::now();
        let backoff_tracker = BackoffTracker::from_failed_times(fail_count);
        let time_diff = (backoff_tracker.next_test_time() - start).abs();

        assert_eq!(backoff_tracker.failed_times(), fail_count);
        // todo: Flaky test
        assert!((time_diff - expected_delay).abs() < TimeDelta::microseconds(10));
    }

    mod delayed_address {
        use super::*;

        #[test]
        fn from_ip_address() {
            let ip_address = IpAddr::from([127, 0, 0, 1]);
            let delayed_address = DelayedAddress::from(ip_address);
            assert_eq!(delayed_address.failed_times(), 1);
            assert_eq!(*delayed_address, ip_address);
        }

        #[test]
        fn failed_times() {
            let ip_address = IpAddr::from([127, 0, 0, 1]);
            let delayed_address = DelayedAddress(BackoffTracker::from_failed_times(3), ip_address);
            assert_eq!(delayed_address.failed_times(), 3);
            assert_eq!(*delayed_address, ip_address);
        }

        #[test]
        fn deref() {
            let ip_address = IpAddr::from([127, 0, 0, 1]);
            let delayed_address = DelayedAddress(BackoffTracker::from_failed_times(3), ip_address);
            assert_eq!(*delayed_address, ip_address);
        }
    }

    mod broken_endpoints {
        use std::{
            net::{Ipv4Addr, Ipv6Addr},
            sync::Arc,
            vec,
        };

        use tokio::{task::yield_now, time::Instant};

        use super::*;

        #[tokio::test]
        async fn get_address() {
            let broken_endpoints = BrokenEndpoints::default();
            broken_endpoints
                .add_address(Ipv4Addr::LOCALHOST.into())
                .await;
            broken_endpoints
                .add_address(Ipv6Addr::LOCALHOST.into())
                .await;

            let actual = broken_endpoints
                .get_address(Ipv4Addr::LOCALHOST.into())
                .await
                .unwrap();

            assert_eq!(actual.failed_times(), 1);
            assert_eq!(*actual, IpAddr::from(Ipv4Addr::LOCALHOST));

            assert!(broken_endpoints
                .get_address([127, 0, 0, 2].into())
                .await
                .is_none());
        }

        #[tokio::test]
        async fn add_address() {
            let broken_endpoints = BrokenEndpoints::default();
            let address = Ipv4Addr::LOCALHOST.into();
            broken_endpoints.add_address(address).await;

            let actual = broken_endpoints.get_address(address).await.unwrap();
            assert_eq!(actual.failed_times(), 1);
        }

        #[tokio::test]
        async fn re_add_address() {
            let broken_endpoints = BrokenEndpoints::default();
            let address = Ipv4Addr::LOCALHOST.into();
            broken_endpoints.add_address(address).await;
            let first = broken_endpoints.get_address(address).await.unwrap();
            broken_endpoints.re_add_address(first).await;
            let actual = broken_endpoints.get_address(address).await.unwrap();
            assert_eq!(actual.failed_times(), 2);
        }

        #[tokio::test]
        async fn addresses() {
            let broken_endpoints = BrokenEndpoints::default();
            let address1 = Ipv4Addr::from([10, 0, 0, 1]).into();
            let address2 = Ipv4Addr::from([192, 168, 1, 1]).into();

            broken_endpoints.add_address(address1).await;
            broken_endpoints.add_address(address2).await;

            let guard = broken_endpoints.addresses().await;
            let addresses = guard.iter().collect::<Vec<_>>();

            assert!(addresses
                .iter()
                .all(|address| { address.failed_times() == 1 }));

            let mut actual = addresses.into_iter().map(Deref::deref).collect::<Vec<_>>();
            actual.sort();
            assert_eq!(actual, vec![&address1, &address2]);
        }

        #[tokio::test]
        async fn replace_with() {
            let broken_endpoints = BrokenEndpoints::default();
            let address1 = Ipv4Addr::from([10, 0, 0, 1]).into();
            let address2 = Ipv4Addr::from([192, 168, 1, 1]).into();

            broken_endpoints.add_address(address1).await;
            broken_endpoints.add_address(address2).await;

            let new_addresses = BinaryHeap::from(vec![DelayedAddress::from(IpAddr::from(
                Ipv4Addr::LOCALHOST,
            ))]);
            broken_endpoints.replace_with(new_addresses).await;

            let guard = broken_endpoints.addresses().await;
            assert_eq!(guard.len(), 1);
            assert_eq!(guard.peek().unwrap().failed_times(), 1);
        }

        #[tokio::test]
        // Note: Can we properly test it without using a real clock and waiting for seconds?
        async fn next_broken_ip_address() {
            let broken_endpoints = Arc::new(BrokenEndpoints::default());
            let address1 = Ipv4Addr::from([10, 0, 0, 1]).into();

            let background = {
                let broken_endpoints = broken_endpoints.clone();
                tokio::spawn(async move {
                    let start = Instant::now();
                    let next_broken_ip = broken_endpoints.next_broken_ip_address().await;
                    let duration = start.elapsed();
                    assert_eq!(next_broken_ip.failed_times(), 1);
                    assert!(duration >= Duration::from_secs(1));
                })
            };
            yield_now().await;
            broken_endpoints.add_address(address1).await;

            println!("Waiting for background task to finish");

            background.await.unwrap();
        }
    }
}
