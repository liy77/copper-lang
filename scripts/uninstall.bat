@echo off
setlocal EnableDelayedExpansion

echo ========================================
echo     Copper Language Uninstaller
echo ========================================
echo.

:: --------------------------------------------------------------------------
:: Detect install scope (global vs per-user) so we touch the right registry
:: hive and install path. Mirrors install.bat: admin -> HKLM + Program Files,
:: otherwise HKCU + %USERPROFILE%\.copper.
:: --------------------------------------------------------------------------
net session >nul 2>&1
if %errorLevel% equ 0 (
    set "INSTALL_DIR=C:\Program Files\Copper"
    set "REG_KEY=HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment"
    set "INSTALL_TYPE=global"
) else (
    set "INSTALL_DIR=%USERPROFILE%\.copper"
    set "REG_KEY=HKCU\Environment"
    set "INSTALL_TYPE=local"
)

echo [INFO] Detected !INSTALL_TYPE! installation
echo [INFO] Target directory: !INSTALL_DIR!
echo.

:: A global install requires admin. If we got here without it, bail out so we
:: don't half-uninstall.
if "!INSTALL_TYPE!"=="global" (
    net session >nul 2>&1
    if !errorLevel! neq 0 (
        echo [ERROR] Global uninstall requires administrator privileges.
        echo Please run this script from an elevated Command Prompt.
        pause
        exit /b 1
    )
)

:: --------------------------------------------------------------------------
:: 1. Remove the install directory tree.
:: --------------------------------------------------------------------------
echo [INFO] Removing installation directory...
if exist "!INSTALL_DIR!" (
    if exist "!INSTALL_DIR!\Cargo.toml"   del /q "!INSTALL_DIR!\Cargo.toml"
    if exist "!INSTALL_DIR!\bin"          rmdir /s /q "!INSTALL_DIR!\bin"
    if exist "!INSTALL_DIR!\lson"         rmdir /s /q "!INSTALL_DIR!\lson"
    if exist "!INSTALL_DIR!\std"          rmdir /s /q "!INSTALL_DIR!\std"
    if exist "!INSTALL_DIR!\uninstall.bat" del /q "!INSTALL_DIR!\uninstall.bat"
    rmdir /q "!INSTALL_DIR!" 2>nul
    if exist "!INSTALL_DIR!" (
        echo [WARNING] Some files remain in !INSTALL_DIR!
    ) else (
        echo [SUCCESS] Installation directory removed.
    )
) else (
    echo [INFO] Installation directory not found.
)
echo.

:: --------------------------------------------------------------------------
:: 2. Strip "%COPPER_PATH%\bin" from the persisted PATH. We do this with a
::    PowerShell one-liner because cmd's string substitution is brittle and
::    chokes on parentheses, semicolons, and `%` inside paths. PowerShell
::    splits on `;`, drops every entry whose unexpanded form is the Copper
::    bin marker, and rejoins.
:: --------------------------------------------------------------------------
echo [INFO] Removing %%COPPER_PATH%%\bin from PATH...

set "PS_HIVE=User"
if "!INSTALL_TYPE!"=="global" set "PS_HIVE=Machine"

powershell -NoProfile -Command ^
  "$hive='!PS_HIVE!';" ^
  "$path=[Environment]::GetEnvironmentVariable('PATH',$hive);" ^
  "if ($null -ne $path) {" ^
  "  $kept=$path.Split(';',[StringSplitOptions]::RemoveEmptyEntries) |" ^
  "        Where-Object { $_ -ne '%%COPPER_PATH%%\bin' -and $_ -ne ([Environment]::ExpandEnvironmentVariables('%%COPPER_PATH%%\bin')) };" ^
  "  [Environment]::SetEnvironmentVariable('PATH', ($kept -join ';'), $hive)" ^
  "}"

if !errorLevel! equ 0 (
    echo [SUCCESS] PATH cleaned.
) else (
    echo [WARNING] Could not clean PATH automatically; remove %%COPPER_PATH%%\bin manually.
)
echo.

:: --------------------------------------------------------------------------
:: 3. Drop the COPPER_PATH variable itself.
:: --------------------------------------------------------------------------
echo [INFO] Removing COPPER_PATH environment variable...
reg delete "!REG_KEY!" /v COPPER_PATH /f >nul 2>&1
if !errorLevel! neq 0 (
    echo [INFO] COPPER_PATH was already absent.
) else (
    echo [SUCCESS] COPPER_PATH removed.
)
echo.

echo [SUCCESS] Copper Language has been uninstalled.
echo Open a new terminal so the PATH change takes effect.
pause
