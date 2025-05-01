use std::{error::Error, fmt::Display};

pub(crate) type BuilderResult<R> = std::result::Result<R, BuilderError>;

/// Possible errors during pooled client generation.
#[derive(Debug)]
pub enum BuilderError {
    /// Unexpected structure of basic gRPC client file.
    ///
    /// This error occurs when basic gRPC client code does not match structure expected from supported tonic-build version.
    UnexpectedStructure,

    /// Missing configuration.
    ///
    /// This error occurs when [`SodaPoolBuilder::build_pools`](crate::SodaPoolBuilder::build_pools) method is called before all required settings are provided.
    MissingConfiguration(String),

    /// I/O error.
    ///
    /// This wraps any I/O error that occurs during file operations.
    IoError(std::io::Error),

    /// Syn error.
    ///
    /// This wraps any error that occurs during parsing Rust code using the `syn` crate.
    SynError(syn::Error),
}

#[doc(hidden)]
#[cfg_attr(coverage_nightly, coverage(off))]
impl From<std::io::Error> for BuilderError {
    fn from(err: std::io::Error) -> Self {
        BuilderError::IoError(err)
    }
}

#[doc(hidden)]
#[cfg_attr(coverage_nightly, coverage(off))]
impl From<syn::Error> for BuilderError {
    fn from(err: syn::Error) -> Self {
        BuilderError::SynError(err)
    }
}

impl BuilderError {
    pub(crate) fn missing_configuration(key: &'static str) -> Self {
        BuilderError::MissingConfiguration(key.to_string())
    }
}

impl Display for BuilderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuilderError::UnexpectedStructure => {
                write!(f, "unexpected structure of basic gRPC client file")
            }
            BuilderError::MissingConfiguration(key) => write!(f, "missing configuration: {}", key),
            BuilderError::IoError(err) => write!(f, "I/O error: {}", err),
            BuilderError::SynError(err) => write!(f, "syn error: {}", err),
        }
    }
}

impl Error for BuilderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            BuilderError::IoError(err) => Some(err),
            BuilderError::SynError(err) => Some(err),
            _ => None,
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn test_missing_configuration() {
        let error = BuilderError::missing_configuration("test_key");
        if let BuilderError::MissingConfiguration(key) = error {
            assert_eq!(key, "test_key");
        } else {
            panic!("Expected MissingConfiguration error");
        }
    }
}
