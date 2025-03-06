use std::sync::Arc;

use tonic::{metadata::MetadataMap, Extensions, IntoRequest};

type ExtensionsGeneratorFn = dyn Fn() -> Extensions + Send + Sync;

#[derive(Clone)]
pub struct CloneableRequest<T>
where
    T: Clone,
{
    metadata: MetadataMap,
    message: T,
    extensions_generator: Arc<ExtensionsGeneratorFn>,
}

impl<T> From<T> for CloneableRequest<T>
where
    T: Clone,
{
    fn from(message: T) -> Self {
        Self {
            metadata: MetadataMap::new(),
            message,
            extensions_generator: Arc::new(Extensions::default),
        }
    }
}

impl<T> CloneableRequest<T>
where
    T: Clone,
{
    pub fn new(message: T) -> Self {
        Self {
            metadata: MetadataMap::new(),
            message,
            extensions_generator: Arc::new(Extensions::default),
        }
    }

    pub fn with_metadata(mut self, metadata: MetadataMap) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn with_extensions_generator(
        mut self,
        extensions_generator: Arc<ExtensionsGeneratorFn>,
    ) -> Self {
        self.extensions_generator = extensions_generator;
        self
    }
}

impl<T> IntoRequest<T> for CloneableRequest<T>
where
    T: Clone,
{
    fn into_request(self) -> tonic::Request<T> {
        tonic::Request::from_parts(self.metadata, (self.extensions_generator)(), self.message)
    }
}
