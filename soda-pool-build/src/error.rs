pub(crate) type BuilderResult<R> = std::result::Result<R, BuilderError>;

#[derive(Debug)]
pub enum BuilderError {
    UnexpectedStructure,
    MissingConfiguration(String),
    IoError(std::io::Error),
    SynError(syn::Error),
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl From<std::io::Error> for BuilderError {
    fn from(err: std::io::Error) -> Self {
        BuilderError::IoError(err)
    }
}

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
