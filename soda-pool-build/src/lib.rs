#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

//! This crate generates pooled gRPC clients (that uses
//! [soda-pool](https://docs.rs/soda-pool) crate) from the original gRPC clients
//! generated by [tonic-build](https://docs.rs/tonic-build) crate.
//!
//! # Usage
//!
//! ```no_run
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     soda_pool_build::configure()
//!         .dir("./protobuf.gen/src")
//!         .build_all_clients()?;
//!     Ok(())
//! }
//! ```
//!
//! # Output Example
//!
//! Please see
//! [example/protobuf.gen](https://github.com/Makinami/soda-pool/blob/main/example/protobuf.gen/src/health_pool.rs)
//! for an example of generated file and
//! [example/client](https://github.com/Makinami/soda-pool/blob/main/example/client/src/main.rs)
//! for its usage.
//!

use std::{
    fs,
    path::{Path, PathBuf},
};

mod error;
pub use error::*;

mod parser;
use parser::parse_grpc_client_file;

mod generator;
mod model;

/// Create a default [`SodaPoolBuilder`].
// Emulates the `tonic_build` interface.
#[must_use]
pub fn configure() -> SodaPoolBuilder {
    SodaPoolBuilder { dir: None }
}

/// Pooled gRPC clients generator.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SodaPoolBuilder {
    dir: Option<PathBuf>,
}

impl SodaPoolBuilder {
    /// Create a new [`SodaPoolBuilder`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the input/output directory of gRPC clients' files.
    pub fn dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.dir = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Build pooled gRPC clients.
    ///
    /// Generate pooled version of gRPC clients from the specified files.
    /// `services` should be a list of files (with or without `.rs` extension)
    /// containing the original gRPC client code. Files will be searched in the
    /// directory specified by `dir`. For each input file, a new file will be
    /// created with the same name but with `_pool` suffix.
    ///
    /// # Errors
    /// Will return [`BuilderError`] on any errors encountered during pooled clients generation.
    #[allow(clippy::missing_panics_doc)]
    pub fn build_clients(
        &self,
        services: impl IntoIterator<Item = impl AsRef<Path>>,
    ) -> BuilderResult<()> {
        let dir = self
            .dir
            .as_ref()
            .ok_or_else(|| BuilderError::missing_configuration("dir"))?;

        services.into_iter().try_for_each(|service| {
            let service_filename = service.as_ref().with_extension("rs");

            let service_file = if service_filename.is_relative() { dir.join(&service_filename) } else { service_filename };
            let service_file_structure = parse_grpc_client_file(&service_file)?;

            if service_file_structure.client_modules.iter().all(|module| module.clients.is_empty()) {
                return Err(BuilderError::GrpcClientNotFound);
            }

            let output = service_file_structure.generate_pooled_version();
            let file = syn::parse2(output)?;
            let formatted = format!(
                "// This file is @generated by soda-pool-build.\n{}",
                prettyplease::unparse(&file),
            );

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

    /// Build pooled gRPC clients for all files in the specified directory.
    ///
    /// This method will search for all files in the directory specified by
    /// `dir` and attempt to build pooled gRPC clients for each file. It will
    /// skip any files that do not contain any recognized gRPC clients.
    ///
    /// # Errors
    /// Will return [`BuilderError`] on any errors encountered during pooled clients generation.
    #[allow(clippy::missing_panics_doc)]
    pub fn build_all_clients(&self) -> BuilderResult<()> {
        let dir = self
            .dir
            .as_ref()
            .ok_or_else(|| BuilderError::missing_configuration("dir"))?;

        let entries = fs::read_dir(dir)?;

        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }

            if entry
                .path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
            {
                match self.build_clients([entry
                    .path()
                    .file_name()
                    .expect("We have checked that this is a file so it must have a name")])
                {
                    Ok(()) | Err(BuilderError::GrpcClientNotFound) => {}
                    Err(e) => return Err(e),
                }
            }
        }

        Ok(())
    }
}
