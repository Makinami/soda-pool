use tonic::{Extensions, IntoRequest, metadata::MetadataMap};

#[derive(Clone)]
pub struct CloneableRequest<T>
where
    T: Clone,
{
    metadata: MetadataMap,
    message: T,
    extensions: Extensions,
}

impl<T> From<T> for CloneableRequest<T>
where
    T: Clone,
{
    fn from(message: T) -> Self {
        Self {
            metadata: MetadataMap::new(),
            message,
            extensions: Extensions::default(),
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
            extensions: Extensions::default(),
        }
    }

    pub fn with_metadata(mut self, metadata: MetadataMap) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn with_extensions(
        mut self,
        extensions: Extensions,
    ) -> Self {
        self.extensions = extensions;
        self
    }
}

impl<T> IntoRequest<T> for CloneableRequest<T>
where
    T: Clone,
{
    fn into_request(self) -> tonic::Request<T> {
        tonic::Request::from_parts(self.metadata, self.extensions, self.message)
    }
}
