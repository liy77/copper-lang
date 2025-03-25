use std::{path::PathBuf, process::Command};
use serde::{Deserialize, Serialize};
use crate::class;

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

class!(
    self {
        #[derive(Debug)]
        pub(crate) class Extractor {
            public {
                #[doc = "The metadata extracted from the Cargo.lock file."]
                metadata: Metadata

                :self {
                    #[doc = "Fetches the metadata from the Cargo.lock file."]
                    fetch_metadata(): Metadata {
                        let out = Command::new("cargo")
                        .arg("metadata")
                        .arg("--format-version")
                        .arg("1")
                        .output()
                        .expect("Failed to execute cargo metadata");

                        let metadata: Metadata = serde_json::from_slice(&out.stdout).expect("Failed to parse metadata");

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

                :companion {
                    #[doc = "Creates a new instance of the Extractor class."]
                    new(): Self {
                        Self {
                            metadata: Metadata {
                                packages: vec![],
                            }
                        }
                    }
                }
            }
        }
    }
);

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
