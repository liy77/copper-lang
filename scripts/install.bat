@echo off
setlocal EnableDelayedExpansion

echo ========================================
echo     Copper Language Installer
echo ========================================
echo.

:: This script lives in scripts/. Hop up to the project root so all
:: subsequent commands operate against Cargo.toml, src/, etc.
cd /d "%~dp0\.."

:: Check if Cargo.toml exists in current directory
if not exist "Cargo.toml" (
    echo [ERROR] Cargo.toml not found.
    echo Please keep install.bat inside the scripts/ folder of the copper-lang project.
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

echo [INFO] Checking Rust installation...
cargo --version
echo.

:: Check if running as administrator
net session >nul 2>&1
if %errorLevel% equ 0 (
    echo [INFO] Running as Administrator: YES
    echo [INFO] Will install globally for all users
    set "INSTALL_DIR=C:\Program Files\Copper"
    set "REG_KEY=HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment"
    set "INSTALL_TYPE=global"
) else (
    echo [INFO] Running as Administrator: NO
    echo [INFO] Will install locally for current user
    set "INSTALL_DIR=%USERPROFILE%\.copper"
    set "REG_KEY=HKCU\Environment"
    set "INSTALL_TYPE=local"
)

set "BIN_DIR=%INSTALL_DIR%\bin"
echo [INFO] Installation directory: !INSTALL_DIR!
echo.

:: Create installation directory
echo [INFO] Creating installation directory...
if not exist "!INSTALL_DIR!" (
    mkdir "!INSTALL_DIR!"
    if !errorLevel! neq 0 (
        echo [ERROR] Failed to create installation directory.
        pause
        exit /b 1
    )
)

if not exist "!BIN_DIR!" (
    mkdir "!BIN_DIR!"
    if !errorLevel! neq 0 (
        echo [ERROR] Failed to create bin directory.
        pause
        exit /b 1
    )
)

:: Build the project in release mode
:: Force a fresh release build so embedded metadata (build date, version)
:: is refreshed and any previous incremental cache cannot poison the install.
echo [INFO] Cleaning previous release build artifacts...
if exist "target\release\cforge.exe" del /f /q "target\release\cforge.exe"

echo [INFO] Building Copper in release mode...
cargo build --release
if %errorLevel% neq 0 (
    echo [ERROR] Failed to build the project.
    pause
    exit /b 1
)

:: Copy the executable
echo [INFO] Installing cforge executable...
copy "target\release\cforge.exe" "!BIN_DIR!\cforge.exe" >nul
if %errorLevel% neq 0 (
    echo [ERROR] Failed to copy executable.
    pause
    exit /b 1
)

:: Copy Cargo.toml for version detection
echo [INFO] Installing project metadata...
if exist "Cargo.toml" (
    copy "Cargo.toml" "!INSTALL_DIR!\Cargo.toml" >nul
)

:: Copy the lson directory
echo [INFO] Installing lson dependencies...
if exist "lson" (
    xcopy "lson" "!INSTALL_DIR!\lson" /E /I /Y >nul
    if !errorLevel! neq 0 (
        echo [ERROR] Failed to copy lson directory.
        pause
        exit /b 1
    )
)

:: Copy std directory
echo [INFO] Installing standard library...
if exist "std" (
    xcopy "std" "!INSTALL_DIR!\std" /E /I /Y >nul
    if !errorLevel! neq 0 (
        echo [ERROR] Failed to copy std directory.
        pause
        exit /b 1
    )
)

:: Set COPPER_PATH environment variable
echo [INFO] Setting COPPER_PATH environment variable...
reg add "!REG_KEY!" /v COPPER_PATH /t REG_SZ /d "!INSTALL_DIR!" /f >nul
if %errorLevel% neq 0 (
    echo [ERROR] Failed to set COPPER_PATH environment variable.
) else (
    echo [SUCCESS] COPPER_PATH set to !INSTALL_DIR!
)

:: Add COPPER_PATH\bin to PATH
echo [INFO] Adding %%COPPER_PATH%%\bin to PATH...

:: Get current PATH
for /f "tokens=2*" %%A in ('reg query "!REG_KEY!" /v PATH 2^>nul') do set "CURRENT_PATH=%%B"
if "!CURRENT_PATH!"=="" set "CURRENT_PATH= "

:: Check if COPPER_PATH\bin is already in PATH
echo !CURRENT_PATH! | findstr /C:"%%COPPER_PATH%%\bin" >nul
if %errorLevel% equ 0 (
    echo [INFO] %%COPPER_PATH%%\bin is already in PATH.
) else (
    echo [INFO] Adding %%COPPER_PATH%%\bin to PATH...
    if "!CURRENT_PATH!"==" " (
        set "NEW_PATH=%%COPPER_PATH%%\bin"
    ) else (
        set "NEW_PATH=!CURRENT_PATH!;%%COPPER_PATH%%\bin"
    )
    reg add "!REG_KEY!" /v PATH /t REG_EXPAND_SZ /d "!NEW_PATH!" /f >nul
    if !errorLevel! neq 0 (
        echo [ERROR] Failed to update PATH.
        echo [WARNING] You may need to add %%COPPER_PATH%%\bin to your PATH manually.
    ) else (
        echo [SUCCESS] Added %%COPPER_PATH%%\bin to PATH.
    )
)

:: Install uninstaller
:: We ship a hand-written uninstall.bat in scripts\ rather than generating one
:: from echo lines — the previous generator's escape soup was extremely fragile
:: and produced broken scripts ("0 was unexpected at this time"). The static
:: script auto-detects install scope at runtime, so no patching is needed.
echo [INFO] Installing uninstaller...
if exist "scripts\uninstall.bat" (
    copy /Y "scripts\uninstall.bat" "!INSTALL_DIR!\uninstall.bat" >nul
    if !errorLevel! neq 0 (
        echo [WARNING] Failed to install uninstaller. You can run it from scripts\uninstall.bat in the source tree.
    )
) else (
    echo [WARNING] scripts\uninstall.bat not found in source tree; uninstaller not installed.
)

echo.
echo ========================================
echo     Installation Complete!
echo ========================================
echo.
echo Installation type: !INSTALL_TYPE!
echo Installation directory: !INSTALL_DIR!
echo Executable location: !BIN_DIR!\cforge.exe
echo.
echo [SUCCESS] Copper Language (cforge) has been installed successfully!
echo.
echo IMPORTANT:
echo - Please restart your command prompt to apply PATH changes
echo - After restart, you can use 'cforge' command from anywhere
echo - To uninstall, run: !INSTALL_DIR!\uninstall.bat
if "!INSTALL_TYPE!"=="global" (
    echo   ^(as administrator^)
)
echo.
echo Usage examples:
echo   cforge run main.crs
echo   cforge -c -i main.crs
echo   cforge run myproject.crs -o custom_output
echo.
echo Press any key to exit...
pause >nul