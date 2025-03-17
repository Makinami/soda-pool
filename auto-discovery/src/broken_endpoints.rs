use std::{
    collections::BinaryHeap,
    mem::replace,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

type DelayedAddress = (Instant, IpAddr);

// Implementation using RwLock should be much nicer, but RwLock doesn't support CondVar.
// Although untested, my current assumption is that broken endpoints will be rare enough
// that using Mutex+CondVar will be faster than RwLock with some other strange synchronization.
#[derive(Default)]
pub(crate) struct BrokenEndpoints {
    addresses: Mutex<BinaryHeap<DelayedAddress>>,
    condvar: std::sync::Condvar,
}

impl BrokenEndpoints {
    /// Replaces the current list of broken endpoints with a new one.
    ///
    /// If the new list is not empty, it will notify the waiting threads.
    ///
    /// Note: While calling replace or swap on the BrokenEndpoints itself won't cause an error,
    /// it also won't notify the waiting threads about the change.
    pub(crate) fn replace_with(&self, new: BinaryHeap<DelayedAddress>) {
        let has_broken = !new.is_empty();
        let mut guard = self.addresses.lock().unwrap();
        let _ = replace(&mut *guard, new);
        if has_broken {
            self.condvar.notify_one();
        }
    }

    pub(crate) fn get_entry(&self, address: IpAddr) -> Option<DelayedAddress> {
        let guard = self.addresses.lock().unwrap();
        guard.iter().find(|(_, addr)| *addr == address).cloned()
    }

    pub(crate) fn add_address(&self, address: IpAddr) {
        let mut guard = self.addresses.lock().unwrap();
        guard.push((Instant::now() + Duration::from_secs(1), address));
        self.condvar.notify_one();
    }

    /// Returns the next broken IP address that should be tested.
    ///
    /// Warning: This function will block until the next broken IP address is available or max_wait_duration has passed.
    pub(crate) fn next_broken_ip_address(&self, max_wait_duration: Duration) -> Option<IpAddr> {
        let max_end_wait = Instant::now() + max_wait_duration;
        loop {
            let mut guard = self.addresses.lock().unwrap();
            let now = Instant::now();
            if let Some((instant, _)) = guard.peek() {
                if now < *instant {
                    let durr = *instant - now;
                    let result = self
                        .condvar
                        .wait_timeout_while(guard, durr, |endpoints| endpoints.is_empty())
                        .unwrap();
                    if result.1.timed_out() {
                        return None;
                    }
                } else {
                    return Some(guard.pop().unwrap().1);
                }
            } else if now < max_end_wait {
                let result = self
                    .condvar
                    .wait_timeout_while(guard, max_end_wait - Instant::now(), |endpoints| {
                        endpoints.is_empty()
                    })
                    .unwrap();
                if result.1.timed_out() {
                    return None;
                }
            } else {
                return None;
            }
        }
    }
}
