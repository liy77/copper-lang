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
        let builded = "struct ".to_string() + &self.name + " {\n";
        
        let fields = self.fields.iter().map(|field| {
            let visibility = if field.is_private { "" } else { "pub" };
            format!("    {} {}: {},\n", visibility, field.name, field.field_type)
        }).collect::<String>();

        "".to_string()
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
        println!("Processing line: {}", line); // Debugging line
        if let Some(cap) = field_re.captures(line) {
            let is_public = cap.get(1).is_some();
            parsed_class.fields.push(Field {
                name: cap[2].to_string(),
                field_type: cap[3].trim().to_string(),
                is_private: !is_public,
            });
            println!("Captured field: {:?}", parsed_class.fields.last()); // Debugging captured field
        } else {
            println!("No match for field regex in line: {}", line); // Debugging non-matching lines
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

        // Parse genéricos do tipo de retorno (se existirem)
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

        // Parse argumentos
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

        // Verifica se o método é apenas uma assinatura (sem implementação)
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
