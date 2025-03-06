use std::sync::Arc;

use tonic::{metadata::MetadataMap, Extensions};

type ExtensionsGeneratorFn = dyn Fn() -> Extensions + Send + Sync;

#[derive(Clone)]
pub struct RequestGenerator<T>
where
    T: Clone,
{
    metadata: MetadataMap,
    message: T,
    extensions_generator: Arc<ExtensionsGeneratorFn>,
}

impl<T> From<T> for RequestGenerator<T>
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

impl<T> RequestGenerator<T>
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

pub trait GeneratesRequest<T> {
    fn generate_request(&self) -> impl tonic::IntoRequest<T>;
}

impl<T> GeneratesRequest<T> for RequestGenerator<T>
where
    T: Clone,
{
    fn generate_request(&self) -> impl tonic::IntoRequest<T> {
        tonic::Request::from_parts(self.metadata.clone(), (self.extensions_generator)(), self.message.clone())
    }
}
