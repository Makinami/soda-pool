use std::{
    fs, path::{Path, PathBuf}
};

use parser::parse_grpc_client_file;

mod parser;
mod error;
mod generator;
mod model;

pub fn configure() -> TonicPoolBuilder {
    TonicPoolBuilder { dir: None }
}

pub struct TonicPoolBuilder {
    dir: Option<PathBuf>,
}

type TonicPoolBuilderError = ();

impl TonicPoolBuilder {
    pub fn dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.dir = Some(dir.as_ref().to_path_buf());
        self
    }

    pub fn build_pools(&self, services: &[impl AsRef<str>]) -> Result<(), TonicPoolBuilderError> {
        let dir = self.dir.as_ref().ok_or(())?;

        services.into_iter().try_for_each(|service| {
            let service_filename = if service.as_ref().ends_with(".rs") {
                service.as_ref().to_string()
            } else {
                format!("{}.rs", service.as_ref())
            };

            let service_file = dir.join(&service_filename);
            let service_file_structure = parse_grpc_client_file(&service_file).unwrap();


            let output =  service_file_structure.generate_pooled_version();
            let file = syn::parse2(output).unwrap();
            let formatted = prettyplease::unparse(&file);

            let output_file = service_file.with_file_name(service_file.file_stem().unwrap().to_str().unwrap().to_owned() + "_pool.rs");

            fs::write(output_file, formatted).unwrap();

            Ok(())
        })
    }
}
