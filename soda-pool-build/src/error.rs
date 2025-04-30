use std::borrow::Cow;

pub(crate) type BuilderResult<R> = std::result::Result<R, BuilderError>;

#[derive(Debug)]
pub enum BuilderError {
    UnexpectedStructure,
    MissingConfiguration(String),
    IoError(std::io::Error),
    SynError(syn::Error),
}

impl From<std::io::Error> for BuilderError {
    fn from(err: std::io::Error) -> Self {
        BuilderError::IoError(err)
    }
}

impl From<syn::Error> for BuilderError {
    fn from(err: syn::Error) -> Self {
        BuilderError::SynError(err)
    }
}

impl BuilderError {
    pub(crate) fn missing_configuration(key: impl Into<Cow<'static, str>>) -> Self {
        BuilderError::MissingConfiguration(key.into().into_owned())
    }
}
