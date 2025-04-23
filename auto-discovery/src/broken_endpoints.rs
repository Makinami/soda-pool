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

    pub(crate) async fn readd_address(&self, address: DelayedAddress) {
        self.add_address_with_current_fail_count(*address, address.failed_times())
            .await;
    }

    pub(crate) async fn add_address_with_current_fail_count(
        &self,
        address: IpAddr,
        current_fail_count: u8,
    ) {
        let next_test_time = BackoffTracker::from_failed_times(current_fail_count);
        let mut guard = self.addresses.lock().await;
        if !guard.iter().any(|DelayedAddress(_, addr)| *addr == address) {
            guard.push(DelayedAddress(next_test_time, address));
            self.notifier.notify_one();
        }
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

    pub(crate) async fn addresses(&self) -> impl Deref<Target = BinaryHeap<DelayedAddress>> + Send {
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
    let failed_times = min(failed_times - 1, 6);
    Duration::from_secs(2u64.pow(u32::from(failed_times)))
}

fn set_retires(timestamp: DateTime<Utc>, failed_times: u8) -> DateTime<Utc> {
    let failed_times = min(failed_times, 0xFF);
    // The last byte of the nanoseconds field is used to store the number of failed times.
    // To make sure we do not go above the maximum nanoseconds value (2_000_000_000), we subtract 0x100.
    let nanos = (timestamp.nanosecond() & 0xFFFF_FF00 | u32::from(failed_times)) - 0x100;
    timestamp
        .with_nanosecond(nanos)
        .expect("couldn't failed to set nanos")
}

fn get_retires(timestamp: DateTime<Utc>) -> u8 {
    (timestamp.nanosecond() & 0xFF) as u8
}
