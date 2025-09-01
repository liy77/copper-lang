use std::{path::PathBuf, process::Command};
use serde::{Deserialize, Serialize};

/// A package extracted from a Cargo.lock file.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct Package {
    pub name: String,
    pub version: String,
    pub source: Option<String>,
    pub manifest_path: PathBuf,
}

/// Metadata extracted from a Cargo.lock file.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct Metadata {
    pub packages: Vec<Package>,
}

    pub(crate) struct Extractor {
        pub metadata: Metadata,
    }

    impl Extractor {
        /// Creates a new instance of the Extractor struct.
        pub fn new() -> Self {
            Self {
                metadata: Metadata {
                    packages: vec![],
                },
            }
        }

        /// Fetches the metadata from the Cargo.lock file.
        pub fn fetch_metadata(&mut self) -> Metadata {
            let out = Command::new("cargo")
                .arg("metadata")
                .arg("--format-version")
                .arg("1")
                .output()
                .expect("Failed to execute cargo metadata");

            let metadata: Metadata =
                serde_json::from_slice(&out.stdout).expect("Failed to parse metadata");

            for package in metadata.clone().packages {
                let manifest_path = PathBuf::from(&package.manifest_path);
                let name = package.name;
                let version = package.version;
                let source = package.source;

                self.metadata.packages.push(Package {
                    name,
                    version,
                    source,
                    manifest_path,
                });
            }

            metadata
        }
    }


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetch_metadata() {
        let mut extractor = Extractor::new();
        let metadata = extractor.fetch_metadata();

        println!("{:?}", metadata);
        assert!(!metadata.packages.is_empty());
    }
}
