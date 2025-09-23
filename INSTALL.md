# Copper Language - Installation

This repository contains a universal installer for the Copper language that allows you to use the `cforge` command globally on your system.

## Prerequisites

- Windows 10/11
- Rust and Cargo installed ([Download here](https://rustup.rs/))

## Quick Installation

```bash
# Run the universal installer
install.bat
```

The installer automatically detects if you're running as administrator:
- **As Administrator**: Installs globally in `C:\Program Files\Copper` (for all users)
- **As Normal User**: Installs locally in `%USERPROFILE%\.copper` (for you only)

## Usage

After installation, restart your terminal and use:

```bash
# Compile and run
cforge run main.crs

# Run default file (main.crs)
cforge run

# Compile only
cforge -c -i main.crs

# Show help
cforge --help
```

## Included Tools

- **`install.bat`** - Universal installer
- **`diagnose.bat`** - Installation diagnostics
- **`build.bat`** - Manual project build
- **`cleanup.bat`** - Development files cleanup

## Command Examples

```bash
# Run main file
cforge run main.crs

# Run without specifying file (uses main.crs by default)
cforge run

# Compile only
cforge -c -i src/main.crs

# Compile with cleanup
cforge --clean -c -i main.crs

# Verbose mode
cforge run main.crs --verbose

# Custom output directory
cforge run myproject.crs -o custom_output
```

## Uninstallation

Run the uninstaller created during installation:

**Global Installation:**
```bash
# Run as administrator
"C:\Program Files\Copper\uninstall.bat"
```

**Local Installation:**
```bash
# Run normally
"%USERPROFILE%\.copper\uninstall.bat"
```

## File Structure

After installation, the following files will be available:

```
Copper/
├── bin/
│   └── cforge.exe          # Main executable
├── lson/                   # LSON parser
│   ├── win32/
│   └── linux/
├── std/                    # Standard library
│   └── import.crs
├── Cargo.toml              # Project metadata
└── uninstall.bat          # Uninstaller script
```

## Troubleshooting

### The `cforge` command is not recognized
1. Check if the installation completed successfully
2. Restart your command prompt/PowerShell
3. Check if PATH was configured correctly:
   ```bash
   echo %PATH%
   ```

### Permission error
- For global installation: Run prompt as administrator
- For local installation: Use normal user privileges

### Dependency issues
Check if Rust is installed:
```bash
cargo --version
rustc --version
```

## Project Configuration

Copper uses a `properties.kson` file for project configuration:

```kson
name = "MyProject"
version = "1.0.0"
edition = 2021

[dependencies]
serde_json = "1.0.120"
regex = "1.10.5"
ai_copper = { git = "https://github.com/CopperRS/ai_copper.git" }
```

## Contributing

To contribute to the project:

1. Fork the repository
2. Create a branch for your feature
3. Commit your changes
4. Open a Pull Request

## License

[Insert license information here]