use std::{
    fs,
    path::{Path, PathBuf},
};

use error::{BuilderError, BuilderResult};
use parser::parse_grpc_client_file;

pub mod error;
mod generator;
mod model;
mod parser;

#[must_use]
pub fn configure() -> TonicPoolBuilder {
    TonicPoolBuilder { dir: None }
}

#[derive(Debug)]
pub struct TonicPoolBuilder {
    dir: Option<PathBuf>,
}

impl TonicPoolBuilder {
    #[must_use]
    pub fn dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.dir = Some(dir.as_ref().to_path_buf());
        self
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn build_pools(&self, services: &[impl AsRef<str>]) -> BuilderResult<()> {
        let dir = self
            .dir
            .as_ref()
            .ok_or_else(|| BuilderError::missing_configuration("dir"))?;

        services.iter().try_for_each(|service| {
            let service_filename = if Path::new(service.as_ref())
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
            {
                service.as_ref().to_string()
            } else {
                format!("{}.rs", service.as_ref())
            };

            let service_file = dir.join(&service_filename);
            let service_file_structure = parse_grpc_client_file(&service_file)?;

            let output = service_file_structure.generate_pooled_version();
            let file = syn::parse2(output)?;
            let formatted = prettyplease::unparse(&file);

            let output_file = {
                let mut filename = service_file
                .file_stem()
                .expect("`service_file` is already certain to hold path to a file by previous check")
                .to_owned();
                filename.push("_pool.rs");
                service_file.with_file_name(filename)
            };

            fs::write(output_file, formatted).unwrap();

            Ok(())
        })
    }
}
