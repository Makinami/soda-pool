use std::time::Duration;

pub trait RetryPolicy {
    fn should_retry(err: &tonic::Status, tries: usize) -> RetryCheckResult;
}

#[derive(Debug, PartialEq)]
pub enum ServerStatus {
    Alive,
    Dead,
}

#[derive(Debug, PartialEq)]
pub enum RetryTime {
    DoNotRetry,
    Immediately,
    After(Duration),
}

#[derive(Debug, PartialEq)]
pub struct RetryCheckResult(pub ServerStatus, pub RetryTime);

/// Default retry policy
///
/// This policy retries the request immediately and marks the server as dead if
/// seems to be a network error or otherwise a problem originating from the
/// client library rather than the server. I also don't have a limit on the
/// number of retries and will continue as long as there is still an alive
/// connection remaining.
#[derive(Debug)]
pub struct DefaultRetryPolicy;

impl RetryPolicy for DefaultRetryPolicy {
    fn should_retry(err: &tonic::Status, _tries: usize) -> RetryCheckResult {
        // Initial tests suggest that source of the error is set only when it comes
        // from the client library (e.g. connection refused) and not the server.
        if std::error::Error::source(err).is_some() {
            RetryCheckResult(ServerStatus::Dead, RetryTime::Immediately)
        } else {
            RetryCheckResult(ServerStatus::Alive, RetryTime::DoNotRetry)
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
        assert_eq!(result, RetryCheckResult(ServerStatus::Alive, RetryTime::DoNotRetry));
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
        assert_eq!(result, RetryCheckResult(ServerStatus::Dead, RetryTime::Immediately));
    }
}
