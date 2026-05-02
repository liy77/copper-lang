@echo off
echo ========================================
echo      Copper Language Builder
echo ========================================
echo.

:: Change to the script's directory
cd /d "%~dp0"
echo [INFO] Working directory: %CD%

:: Check if Cargo.toml exists in current directory
if not exist "Cargo.toml" (
    echo [ERROR] Cargo.toml not found in current directory.
    echo Please run this script from the copper-lang project root directory.
    pause
    exit /b 1
)

:: Check if Rust is installed
where cargo >nul 2>&1
if %errorLevel% neq 0 (
    echo [ERROR] Cargo/Rust is not installed or not in PATH.
    echo Please install Rust from https://rustup.rs/ and try again.
    pause
    exit /b 1
)

echo [INFO] Building Copper Language...
echo.

:: Build in release mode
cargo build --release
if %errorLevel% neq 0 (
    echo [ERROR] Build failed.
    pause
    exit /b 1
)

echo.
echo [SUCCESS] Build completed successfully!
echo.
echo Executable location: target\release\cforge.exe
echo.
echo You can now:
echo 1. Run the installer (install.bat)
echo 2. Or use directly: target\release\cforge.exe
echo.
echo Examples:
echo   target\release\cforge.exe run main.crs
echo   target\release\cforge.exe -c -i main.crs
echo.
pause