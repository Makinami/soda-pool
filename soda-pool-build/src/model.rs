use syn::{Ident, Type};

#[derive(Debug)]
pub(crate) struct GrpcClientFile {
    pub(crate) file_module: Ident,
    pub(crate) client_modules: Vec<GrpcClientModule>,
}

#[derive(Debug)]
pub(crate) struct GrpcClientModule {
    pub(crate) name: Ident,
    pub(crate) clients: Vec<GrpcClientImpl>,
}

#[derive(Debug)]
pub(crate) struct GrpcClientImpl {
    pub(crate) name: Ident,
    pub(crate) methods: Vec<GrpcClientMethod>,
}

#[derive(Debug)]
pub(crate) struct GrpcClientMethod {
    pub(crate) name: Ident,
    pub(crate) request_type: Type,
    pub(crate) response_type: Type,
}
