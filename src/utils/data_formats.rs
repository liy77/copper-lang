/// Native JSON functions for Copper language
pub mod json {
    use serde_json::{Value, Result};

    /// Parse JSON string into a JSON value
    pub fn parse(json_str: &str) -> Result<Value> {
        serde_json::from_str(json_str)
    }

    /// Convert JSON value to string
    pub fn stringify(value: &Value) -> String {
        serde_json::to_string(value).unwrap_or_default()
    }

    /// Pretty print JSON value
    pub fn pretty(value: &Value) -> String {
        serde_json::to_string_pretty(value).unwrap_or_default()
    }

    /// Get value from JSON object by key
    pub fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
        value.get(key)
    }

    /// Set value in JSON object
    pub fn set(value: &mut Value, key: &str, new_value: Value) {
        if let Some(obj) = value.as_object_mut() {
            obj.insert(key.to_string(), new_value);
        }
    }

    /// Check if JSON value has a key
    pub fn has_key(value: &Value, key: &str) -> bool {
        value.get(key).is_some()
    }

    /// Get all keys from JSON object
    pub fn keys(value: &Value) -> Vec<String> {
        match value {
            Value::Object(map) => map.keys().cloned().collect(),
            _ => vec![],
        }
    }

    /// Create empty JSON object
    pub fn object() -> Value {
        Value::Object(serde_json::Map::new())
    }

    /// Create empty JSON array
    pub fn array() -> Value {
        Value::Array(vec![])
    }
}

/// Native TOML functions for Copper language
pub mod toml_utils {
    use toml::{Value, from_str, to_string};

    /// Parse TOML string into a TOML value
    pub fn parse(toml_str: &str) -> Result<Value, toml::de::Error> {
        from_str(toml_str)
    }

    /// Convert TOML value to string
    pub fn stringify(value: &Value) -> Result<String, toml::ser::Error> {
        to_string(value)
    }

    /// Get value from TOML table by key
    pub fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
        value.get(key)
    }

    /// Set value in TOML table
    pub fn set(value: &mut Value, key: &str, new_value: Value) {
        if let Some(table) = value.as_table_mut() {
            table.insert(key.to_string(), new_value);
        }
    }

    /// Check if TOML value has a key
    pub fn has_key(value: &Value, key: &str) -> bool {
        value.get(key).is_some()
    }

    /// Get all keys from TOML table
    pub fn keys(value: &Value) -> Vec<String> {
        match value {
            Value::Table(table) => table.keys().cloned().collect(),
            _ => vec![],
        }
    }

    /// Create empty TOML table
    pub fn table() -> Value {
        Value::Table(toml::Table::new())
    }

    /// Create empty TOML array
    pub fn array() -> Value {
        Value::Array(vec![])
    }
}

/// Native XML functions for Copper language
/// Native XML functions for Copper language
pub mod xml {
    use quick_xml::{
        escape::{escape, unescape}, events::Event, Reader
    };
    use std::collections::HashMap;

    /// Simple XML element structure
    #[derive(Debug, Clone)]
    pub struct XmlElement {
        pub name: String,
        pub attributes: HashMap<String, String>,
        pub text: String,
        pub children: Vec<XmlElement>,
    }

    /// Parse XML string into a simplified structure
    pub fn parse(xml_str: &str) -> Result<XmlElement, Box<dyn std::error::Error + Send + Sync>> {
        let mut reader = Reader::from_str(xml_str);
        // Recent versions use config_mut()
        reader.config_mut().trim_text(true);

        let mut root = XmlElement {
            name: "copper-xml-reader".to_string(),
            attributes: HashMap::new(),
            text: String::new(),
            children: vec![],
        };

        let mut buf = Vec::<u8>::new();
        let mut current_element: Option<XmlElement> = None;
        let mut element_stack: Vec<XmlElement> = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();

                    let mut attributes = HashMap::new();
                    for attr in e.attributes() {
                        let attr = attr?;
                        let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                        let value = attr.unescape_value()?.into_owned();
                        attributes.insert(key, value);
                    }

                    let element = XmlElement {
                        name,
                        attributes,
                        text: String::new(),
                        children: vec![],
                    };

                    if let Some(parent) = current_element.take() {
                        element_stack.push(parent);
                    }
                    current_element = Some(element);
                }

                Ok(Event::Empty(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();

                    let mut attributes = HashMap::new();
                    for attr in e.attributes() {
                        let attr = attr?;
                        let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                        let value = attr.unescape_value()?.into_owned();
                        attributes.insert(key, value);
                    }

                    let element = XmlElement {
                        name,
                        attributes,
                        text: String::new(),
                        children: vec![],
                    };

                    if let Some(mut cur) = current_element.take() {
                        cur.children.push(element);
                        current_element = Some(cur);
                    } else if root.name == "copper-xml-reader"
                        && root.children.is_empty()
                        && root.text.is_empty()
                    {
                        root = element;
                    } else {
                        root.children.push(element);
                    }
                }

                Ok(Event::End(_)) => {
                    if let Some(element) = current_element.take() {
                        if let Some(mut parent) = element_stack.pop() {
                            parent.children.push(element);
                            current_element = Some(parent);
                        } else {
                            root = element;
                        }
                    }
                }

                Ok(Event::Text(e)) => {
                    if let Some(ref mut element) = current_element {
                        let raw = str::from_utf8(e.as_ref())?;           // BytesText -> &str
                        let text = unescape(raw)?.into_owned();           // Now it works
                        element.text.push_str(&text);
                    }
                }

                Ok(Event::CData(e)) => {
                    if let Some(ref mut element) = current_element {
                        // Keep content literally
                        element.text.push_str(&String::from_utf8_lossy(e.as_ref()));
                    }
                }

                Ok(Event::Comment(_)) | Ok(Event::Decl(_)) | Ok(Event::PI(_)) | Ok(Event::DocType(_)) => {
                    // Ignored
                }
                Ok(Event::GeneralRef(_)) => {
                    // Ignored (general reference, like <!ENTITY ...>)
                }

                Ok(Event::Eof) => break,

                Err(e) => return Err(Box::new(e)),
            }

            buf.clear();
        }

        // Close remaining elements in stack (possibly malformed XML)
        while let Some(mut parent) = element_stack.pop() {
            if let Some(child) = current_element.take() {
                parent.children.push(child);
            }
            current_element = Some(parent);
        }
        if let Some(final_top) = current_element {
            root = final_top;
        }

        Ok(root)
    }

    /// Convert XML element to string (recursive, with escaping)
    pub fn stringify(element: &XmlElement) -> String {
        fn write_el(el: &XmlElement, out: &mut String) {
            out.push('<');
            out.push_str(&el.name);
            for (k, v) in &el.attributes {
                out.push(' ');
                out.push_str(k);
                out.push_str("=\"");
                use std::fmt::Write as _;
                let _ = write!(out, "{}", escape(v));
                out.push('"');
            }

            let has_children = !el.children.is_empty();
            let has_text = !el.text.is_empty();

            if !has_children && !has_text {
                out.push_str("/>");
                return;
            }

            out.push('>');

            if has_text {
                use std::fmt::Write as _;
                let _ = write!(out, "{}", escape(&el.text));
            }

            for c in &el.children {
                write_el(c, out);
            }

            out.push_str("</");
            out.push_str(&el.name);
            out.push('>');
        }

        let mut s = String::new();
        write_el(element, &mut s);
        s
    }

    /// Create empty XML element
    pub fn element(name: &str) -> XmlElement {
        XmlElement {
            name: name.to_string(),
            attributes: HashMap::new(),
            text: String::new(),
            children: vec![],
        }
    }

    /// Set attribute on XML element
    pub fn set_attribute(element: &mut XmlElement, key: &str, value: &str) {
        element
            .attributes
            .insert(key.to_string(), value.to_string());
    }

    /// Get attribute from XML element
    pub fn get_attribute<'a>(element: &'a XmlElement, key: &str) -> Option<&'a String> {
        element.attributes.get(key)
    }

    /// Set text content of XML element
    pub fn set_text(element: &mut XmlElement, text: &str) {
        element.text = text.to_string();
    }

    /// Add child element
    pub fn add_child(parent: &mut XmlElement, child: XmlElement) {
        parent.children.push(child);
    }

    /// Find child by name
    pub fn find_child<'a>(element: &'a XmlElement, name: &str) -> Option<&'a XmlElement> {
        element.children.iter().find(|child| child.name == name)
    }

    /// Find all children by name
    pub fn find_children<'a>(element: &'a XmlElement, name: &str) -> Vec<&'a XmlElement> {
        element
            .children
            .iter()
            .filter(|child| child.name == name)
            .collect()
    }
}
