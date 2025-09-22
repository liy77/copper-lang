use once_cell::sync::Lazy;
use serde_json::Value;
use colored::Colorize;

use crate::{cargo, cforge::fetch::check_version_exists};

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
    name: String,
    version: String,
    features: Vec<String>,
    kind: DepKind,
    git: Option<String>,
    branch: Option<String>,
    tag: Option<String>,
}

pub struct Properties<'a> {
    name: &'a str,
    version: &'a str,
    edition: u64,
    dependencies: Vec<Dependency>,
}

const METADATA: Lazy<cargo::Metadata> = Lazy::new(|| {
    let mut extractor = cargo::Extractor::new();
    extractor.fetch_metadata()
});

// Map dependencies from properties.kson to Cargo.toml
async fn map_deps<'a>(props: &mut Properties<'a>, deps: &'a Value, mode: MapDepMode) {
    let deps = deps.as_object().unwrap();

    let kind = match mode {
        MapDepMode::Normal => {
            println!("📩 Installing normal dependencies");
            (DepKind::NormalOnlyVersion, DepKind::NormalJson)
        },
        MapDepMode::Dev => {
            println!("🧑‍💻 Installing dev dependencies");
            (DepKind::DevOnlyVersion, DepKind::DevJson)
        }
        MapDepMode::Build => {
            println!("🔨 Installing build dependencies");
            (DepKind::BuildOnlyVersion, DepKind::BuildJson)
        }
    };

    for (name, value) in deps {
        println!("🔍 Debug: Processing dependency '{}' with value: {}", name, serde_json::to_string(value).unwrap());
        
        if value.is_string() {
            // Simple version dependency
            let version = value.as_str().unwrap().trim_matches('"').to_string();
            
            if !version.is_empty() && version != "*" && version != "latest" {
                props.dependencies.push(Dependency {
                    name: name.to_string(),
                    version: version.clone(),
                    features: Vec::new(),
                    kind: kind.0,
                    git: None,
                    branch: None,
                    tag: None,
                });
                println!("✅ {} {} {}", name.green(), "=>".yellow(), version.black());
                continue;
            }

            let (valid, v) = check_version_exists(&name, &version, None).await.unwrap_or((false, version.clone()));
            if !valid {
                let packages = &METADATA.packages;
                let version_found = packages.iter().find(|p| {
                    if &version == "*" {
                        return &p.name == name;
                    }
                    &p.name == name && p.version == version
                });

                if version_found.is_none() {
                    println!("⚠️  {} {} {} (using original version)", name.green(), "=>".yellow(), version.black());
                } else {
                    let found_version = &version_found.unwrap().version;
                    println!("✅ {} {} {}", name.green(), "=>".yellow(), found_version.black());
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
            });
            
        } else if value.is_object() {
            // Complex dependency (with git, features, etc.)
            let dep_obj = value.as_object().unwrap();
            
            // Check if it's a git dependency
            if let Some(git_url) = dep_obj.get("git").and_then(|v| v.as_str()) {
                println!("🔗 Git dependency found: {} => {}", name, git_url);
                
                props.dependencies.push(Dependency {
                    name: name.to_string(),
                    version: "0.1.0".to_string(), // Git dependencies don't need version
                    features: dep_obj.get("features")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|f| f.as_str()).map(|s| s.to_string()).collect())
                        .unwrap_or_default(),
                    kind: kind.1,
                    git: Some(git_url.to_string()),
                    branch: dep_obj.get("branch").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    tag: dep_obj.get("tag").and_then(|v| v.as_str()).map(|s| s.to_string()),
                });
                
                println!("✅ {} {} {}", name.green(), "=>".yellow(), git_url.black());
            } else if let Some(version) = dep_obj.get("version").and_then(|v| v.as_str()) {
                // Regular dependency with version and possibly features
                let (valid, v) = check_version_exists(&name, &version, None).await.unwrap_or((false, version.to_string()));
                
                props.dependencies.push(Dependency {
                    name: name.to_string(),
                    version: if valid { v.clone() } else { version.to_string() },
                    features: dep_obj.get("features")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|f| f.as_str()).map(|s| s.to_string()).collect())
                        .unwrap_or_default(),
                    kind: kind.1,
                    git: None,
                    branch: None,
                    tag: None,
                });
                
                println!("✅ {} {} {}", name.green(), "=>".yellow(), if valid { v } else { version.to_string() }.black());
            }
        }
    }
}


impl<'a> Properties<'a> {
    pub fn new() -> Self {
        Self {
            name: "",
            version: "",
            edition: 2018,
            dependencies: Vec::new(),
        }
    }

    pub async fn from_toml(toml: &'a Value) -> Self {
        let mut properties = Self::new();

        validate!(properties, name, toml["package"]["name"].as_str());
        validate!(properties, version, toml["package"]["version"].as_str());
        validate!(properties, edition, toml["package"]["edition"].as_str().and_then(|e| e.parse::<u64>().ok()));

        let deps = toml["dependencies"].as_object();
        let dev_deps = toml["dev-dependencies"].as_object();
        let build_deps = toml["build-dependencies"].as_object();

        if deps.is_some() {
            map_deps(&mut properties, &toml["dependencies"], MapDepMode::Normal).await;
        }

        if dev_deps.is_some() {
            map_deps(&mut properties, &toml["dev-dependencies"], MapDepMode::Dev).await;
        }

        if build_deps.is_some() {
            map_deps(&mut properties, &toml["build-dependencies"], MapDepMode::Build).await;
        }

        properties
    }

    pub async fn from_kson(kson: &'a Value) -> Self {
        let mut properties = Self::new();

        println!("🔍 Debug: Full KSON JSON: {}", serde_json::to_string_pretty(kson).unwrap());

        validate!(properties, name, kson["name"].as_str());
        validate!(properties, version, kson["version"].as_str());
        validate!(properties, edition, kson["edition"].as_u64());

        let deps = kson["dependencies"].as_object();
        let dev_deps = kson["dev_dependencies"].as_object();
        let build_deps = kson["build_dependencies"].as_object();

        println!("🔍 Debug: Dependencies found: {}", deps.is_some());
        if let Some(deps_obj) = deps {
            println!("🔍 Debug: Dependencies content: {}", serde_json::to_string_pretty(deps_obj).unwrap());
        }

        if deps.is_some() {
            map_deps(&mut properties, &kson["dependencies"], MapDepMode::Normal).await;
        }

        if dev_deps.is_some() {
            map_deps(&mut properties, &kson["dev_dependencies"], MapDepMode::Dev).await;
        }

        if build_deps.is_some() {
            map_deps(&mut properties, &kson["build_dependencies"], MapDepMode::Build).await;
        }

        properties
    }

    pub async fn add_dependency(&mut self, name: &str, version: &str) {
        let actual_version = if version == "latest" {
            let (valid, v) = check_version_exists(name, "*", None).await.unwrap_or((false, "1.0.0".to_string()));
            if valid { v } else { "1.0.0".to_string() }
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
        };

        // Verificar se a dependência já existe
        if !self.dependencies.iter().any(|d| d.name == name) {
            self.dependencies.push(dep);
            println!("✅ {} {} {}", name.green(), "=>".yellow(), actual_version.black());
        }
    }

    pub fn to_toml(&self) -> String {
        let mut deps_str = String::new();

        for dep in &self.dependencies {
            if let Some(git_url) = &dep.git {
                // Git dependency
                deps_str.push_str(&format!("{} = {{ git = \"{}\"", dep.name, git_url));
                if let Some(branch) = &dep.branch {
                    deps_str.push_str(&format!(", branch = \"{}\"", branch));
                }
                if let Some(tag) = &dep.tag {
                    deps_str.push_str(&format!(", tag = \"{}\"", tag));
                }
                if !dep.features.is_empty() {
                    deps_str.push_str(&format!(", features = [{}]", dep.features.iter().map(|f| format!("\"{}\"", f)).collect::<Vec<_>>().join(", ")));
                }
                deps_str.push_str(" }\n");
            } else if matches!(dep.kind, DepKind::NormalOnlyVersion | DepKind::DevOnlyVersion | DepKind::BuildOnlyVersion) {
                deps_str.push_str(&format!("{} = \"{}\"\n", dep.name, dep.version));
            } else {
                deps_str.push_str(&format!("{} = {{ version = \"{}\"", dep.name, dep.version));
                if !dep.features.is_empty() {
                    deps_str.push_str(&format!(", features = [{}]", dep.features.iter().map(|f| format!("\"{}\"", f)).collect::<Vec<_>>().join(", ")));
                }
                deps_str.push_str(" }\n");
            }
        }

        let dependencies_section = if deps_str.is_empty() {
            String::new()
        } else {
            format!("\n[dependencies]\n{}", deps_str)
        };

        format!(r#"# Cargo.toml generated by CForge v{}
[package]
name = "{name}"
version = "{version}"
edition = "{edition}"

[[bin]]
name = "{name}"
path = "src/main.rs"{dependencies_section}"#, 
            std::env::var("CFORGE_VERSION").unwrap(), 
            name = self.name, 
            version = self.version, 
            edition = self.edition, 
            dependencies_section = dependencies_section
        )
    }
}