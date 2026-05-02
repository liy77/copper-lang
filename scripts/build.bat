@echo off
echo ========================================
echo      Copper Language Builder
echo ========================================
echo.

:: Hop from scripts/ up to the project root.
cd /d "%~dp0\.."
echo [INFO] Working directory: %CD%

:: Check if Cargo.toml exists in current directory
if not exist "Cargo.toml" (
    echo [ERROR] Cargo.toml not found.
    echo Please keep build.bat inside the scripts/ folder of the copper-lang project.
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
echo 1. Run the installer:    scripts\install.bat
echo 2. Or use it directly:   target\release\cforge.exe
echo.
echo Examples:
echo   target\release\cforge.exe run main.crs
echo   target\release\cforge.exe -c -i main.crs
echo.
pause