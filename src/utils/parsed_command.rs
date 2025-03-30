#[derive(Debug, Clone)]
pub struct ParsedCommand {
    pub(crate) name: String,
    pub(crate) args: Vec<String>,
    pub(crate) is_file: bool,
    pub(crate) is_dir: bool,
    pub(crate) is_valid: bool,
}

impl ParsedCommand {
    pub fn new(name: String, args: Vec<String>) -> Self {
        Self { name, args, is_file: false, is_dir: false, is_valid: true }
    }

    pub fn set_file(&mut self, is_file: bool) {
        self.is_file = is_file;
    }

    pub fn set_dir(&mut self, is_dir: bool) {
        self.is_dir = is_dir;
    }

    pub fn set_valid(&mut self, is_valid: bool) {
        self.is_valid = is_valid;
    }
}

pub struct ParsedCommands {
    pub(crate) commands: Vec<ParsedCommand>,
}

impl ParsedCommands {
    pub fn new() -> Self {
        Self { commands: Vec::new() }
    }

    pub fn add_command(&mut self, command: ParsedCommand) {
        self.commands.push(command);
    }

    pub fn get_command(&self, name: &str) -> Option<&ParsedCommand> {
        self.commands.iter().find(|cmd| cmd.name == name)
    }
}