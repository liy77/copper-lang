use std::{io::Write, process::{Command, Stdio}};

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
        }
    }

    pub fn write_main_function(&mut self) {
        if self.main_function_code.is_empty() {
            return;
        }

        self.force_append(&("\n\nfn main() {\n".to_owned() + &self.main_function_code.replace("\n\n", "") + "}"), false);
    }

    pub fn append_to_main_function(&mut self, value: &str, space: bool) {
        self.main_function_code.push_str(&(value.to_owned() + (if space { " " } else { "" })));
    }

    pub fn append(&mut self, value: &str, space: bool) {
        if !self.is_inside_function && !self.is_function {
            self.append_to_main_function(value, space);
        } else {
            self.force_append(value, space);
        }
    }

    pub fn force_append(&mut self, value: &str, space: bool) {
        self.value.push_str(&(value.to_owned() + (if space { " " } else { "" })));
    }

    pub fn ff_append(&mut self, value: &str, space: bool) {
        if self.is_inside_function {
            self.append(value, space);
        } else {
            self.force_append(value, space);
        }
    }

    pub fn enter_function(&mut self) {
        self.is_function = true;
        self.append(&"fn ".to_owned(), false);
    }

    pub fn enter_class(&mut self, name: &str) {
        self.is_class = true;
        self.append(&("struct ".to_owned() + name + " {"), false);
    }

    pub fn exit_class(&mut self) {
        self.is_class = false;
        self.append(&"}".to_owned(), false);
    }

    pub fn exit_function(&mut self) {
        self.is_function = false;
    }

    pub fn return_type(&mut self, value: String) {
        self.return_type = value;
    }

    pub fn add_required_import(&mut self, value: &str) {
        self.value = "use ".to_owned() + value + " as " + "__" + value + "__" + "; // Imported by CForge for implementations\n" + &self.value;
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
            let full_aliases = format!("// Native data type aliases for Copper\n{}\n{}\n", imports, aliases);
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
        let mut deps = Vec::new();
        
        if self.uses_json {
            deps.push("serde_json".to_string());
        }
        
        if self.uses_toml {
            deps.push("toml".to_string());
        }
        
        // XML não precisa de dependência externa por enquanto (usa String)
        
        deps
    }

    pub fn has_required_import(&self, value: &str) -> bool {
        self.value.contains(&("use ".to_owned() + value + " as " + "__" + value + "__" + ";\n"))
    }

    pub fn get(&mut self) -> std::result::Result<String, Box<dyn std::error::Error>> {
        self.value = self.value.trim().to_string();

        // Inicia um processo rustfmt
        let mut process = Command::new("rustfmt")
            .arg("--emit")
            .arg("stdout")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        
        // Escreve o código na entrada padrão do processo rustfmt
        {
            let stdin = process.stdin.as_mut().ok_or("Failed to open stdin").unwrap();
            stdin.write_all(self.value.as_bytes())?;
        }

        // Captura a saída padrão do processo rustfmt
        let output = process.wait_with_output()?;
        let formatted = String::from_utf8(output.stdout)?;

        if formatted.is_empty() {
            return Ok(self.value.clone());
        }

        Ok(formatted)
    }
}