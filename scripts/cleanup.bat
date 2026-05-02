@echo off
echo ========================================
echo      Copper Project Cleanup
echo ========================================
echo.

:: Hop from scripts/ up to the project root.
cd /d "%~dp0\.."

echo [INFO] Cleaning up development artifacts in %CD%

:: Remove Rust build output
if exist "target" (
    echo [INFO] Removing target/ ...
    rmdir /s /q "target" 2>nul
    echo [SUCCESS] target/ removed
)

:: Remove generated transpilation output
if exist "dist" (
    echo [INFO] Removing dist/ ...
    rmdir /s /q "dist" 2>nul
    echo [SUCCESS] dist/ removed
)

:: Remove tokenizer/parser debug logs left in the project root
if exist "cforge_tokenizer_debug.log" (
    del /f /q "cforge_tokenizer_debug.log"
    echo [SUCCESS] cforge_tokenizer_debug.log removed
)

echo.
echo [INFO] Files retained for distribution:
echo   - Cargo.toml, Cargo.lock, build.rs   ^(Rust project metadata^)
echo   - src/                                ^(compiler source^)
echo   - scripts/                            ^(install/build/diagnose tools^)
echo   - docs/                               ^(installation guide^)
echo   - lson/, std/                         ^(runtime assets^)
echo   - properties.kson                     ^(project configuration^)
echo   - examples/                           ^(.crs sample programs^)
echo   - main.crs                            ^(default file for `cforge run`^)
echo.

echo [SUCCESS] Cleanup complete.
echo.
echo To create a distributable package:
echo   1. Archive this directory.
echo   2. Users extract it and run scripts\install.bat.
echo.
pause
