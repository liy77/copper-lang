mod field;
mod method;
mod generic;

use regex::Regex;
use generic::Generic;
use field::Field;
use method::Method;

#[derive(Debug)]
pub struct ParsedClass {
    pub name: String,
    pub child: Option<String>,
    pub(crate) generics: Vec<Generic>,
    pub(crate) fields: Vec<Field>,
    pub(crate) methods: Vec<Method>,
}

impl ToString for ParsedClass {
    fn to_string(&self) -> String {
        let mut builded = format!("struct {} {{\n", self.name);
        
        // Adicionar campos
        for field in &self.fields {
            let visibility = if field.is_private { "" } else { "pub " };
            builded.push_str(&format!("    {}{}: {},\n", visibility, field.name, field.field_type));
        }
        builded.push_str("}\n\n");

        // Implementação dos métodos
        builded.push_str(&format!("impl {} {{\n", self.name));
        for method in &self.methods {
            let visibility = if method.is_private { "" } else { "pub " };
            let generics = if !method.generics.is_empty() {
                let generics_str = method.generics.iter().map(|g| g.to_string()).collect::<Vec<_>>().join(", ");
                format!("<{}>", generics_str)
            } else {
                "".to_string()
            };
            let args = method.args.iter()
                .map(|(name, ty)| format!("{}: {}", name, ty))
                .collect::<Vec<_>>()
                .join(", ");
            let return_type = if method.return_type != "()" {
                format!(" -> {}", method.return_type)
            } else {
                "".to_string()
            };
            let body = if method.body.is_empty() {
                ";".to_string()
            } else {
                format!(" {{\n        {}\n    }}", method.body)
            };
            builded.push_str(&format!(
                "    {}fn {}{}({}){}{}\n",
                visibility, method.name, generics, args, return_type, body
            ));
        }
        builded.push_str("}\n");

        builded
    }
}

pub fn parse_class(class: &str) -> ParsedClass {
    let mut parsed_class = ParsedClass {
        name: String::new(),
        child: None,
        generics: Vec::new(),
        fields: Vec::new(),
        methods: Vec::new(),
    };

    // Regex para capturar o nome da classe e seus genéricos
    let class_re = Regex::new(r"class\s+(\w+)<([^>]*)>\s*\{").unwrap();
    if let Some(caps) = class_re.captures(class) {
        parsed_class.name = caps[1].to_string();

        let generics_str = &caps[2];
        for generic in generics_str.split(',') {
            let generic = generic.trim();
            if generic.starts_with('\'') {
                parsed_class.generics.push(Generic::Lifetime(generic.to_string()));
            } else {
                parsed_class.generics.push(Generic::Type(generic.to_string()));
            }
        }
    }

    // Regex para capturar campos
    let field_re = Regex::new(r"^\s*(pub|crate|priv)?\s*(\w+):\s*([^;]+);\s*$").unwrap();

    // Split class definition into lines for easier parsing
    for line in class.lines() {
        if let Some(cap) = field_re.captures(line) {
            let is_public = cap.get(1).is_some();
            parsed_class.fields.push(Field {
                name: cap[2].to_string(),
                field_type: cap[3].trim().to_string(),
                is_private: !is_public,
            });
        }
    }

    // Regex para capturar métodos
    let method_re = Regex::new(
        r"(?s)(?P<pub>pub\s+)?(?P<return_type>\w+)(?:<(?P<return_type_generics>[^>]*)>)?\s+(?P<name>\w+)\s*\((?P<args>[^\)]*)\)\s*(?P<const>const\s*)?(?P<body>\{[^}]*\}|;)"
    ).unwrap();

    for cap in method_re.captures_iter(class) {
        let name = &cap["name"];
        let return_type = &cap["return_type"];
        let return_type_generics_str = cap.name("return_type_generics").map_or("", |m| m.as_str());
        let args_str = &cap["args"];
        let body = &cap["body"];
        let is_const = cap.name("const").is_some();
        let is_public = cap.name("pub").is_some();

        // Parse generics of return type if exists
        let mut return_type_generics = Vec::new();
        if !return_type_generics_str.is_empty() {
            for generic in return_type_generics_str.split(',') {
                let generic = generic.trim();
                if generic.starts_with('\'') {
                    return_type_generics.push(Generic::Lifetime(generic.to_string()));
                } else {
                    return_type_generics.push(Generic::Type(generic.to_string()));
                }
            }
        }

        // Parse arguments
        let mut args = Vec::new();
        for arg in args_str.split(',') {
            let arg = arg.trim();
            if !arg.is_empty() {
                let parts: Vec<&str> = arg.split(':').collect();
                if parts.len() == 2 {
                    args.push((parts[0].trim().to_string(), parts[1].trim().to_string()));
                }
            }
        }

        // Check if the method is abstract
        let is_empty_method = body == ";";

        parsed_class.methods.push(Method {
            name: name.to_string(),
            return_type: return_type.to_string(),
            is_private: !is_public,
            is_const,
            generics: return_type_generics, // Genéricos do tipo de retorno
            args,
            body: if is_empty_method { String::new() } else { body.trim().to_string() },
        });
    }

    parsed_class
}

#[cfg(test)]
#[test]
fn test() {
    let class = r#"
class Test<T, 'a> {
    pub field1: i32;
    field2: String;
    priv field3: Vec<T>;

    pub Self new() {
        Self {
            field1: 0,
            field2: String::new(),
            field3: Vec::new(),
        }
    }

    pub &Vec<T> get_field3(&self) {
        &self.field3
    }
}
    "#;

    let parsed_class = parse_class(class);
    println!("{:#?}", parsed_class.to_string());
}