@echo off
setlocal EnableDelayedExpansion

echo ========================================
echo    Copper Language Diagnostic Tool
echo ========================================
echo.

echo [INFO] Checking installation status...
echo.

:: Check if running as admin
net session >nul 2>&1
if %errorLevel% equ 0 (
    echo [INFO] Running as Administrator: YES
    set "INSTALL_DIR=C:\Program Files\Copper"
    set "REG_KEY=HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment"
) else (
    echo [INFO] Running as Administrator: NO
    set "INSTALL_DIR=%USERPROFILE%\.copper"
    set "REG_KEY=HKCU\Environment"
)

echo [INFO] Expected installation directory: !INSTALL_DIR!
echo.

:: Check if installation directory exists
if exist "!INSTALL_DIR!" (
    echo [SUCCESS] Installation directory found: !INSTALL_DIR!
    
    :: Check executable
    if exist "!INSTALL_DIR!\bin\cforge.exe" (
        echo [SUCCESS] Executable found: !INSTALL_DIR!\bin\cforge.exe
        
        :: Get file info
        for %%F in ("!INSTALL_DIR!\bin\cforge.exe") do (
            echo [INFO] File size: %%~zF bytes
            echo [INFO] File date: %%~tF
        )
        
        :: Check if Cargo.toml was copied
        if exist "!INSTALL_DIR!\Cargo.toml" (
            echo [SUCCESS] Project metadata found: !INSTALL_DIR!\Cargo.toml
        ) else (
            echo [WARNING] Project metadata missing: !INSTALL_DIR!\Cargo.toml
        )
    ) else (
        echo [ERROR] Executable NOT found: !INSTALL_DIR!\bin\cforge.exe
    )
    
    :: List contents
    echo.
    echo [INFO] Installation directory contents:
    dir "!INSTALL_DIR!" /B 2>nul
    if exist "!INSTALL_DIR!\bin" (
        echo [INFO] Bin directory contents:
        dir "!INSTALL_DIR!\bin" /B 2>nul
    )
) else (
    echo [ERROR] Installation directory NOT found: !INSTALL_DIR!
)

echo.
echo [INFO] Checking PATH configuration...

:: Check system PATH
echo [INFO] Checking PATH in registry...
for /f "tokens=2*" %%A in ('reg query "!REG_KEY!" /v PATH 2^>nul') do (
    set "CURRENT_PATH=%%B"
    echo [INFO] Current PATH: !CURRENT_PATH!
    
    echo !CURRENT_PATH! | findstr /C:"!INSTALL_DIR!\bin" >nul
    if !errorLevel! equ 0 (
        echo [SUCCESS] Copper bin directory found in PATH
    ) else (
        echo [ERROR] Copper bin directory NOT found in PATH
    )
)

:: Check current session PATH
echo.
echo [INFO] Current session PATH:
echo %PATH% | findstr /C:"Copper" >nul
if %errorLevel% equ 0 (
    echo [SUCCESS] Copper found in current session PATH
) else (
    echo [ERROR] Copper NOT found in current session PATH
)

:: Check COPPER_PATH
echo.
echo [INFO] Checking COPPER_PATH...
for /f "tokens=2*" %%A in ('reg query "!REG_KEY!" /v COPPER_PATH 2^>nul') do (
    echo [SUCCESS] COPPER_PATH set to: %%B
)

:: Test if cforge command works
echo.
echo [INFO] Testing cforge command...
where cforge >nul 2>&1
if %errorLevel% equ 0 (
    echo [SUCCESS] cforge command found in PATH
    where cforge
    
    echo [INFO] Testing cforge execution...
    cforge --version 2>nul
    if !errorLevel! equ 0 (
        echo [SUCCESS] cforge executes successfully
    ) else (
        echo [ERROR] cforge found but failed to execute
    )
) else (
    echo [ERROR] cforge command NOT found in PATH
    
    :: Try direct execution
    if exist "!INSTALL_DIR!\bin\cforge.exe" (
        echo [INFO] Trying direct execution...
        "!INSTALL_DIR!\bin\cforge.exe" --version 2>nul
        if !errorLevel! equ 0 (
            echo [SUCCESS] Direct execution works - PATH issue
        ) else (
            echo [ERROR] Direct execution also fails - executable issue
        )
    )
)

echo.
echo ========================================
echo           Diagnostic Complete
echo ========================================
echo.

if exist "!INSTALL_DIR!\bin\cforge.exe" (
    echo SOLUTION STEPS:
    echo 1. Close this terminal
    echo 2. Open a new terminal (Command Prompt or PowerShell)
    echo 3. Try: cforge --help
    echo.
    echo If still not working:
    echo 1. Restart your computer
    echo 2. Try again
    echo.
    echo Manual test: "!INSTALL_DIR!\bin\cforge.exe" --help
) else (
    echo PROBLEM: Installation not found or incomplete
    echo SOLUTION: Run the installer again
)

echo.
pause