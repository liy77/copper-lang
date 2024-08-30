use serde_json::Value;
use colored::Colorize;

use crate::cforge::fetch::check_version_exists;

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
}

pub struct Properties<'a> {
    name: &'a str,
    version: &'a str,
    edition: u64,
    dependencies: Vec<Dependency>,
}

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


    let mut inside_dep_information = false;
    for (name, value) in deps {
        if value.is_object() {
            inside_dep_information = true;
        }


        let mut version = String::new();
        if value.is_string() && !value.is_object() {
            version = value.to_string();

            let (valid, v) = check_version_exists(&name, &version, None).await.unwrap();
            if !valid {
                println!("❌ {} {} {}", name.green(), "=>".yellow(), version.black());
                continue;
            }
            
            version = v; // Replace * or latest with the actual version

            props.dependencies.push(Dependency {
                name: name.to_string(),
                version: version.to_string(),
                features: Vec::new(),
                kind: kind.0,
            });
        } else if !inside_dep_information {
            let dep = value.as_object().unwrap();
            let features_vec: Vec<Value> = vec![];
            let features = dep["features"].as_array().unwrap_or(&features_vec);
            
            version = dep["version"].as_str().expect("Version is required in properties.kson").to_string();

            let (valid, v) = check_version_exists(&name, &version, None).await.unwrap();

            if !valid {
                println!("❌ {} {} {}", name.green(), "->".yellow(), version.black());
                continue;
            }

            version = v; // Replace * or latest with the actual version

            props.dependencies.push(Dependency {
                name: name.to_string(),
                version: version.to_string(),
                features: features.iter().map(|f| f.as_str().unwrap().to_string()).collect(),
                kind: kind.1,
            });
        }

        println!("✅ {} {} {}", name.green(), "->".yellow(), version.black());
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

    pub async fn from_kson(kson: &'a Value) -> Self {
        let mut properties = Self::new();

        validate!(properties, name, kson["name"].as_str());
        validate!(properties, version, kson["version"].as_str());
        validate!(properties, edition, kson["edition"].as_u64());

        let deps = kson["dependencies"].as_object();
        let dev_deps = kson["dev_dependencies"].as_object();
        let build_deps = kson["build_dependencies"].as_object();

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

    pub fn to_toml(&self) -> String {
        let mut deps_str = String::new();

        for dep in &self.dependencies {
            if matches!(dep.kind, DepKind::NormalOnlyVersion | DepKind::DevOnlyVersion | DepKind::BuildOnlyVersion) {
                deps_str.push_str(&format!("{} = \"{}\"\n", dep.name, dep.version));
            } else {
                deps_str.push_str(&format!("[{}]\nversion = \"{}\"\n", dep.name, dep.version));
                if !dep.features.is_empty() {
                    deps_str.push_str(&format!("features = [{}]\n", dep.features.join(", ")));
                }
            }
        }

        format!(r#"# Cargo.toml generated by CForge v{}
[package]
name = "{name}"
version = "{version}"
edition = "{edition}"

[dependencies]
{deps_str}"#, std::env::var("CFORGE_VERSION").unwrap(), name = self.name, version = self.version, edition = self.edition, deps_str = deps_str)
    }
}