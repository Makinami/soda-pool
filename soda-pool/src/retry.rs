use std::time::Duration;

/// Retry policy for the request.
///
/// This trait is used to determine whether a request should be retried or not
/// based on the error returned by the server and number of attempts. It also
/// provides information about the status of the server and the time to wait
/// before retrying the request.
///
/// # Note
///
/// If there are no more alive connections in the pool, the request will not be
/// retried and the last error will be returned to the caller.
/// **The retry policy cannot be used to override this behavior.**
pub trait RetryPolicy {
    /// Called to determine the status of the server and whether the request
    /// should be retried or not.
    fn should_retry(err: &tonic::Status, tries: usize) -> RetryPolicyResult;
}

/// Status of the server.
#[derive(Debug, PartialEq)]
pub enum ServerStatus {
    /// The server should be treated as alive.
    Alive,

    /// The server should be treated as dead.
    Dead,
}

/// Retry time of the failed request.
#[derive(Debug, PartialEq)]
pub enum RetryTime {
    /// Do not retry the request.
    DoNotRetry,

    /// Retry the request immediately.
    Immediately,

    /// Retry the request after a certain delay.
    After(Duration),
}

/// Result of the retry policy.
///
/// [`RetryPolicy::should_retry`] returns this type to indicate the status of
/// the server and the time to wait before retrying the request.
pub type RetryPolicyResult = (ServerStatus, RetryTime);

/// Default retry policy.
///
/// This policy retries the request immediately and marks the server as dead if
/// it seems to be a network error or otherwise a problem originating from the
/// client library rather than the server. I also don't have a limit on the
/// number of retries and will continue as long as there is still an alive
/// connection remaining.
#[derive(Debug)]
pub struct DefaultRetryPolicy;

impl RetryPolicy for DefaultRetryPolicy {
    fn should_retry(err: &tonic::Status, _tries: usize) -> RetryPolicyResult {
        // Initial tests suggest that source of the error is set only when it comes
        // from the client library (e.g. connection refused) and not the server.
        if std::error::Error::source(err).is_some() {
            (ServerStatus::Dead, RetryTime::Immediately)
        } else {
            (ServerStatus::Alive, RetryTime::DoNotRetry)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Status;

    #[test]
    fn test_default_retry_policy_alive() {
        let err = Status::new(tonic::Code::Unknown, "test error");
        let result = DefaultRetryPolicy::should_retry(&err, 1);
        assert_eq!(result, (ServerStatus::Alive, RetryTime::DoNotRetry));
    }

    #[test]
    fn test_default_retry_policy_dead() {
        #[derive(Debug)]
        struct TestError;
        impl std::fmt::Display for TestError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "Test error")
            }
        }
        impl std::error::Error for TestError {}

        let err = Status::from_error(Box::new(TestError));
        let result = DefaultRetryPolicy::should_retry(&err, 1);
        assert_eq!(result, (ServerStatus::Dead, RetryTime::Immediately));
    }
}
