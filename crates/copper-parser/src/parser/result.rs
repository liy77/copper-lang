use std::{
    io::Write,
    process::{Command, Stdio},
};

/// The return-kind of a user-defined `main` function, used by `finalize` to
/// generate the real `fn main` entry point that calls the renamed
/// `__copper_main`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnKind {
    /// User `main` returns an integer type (i64/i32/…): the real entry calls
    /// `std::process::exit(__copper_main() as i32)`.
    Int,
    /// User `main` returns unit / void: the real entry just calls
    /// `__copper_main();`.
    Unit,
}

#[derive(Debug, Clone)]
pub struct Result {
    pub value: String,
    pub main_function_code: String,
    pub(crate) return_type: String,
    pub(crate) is_function: bool,
    pub(crate) is_class: bool,
    pub(crate) is_inside_function: bool,
    pub(crate) is_copper_function: bool,
    pub(crate) uses_json: bool,
    pub(crate) uses_xml: bool,
    pub(crate) uses_toml: bool,
    pub(crate) cstd_used: bool,
    /// Native std modules (other than cstd) pulled in via
    /// `import { … } from <module>` — currently `net` / `http`. Each is
    /// bundled as `pub mod <name> { ... }` and may inject crate deps.
    pub(crate) used_std_modules: std::collections::BTreeSet<String>,
    /// External crates pulled in by `import { … } from <crate>` (anything that
    /// isn't `std`/`cstd`/a local module). cforge resolves their versions and
    /// adds them to the generated Cargo.toml — after filtering out names that
    /// are actually sibling modules.
    pub(crate) external_crates: Vec<String>,
    /// Every Copper struct parsed, as `(struct_name, field_names)`, recorded so
    /// that — when the `reflect` module is imported — cforge can append an
    /// `impl reflect::Reflect for <Struct>` at end-of-output (after the struct
    /// definitions). Generic structs are not recorded (their impl would need
    /// generic bounds — out of MVP scope).
    pub(crate) reflect_structs: Vec<(String, Vec<String>)>,
    /// Transpiled Rust source (crate-root lib) for each used std module, as
    /// `(module_name, lib_source)`. cforge writes each as a separate local
    /// crate under `__copper__/std/<name>/` and path-depends on it, instead of
    /// inlining a `pub mod` into main.rs.
    pub(crate) std_lib_crates: Vec<(String, String)>,
    /// Set when the user defines their own `func main()`. The function is
    /// emitted under the renamed symbol `__copper_main`, and `finalize` uses
    /// this to synthesize the real `fn main` entry point (and to suppress the
    /// auto-generated `fn main` wrapper that would otherwise collide).
    pub(crate) user_main: Option<ReturnKind>,
}

impl Default for Result {
    fn default() -> Self {
        Self::new()
    }
}

impl Result {
    pub fn new() -> Self {
        Self {
            value: String::new(),
            main_function_code: String::new(),
            return_type: String::new(),
            is_function: false,
            is_class: false,
            is_inside_function: false,
            is_copper_function: false,
            uses_json: false,
            uses_xml: false,
            uses_toml: false,
            cstd_used: false,
            used_std_modules: std::collections::BTreeSet::new(),
            external_crates: Vec::new(),
            reflect_structs: Vec::new(),
            std_lib_crates: Vec::new(),
            user_main: None,
        }
    }

    /// Record a used std module's transpiled lib source so cforge can write it
    /// as a separate local crate (`__copper__/std/<name>/`).
    pub fn add_std_lib_crate(&mut self, name: &str, lib_source: &str) {
        if !self.std_lib_crates.iter().any(|(n, _)| n == name) {
            self.std_lib_crates
                .push((name.to_string(), lib_source.to_string()));
        }
    }

    /// The `(name, lib_source)` pairs for every used std module.
    pub fn std_lib_crates(&self) -> &[(String, String)] {
        &self.std_lib_crates
    }

    /// Prepend raw text (e.g. `use copper_http as http;` aliases) to the
    /// generated Rust, ahead of everything else.
    pub fn prepend_value(&mut self, text: &str) {
        self.value = format!("{}{}", text, self.value);
    }

    /// Record that the user defined their own `main` (renamed to
    /// `__copper_main` on emit), with the given return kind.
    pub fn set_user_main(&mut self, kind: ReturnKind) {
        self.user_main = Some(kind);
    }

    /// Record a Copper struct (name + ordered field names) for reflect codegen.
    pub fn record_reflect_struct(&mut self, name: &str, fields: Vec<String>) {
        self.reflect_structs
            .push((name.to_string(), fields));
    }

    /// If the `reflect` module is imported, append one
    /// `impl reflect::Reflect for <Struct>` per recorded struct to the output.
    /// Called at end-of-output so import-before-struct ordering doesn't matter,
    /// and emitted *after* the struct definitions already in `self.value`.
    pub fn append_reflect_impls(&mut self) {
        if !self.used_std_modules.contains("reflect") {
            return;
        }
        let mut out = String::new();
        for (name, fields) in &self.reflect_structs {
            out.push_str(&format!("impl reflect::Reflect for {} {{\n", name));
            out.push_str("    fn reflect(&self) -> reflect::Reflected {\n");
            out.push_str(&format!(
                "        reflect::Reflected::new(\"{}\", vec![\n",
                name
            ));
            // Each field reflects via `to_value()` — implemented for scalars,
            // `Vec<T>`, `Option<T>`, and (auto-derived) every Copper struct —
            // so a nested struct field recurses as `Value::Object(...)`. The
            // fully-qualified call avoids needing the trait in scope.
            for field in fields {
                out.push_str(&format!(
                    "            (\"{f}\".to_string(), reflect::Reflect::to_value(&self.{f})),\n",
                    f = field
                ));
            }
            out.push_str("        ])\n");
            out.push_str("    }\n");
            // The companion `to_value` so this struct nests inside another.
            out.push_str("    fn to_value(&self) -> reflect::Value {\n");
            out.push_str("        reflect::Value::Object(self.reflect())\n");
            out.push_str("    }\n");
            out.push_str("}\n\n");
        }
        if !out.is_empty() {
            self.value.push('\n');
            self.value.push_str(&out);
        }
    }

    /// Register an external crate imported via `from <crate>`.
    pub fn mark_external_crate(&mut self, name: &str) {
        let n = name.to_string();
        if !self.external_crates.contains(&n) {
            self.external_crates.push(n);
        }
    }

    pub fn write_main_function(&mut self) {
        // The collected top-level body, with the cosmetic double-newline
        // collapsing the existing wrapper applied. Used both to decide whether
        // there is *meaningful* top-level code and to emit the auto-wrapper.
        let body = self.main_function_code.replace("\n\n", "");
        // "Meaningful" top-level code is anything beyond stray statement
        // separators / whitespace. After a user `func` the closing-brace
        // newline can leave a lone `";\n"` in main_function_code; that is NOT
        // a real top-level statement, so don't let it trigger the auto-wrapper
        // or the both-top-level-and-main conflict diagnostic.
        let top_level_nonempty = body
            .chars()
            .any(|c| !c.is_whitespace() && c != ';');

        match self.user_main {
            // The user wrote their own `main` (emitted as `__copper_main`).
            // Generate the single real `fn main` entry that calls it, and do
            // NOT emit the auto-wrapper (it would create a duplicate `fn main`).
            Some(kind) => {
                if top_level_nonempty {
                    // Both top-level statements AND a user `func main` exist:
                    // genuinely ambiguous. Emit a clear compile-time error
                    // rather than silently dropping one or producing surprising
                    // output.
                    self.force_append(
                        "\n\ncompile_error!(\"a program cannot have both top-level statements and a `main` function; use one or the other\");\n",
                        false,
                    );
                }
                let entry = match kind {
                    ReturnKind::Int => {
                        "\n\nfn main() { std::process::exit(__copper_main() as i32); }"
                    }
                    ReturnKind::Unit => "\n\nfn main() { __copper_main(); }",
                };
                self.force_append(entry, false);
            }
            // No user `main`: emit the conventional auto-wrapper around the
            // top-level statements, exactly as before.
            None => {
                if self.main_function_code.is_empty() {
                    return;
                }
                self.force_append(
                    &("\n\nfn main() {\n".to_owned() + &body + "}"),
                    false,
                );
            }
        }
    }

    pub fn append_to_main_function(&mut self, value: &str, space: bool) {
        self.main_function_code
            .push_str(&(value.to_owned() + (if space { " " } else { "" })));
    }

    pub fn append(&mut self, value: &str, space: bool) {
        if !self.is_inside_function && !self.is_function {
            self.append_to_main_function(value, space);
        } else {
            self.force_append(value, space);
        }
    }

    pub fn force_append(&mut self, value: &str, space: bool) {
        self.value
            .push_str(&(value.to_owned() + (if space { " " } else { "" })));
    }

    pub fn ff_append(&mut self, value: &str, space: bool) {
        if self.is_inside_function {
            self.append(value, space);
        } else {
            self.force_append(value, space);
        }
    }

    pub fn enter_function(&mut self) {
        self.enter_function_vis(false);
    }

    /// Emit a function header, optionally `pub`. `pub func` → `pub fn`.
    pub fn enter_function_vis(&mut self, is_pub: bool) {
        self.is_function = true;
        self.append(if is_pub { "pub fn " } else { "fn " }, false);
    }

    pub fn enter_unsafe_function(&mut self) {
        self.enter_unsafe_function_vis(false);
    }

    /// Emit an unsafe function header, optionally `pub`.
    pub fn enter_unsafe_function_vis(&mut self, is_pub: bool) {
        self.is_function = true;
        self.append(
            if is_pub {
                "pub unsafe fn "
            } else {
                "unsafe fn "
            },
            false,
        );
    }

    pub fn enter_class(&mut self, name: &str) {
        self.is_class = true;
        self.append(&("struct ".to_owned() + name + " {"), false);
    }

    pub fn exit_class(&mut self) {
        self.is_class = false;
        self.append("}", false);
    }

    pub fn exit_function(&mut self) {
        self.is_function = false;
    }

    pub fn return_type(&mut self, value: String) {
        self.return_type = value;
    }

    pub fn add_required_import(&mut self, value: &str) {
        self.value = "use ".to_owned()
            + value
            + " as "
            + "__"
            + value
            + "__"
            + "; // Imported by CForge for implementations\n"
            + &self.value;
    }

    pub fn mark_cstd_used(&mut self, _name: &str) {
        self.cstd_used = true;
    }

    /// Mark a native std module (e.g. "net", "http") as imported, so the
    /// compiler bundles its `pub mod` and injects any crate deps.
    pub fn mark_std_module(&mut self, name: &str) {
        self.used_std_modules.insert(name.to_string());
    }

    /// The native std modules imported, sorted (deterministic emit order).
    pub fn used_std_modules(&self) -> Vec<String> {
        self.used_std_modules.iter().cloned().collect()
    }

    pub fn cstd_is_used(&self) -> bool {
        self.cstd_used
    }

    pub fn prepend_cstd_module(&mut self, transpiled_module: &str) {
        self.value = transpiled_module.to_string() + "\n" + &self.value;
    }

    pub fn add_data_type_aliases(&mut self) {
        let mut aliases = String::new();
        let mut imports = String::new();

        if self.uses_json {
            imports.push_str("use serde_json::{json, Value as JsonValue};\n");
            aliases.push_str("// JsonValue type alias already imported from serde_json\n");
        }

        if self.uses_xml {
            aliases.push_str("type XmlValue = String; // For now, XML is represented as String\n");
        }

        if self.uses_toml {
            imports.push_str("use toml;\n");
            aliases.push_str("type TomlValue = toml::Value;\n");
        }

        if !aliases.is_empty() {
            let full_aliases = format!(
                "// Native data type aliases for Copper\n{}\n{}\n",
                imports, aliases
            );
            self.value = full_aliases + &self.value;
        }
    }

    pub fn mark_json_usage(&mut self) {
        self.uses_json = true;
    }

    pub fn mark_xml_usage(&mut self) {
        self.uses_xml = true;
    }

    pub fn mark_toml_usage(&mut self) {
        self.uses_toml = true;
    }

    pub fn get_required_dependencies(&self) -> Vec<String> {
        // Dependencies for the MAIN project's Cargo.toml. Crate-backed std
        // MODULES (http/json/crypto) no longer add their crates here — each is
        // a separate local crate under `__copper__/std/<name>/` that carries
        // its own deps (see `std_module_crate_deps`); the main project just
        // path-depends on `copper_<name>` (added by cforge). What remains here
        // is the inline DATA-TYPE crates (the `json`/`toml` value types used
        // directly in main, not via a module) plus the user's external crates.
        const SERDE_JSON: &str = "serde_json@1";
        const TOML: &str = "toml@0.8";

        let mut deps = Vec::new();
        let mut add = |spec: &str, deps: &mut Vec<String>| {
            if !deps.iter().any(|d| d == spec) {
                deps.push(spec.to_string());
            }
        };

        if self.uses_json {
            add(SERDE_JSON, &mut deps);
        }
        if self.uses_toml {
            add(TOML, &mut deps);
        }

        // XML needs no external dependency for now (uses String).

        // External crates from `import { … } from <crate>`.
        for c in &self.external_crates {
            if !deps.contains(c) {
                deps.push(c.clone());
            }
        }

        deps
    }

    pub fn has_required_import(&self, value: &str) -> bool {
        self.value
            .contains(&("use ".to_owned() + value + " as " + "__" + value + "__" + ";\n"))
    }

    pub fn get(&mut self) -> std::result::Result<String, Box<dyn std::error::Error>> {
        self.value = self.value.trim().to_string();

        // Start a rustfmt process.
        let mut process = Command::new("rustfmt")
            .arg("--emit")
            .arg("stdout")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;

        // Write the code to rustfmt's stdin.
        {
            let stdin = process
                .stdin
                .as_mut()
                .ok_or("Failed to open stdin")
                .unwrap();
            stdin.write_all(self.value.as_bytes())?;
        }

        // Capture rustfmt's stdout.
        let output = process.wait_with_output()?;
        let formatted = String::from_utf8(output.stdout)?;

        if formatted.is_empty() {
            return Ok(self.value.clone());
        }

        Ok(formatted)
    }
}
