use colored::Colorize;
use once_cell::sync::Lazy;
use serde_json::Value;
use std::path::Path;

use crate::{
    cargo,
    cforge::{fetch::check_version_exists, vprint},
};

#[derive(Clone)]
pub enum PropertyKind {
    String,
    Number,
    Boolean,
    Array,
    Object,
}

macro_rules! validate {
    ($struct: expr, $field: ident, $to_validate: expr) => {
        let err_msg = "Invalid propertie value".to_string();

        if let Some($field) = $to_validate {
            $struct.$field = $field;
        } else {
            panic!("{}", err_msg);
        }
    };
}

#[derive(Clone, Copy)]
pub enum DepKind {
    NormalOnlyVersion,
    NormalJson,
    DevOnlyVersion,
    DevJson,
    BuildOnlyVersion,
    BuildJson,
}

enum MapDepMode {
    Normal,
    Dev,
    Build,
}

pub struct Dependency {
    pub name: String,
    pub version: String,
    pub features: Vec<String>,
    pub kind: DepKind,
    pub git: Option<String>,
    pub branch: Option<String>,
    pub tag: Option<String>,
    /// Local path dependency (`{ path = "..." }`).
    pub path: Option<String>,
}

pub struct Properties<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub edition: u64,
    pub kind: String,
    pub bin_name: Option<String>,
    pub lib_name: Option<String>,
    pub lto: Option<bool>,
    pub dependencies: Vec<Dependency>,
}

const METADATA: Lazy<cargo::Metadata> = Lazy::new(|| {
    let mut extractor = cargo::Extractor::new();
    extractor.fetch_metadata()
});

/// Normalize a path dependency relative to the project root so that Cargo
/// running inside `dist/rust/` can resolve it reliably. If the target directory
/// is itself a Copper package (`properties.kson` with a `dist/rust` output),
/// point to its `dist/rust` crate directory.
fn normalize_dep_path(raw_path: &str) -> String {
    normalize_dep_path_in_dir(raw_path, &std::env::current_dir().unwrap_or_default())
}

fn normalize_dep_path_in_dir(raw_path: &str, base_dir: &Path) -> String {
    let p = Path::new(raw_path);
    // Built-in std crate paths (`__copper__/std/...`) are already relative to `dist/rust`.
    if raw_path.starts_with("__copper__/") {
        return raw_path.to_string();
    }
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        base_dir.join(p)
    };
    let canon = abs.canonicalize().unwrap_or(abs);
    // If target is a pure Copper package (has `properties.kson` and no `Cargo.toml`),
    // ensure it is transpiled and point Cargo to its `dist/rust` directory.
    let target = if canon.join("properties.kson").exists() && !canon.join("Cargo.toml").exists() {
        crate::cforge::ensure_copper_dep_built(&canon);
        canon.join("dist").join("rust")
    } else {
        canon
    };
    let s = target.to_string_lossy().replace('\\', "/");
    s.strip_prefix("//?/").unwrap_or(&s).to_string()
}

// Map dependencies from properties.kson to Cargo.toml
async fn map_deps<'a>(
    props: &mut Properties<'a>,
    deps: &'a Value,
    mode: MapDepMode,
    base_dir: &Path,
) {
    let Some(deps) = deps.as_object() else {
        return;
    };

    let kind = match mode {
        MapDepMode::Normal => {
            vprint!("📩 Installing normal dependencies");
            (DepKind::NormalOnlyVersion, DepKind::NormalJson)
        }
        MapDepMode::Dev => {
            vprint!("🧑‍💻 Installing dev dependencies");
            (DepKind::DevOnlyVersion, DepKind::DevJson)
        }
        MapDepMode::Build => {
            vprint!("🔨 Installing build dependencies");
            (DepKind::BuildOnlyVersion, DepKind::BuildJson)
        }
    };

    for (name, value) in deps {
        vprint!(
            "🔍 Debug: Processing dependency '{}' with value: {}",
            name,
            serde_json::to_string(value).unwrap()
        );

        if value.is_string() {
            let version = value.as_str().unwrap().trim_matches('"').to_string();

            if let Some(p) = version.strip_prefix("path:") {
                let norm = normalize_dep_path_in_dir(p.trim(), base_dir);
                props.dependencies.push(Dependency {
                    name: name.to_string(),
                    version: "0.1.0".to_string(),
                    features: Vec::new(),
                    kind: kind.0,
                    git: None,
                    branch: None,
                    tag: None,
                    path: Some(norm.clone()),
                });
                println!("📦 {} {} {}", name.green(), "=>".yellow(), norm.black());
                continue;
            }

            if !version.is_empty() && version != "*" && version != "latest" {
                props.dependencies.push(Dependency {
                    name: name.to_string(),
                    version: version.clone(),
                    features: Vec::new(),
                    kind: kind.0,
                    git: None,
                    branch: None,
                    tag: None,
                    path: None,
                });
                println!("✅ {} {} {}", name.green(), "=>".yellow(), version.black());
                continue;
            }

            let (valid, v) = check_version_exists(name, &version, None)
                .await
                .unwrap_or((false, version.clone()));
            if !valid {
                let packages = &METADATA.packages;
                let version_found = packages.iter().find(|p| {
                    if &version == "*" {
                        return &p.name == name;
                    }
                    &p.name == name && p.version == version
                });

                if version_found.is_none() {
                    println!(
                        "⚠️  {} {} {} (using original version)",
                        name.green(),
                        "=>".yellow(),
                        version.black()
                    );
                } else {
                    let found_version = &version_found.unwrap().version;
                    println!(
                        "✅ {} {} {}",
                        name.green(),
                        "=>".yellow(),
                        found_version.black()
                    );
                }
            } else {
                println!("✅ {} {} {}", name.green(), "=>".yellow(), v.black());
            }

            props.dependencies.push(Dependency {
                name: name.to_string(),
                version: if valid { v } else { version },
                features: Vec::new(),
                kind: kind.0,
                git: None,
                branch: None,
                tag: None,
                path: None,
            });
        } else if value.is_object() {
            let dep_obj = value.as_object().unwrap();

            let features: Vec<String> = dep_obj
                .get("features")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|f| f.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();

            if let Some(path_str) = dep_obj.get("path").and_then(|v| v.as_str()) {
                let norm = normalize_dep_path_in_dir(path_str, base_dir);
                props.dependencies.push(Dependency {
                    name: name.to_string(),
                    version: "0.1.0".to_string(),
                    features,
                    kind: kind.1,
                    git: None,
                    branch: None,
                    tag: None,
                    path: Some(norm.clone()),
                });
                println!("📦 {} {} {}", name.green(), "=>".yellow(), norm.black());
            } else if let Some(git_url) = dep_obj.get("git").and_then(|v| v.as_str()) {
                println!("🔗 Git dependency found: {} => {}", name, git_url);

                props.dependencies.push(Dependency {
                    name: name.to_string(),
                    version: "0.1.0".to_string(),
                    features,
                    kind: kind.1,
                    git: Some(git_url.to_string()),
                    branch: dep_obj
                        .get("branch")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    tag: dep_obj
                        .get("tag")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    path: None,
                });

                println!("✅ {} {} {}", name.green(), "=>".yellow(), git_url.black());
            } else if let Some(version) = dep_obj.get("version").and_then(|v| v.as_str()) {
                let resolved_ver = if !version.is_empty() && version != "*" && version != "latest" {
                    version.to_string()
                } else {
                    check_version_exists(name, version, None)
                        .await
                        .map(|(ok, v)| if ok { v } else { version.to_string() })
                        .unwrap_or_else(|_| version.to_string())
                };

                props.dependencies.push(Dependency {
                    name: name.to_string(),
                    version: resolved_ver.clone(),
                    features,
                    kind: kind.1,
                    git: None,
                    branch: None,
                    tag: None,
                    path: None,
                });

                println!(
                    "✅ {} {} {}",
                    name.green(),
                    "=>".yellow(),
                    resolved_ver.black()
                );
            }
        }
    }
}

impl<'a> Default for Properties<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Properties<'a> {
    pub fn new() -> Self {
        Self {
            name: "",
            version: "",
            edition: 2021,
            kind: "bin".to_string(),
            bin_name: None,
            lib_name: None,
            lto: None,
            dependencies: Vec::new(),
        }
    }

    pub async fn from_toml(toml: &'a Value) -> Self {
        let mut properties = Self::new();

        validate!(properties, name, toml["package"]["name"].as_str());
        validate!(properties, version, toml["package"]["version"].as_str());
        validate!(
            properties,
            edition,
            toml["package"]["edition"]
                .as_str()
                .and_then(|e| e.parse::<u64>().ok())
        );

        let deps = toml["dependencies"].as_object();
        let dev_deps = toml["dev-dependencies"].as_object();
        let build_deps = toml["build-dependencies"].as_object();

        let cwd = std::env::current_dir().unwrap_or_default();
        if deps.is_some() {
            map_deps(
                &mut properties,
                &toml["dependencies"],
                MapDepMode::Normal,
                &cwd,
            )
            .await;
        }

        if dev_deps.is_some() {
            map_deps(
                &mut properties,
                &toml["dev-dependencies"],
                MapDepMode::Dev,
                &cwd,
            )
            .await;
        }

        if build_deps.is_some() {
            map_deps(
                &mut properties,
                &toml["build-dependencies"],
                MapDepMode::Build,
                &cwd,
            )
            .await;
        }

        properties
    }

    pub async fn from_kson(kson: &'a Value) -> Self {
        let cwd = std::env::current_dir().unwrap_or_default();
        Self::from_kson_in_dir(kson, &cwd).await
    }

    pub async fn from_kson_in_dir(kson: &'a Value, base_dir: &Path) -> Self {
        let mut properties = Self::new();

        validate!(properties, name, kson["name"].as_str());
        validate!(properties, version, kson["version"].as_str());
        properties.edition = kson["edition"].as_u64().unwrap_or(2021);
        if let Some(k) = kson["kind"].as_str() {
            properties.kind = k.to_string();
        }
        if let Some(bn) = kson["bin_name"].as_str() {
            properties.bin_name = Some(bn.to_string());
        }
        if let Some(ln) = kson["lib_name"].as_str() {
            properties.lib_name = Some(ln.to_string());
        }
        if let Some(lto) = kson["lto"].as_bool() {
            properties.lto = Some(lto);
        }

        let deps = kson["dependencies"].as_object();
        let dev_deps = kson["dev_dependencies"].as_object();
        let build_deps = kson["build_dependencies"].as_object();

        if deps.is_some() {
            map_deps(
                &mut properties,
                &kson["dependencies"],
                MapDepMode::Normal,
                base_dir,
            )
            .await;
        }

        if dev_deps.is_some() {
            map_deps(
                &mut properties,
                &kson["dev_dependencies"],
                MapDepMode::Dev,
                base_dir,
            )
            .await;
        }

        if build_deps.is_some() {
            map_deps(
                &mut properties,
                &kson["build_dependencies"],
                MapDepMode::Build,
                base_dir,
            )
            .await;
        }

        properties
    }

    pub async fn add_dependency(&mut self, name: &str, version: &str) {
        if self.dependencies.iter().any(|d| d.name == name) {
            return;
        }
        let actual_version = if version == "latest" {
            let (valid, v) = check_version_exists(name, "*", None)
                .await
                .unwrap_or((false, "1.0.0".to_string()));
            if valid {
                v
            } else {
                "1.0.0".to_string()
            }
        } else {
            version.to_string()
        };

        let dep = Dependency {
            name: name.to_string(),
            version: actual_version.clone(),
            features: Vec::new(),
            kind: DepKind::NormalOnlyVersion,
            git: None,
            branch: None,
            tag: None,
            path: None,
        };

        self.dependencies.push(dep);
        println!(
            "✅ {} {} {}",
            name.green(),
            "=>".yellow(),
            actual_version.black()
        );
    }

    /// Register a local path dependency: `name = { path = "<path>" }`.
    pub fn add_path_dependency(&mut self, name: &str, path: &str) {
        if self.dependencies.iter().any(|d| d.name == name) {
            return;
        }
        let norm = normalize_dep_path(path);
        self.dependencies.push(Dependency {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            features: Vec::new(),
            kind: DepKind::NormalOnlyVersion,
            git: None,
            branch: None,
            tag: None,
            path: Some(norm.clone()),
        });
        println!("📦 {} {} {}", name.green(), "=>".yellow(), norm.black());
    }

    pub fn to_toml(&self) -> String {
        let mut deps_str = String::new();

        for dep in &self.dependencies {
            if let Some(p) = &dep.path {
                if dep.features.is_empty() {
                    deps_str.push_str(&format!("{} = {{ path = \"{}\" }}\n", dep.name, p));
                } else {
                    deps_str.push_str(&format!(
                        "{} = {{ path = \"{}\", features = [{}] }}\n",
                        dep.name,
                        p,
                        dep.features
                            .iter()
                            .map(|f| format!("\"{}\"", f))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            } else if let Some(git_url) = &dep.git {
                deps_str.push_str(&format!("{} = {{ git = \"{}\"", dep.name, git_url));
                if let Some(branch) = &dep.branch {
                    deps_str.push_str(&format!(", branch = \"{}\"", branch));
                }
                if let Some(tag) = &dep.tag {
                    deps_str.push_str(&format!(", tag = \"{}\"", tag));
                }
                if !dep.features.is_empty() {
                    deps_str.push_str(&format!(
                        ", features = [{}]",
                        dep.features
                            .iter()
                            .map(|f| format!("\"{}\"", f))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                deps_str.push_str(" }\n");
            } else if dep.features.is_empty()
                && matches!(
                    dep.kind,
                    DepKind::NormalOnlyVersion
                        | DepKind::DevOnlyVersion
                        | DepKind::BuildOnlyVersion
                )
            {
                deps_str.push_str(&format!("{} = \"{}\"\n", dep.name, dep.version));
            } else if dep.features.is_empty() {
                deps_str.push_str(&format!("{} = \"{}\"\n", dep.name, dep.version));
            } else {
                deps_str.push_str(&format!(
                    "{} = {{ version = \"{}\", features = [{}] }}\n",
                    dep.name,
                    dep.version,
                    dep.features
                        .iter()
                        .map(|f| format!("\"{}\"", f))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }

        let dependencies_section = if deps_str.is_empty() {
            String::new()
        } else {
            format!("\n[dependencies]\n{}", deps_str)
        };

        let target_section = match self.kind.as_str() {
            "lib" => {
                let ln = self
                    .lib_name
                    .clone()
                    .unwrap_or_else(|| self.name.replace('-', "_"));
                format!("\n[lib]\nname = \"{ln}\"\npath = \"src/lib.rs\"")
            }
            "both" => {
                let ln = self
                    .lib_name
                    .clone()
                    .unwrap_or_else(|| self.name.replace('-', "_"));
                let bn = self
                    .bin_name
                    .clone()
                    .unwrap_or_else(|| self.name.to_string());
                format!(
                    "\n[lib]\nname = \"{ln}\"\npath = \"src/lib.rs\"\n\n[[bin]]\nname = \"{bn}\"\npath = \"src/main.rs\""
                )
            }
            _ => {
                let bn = self
                    .bin_name
                    .clone()
                    .unwrap_or_else(|| self.name.to_string());
                format!("\n[[bin]]\nname = \"{bn}\"\npath = \"src/main.rs\"")
            }
        };

        let profile_section = match self.lto {
            Some(lto_val) => format!("\n[profile.release]\nlto = {lto_val}\n"),
            None => String::new(),
        };

        format!(
            r#"# Cargo.toml generated by CForge v{}
[package]
name = "{name}"
version = "{version}"
edition = "{edition}"

[workspace]
{target_section}{dependencies_section}{profile_section}"#,
            std::env::var("CFORGE_VERSION")
                .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string()),
            name = self.name,
            version = self.version,
            edition = self.edition,
            target_section = target_section,
            dependencies_section = dependencies_section,
            profile_section = profile_section,
        )
    }
}
