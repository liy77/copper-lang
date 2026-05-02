@echo off
echo ========================================
echo      Copper Project Cleanup
echo ========================================
echo.

echo [INFO] Cleaning up development files...

:: Remove build artifacts
if exist "target" (
    echo [INFO] Removing target directory...
    rmdir /s /q "target" 2>nul
    echo [SUCCESS] Target directory removed
)

:: Keep only essential files for distribution
echo.
echo [INFO] Essential files for distribution:
echo - install.bat (Universal installer)
echo - diagnose.bat (Diagnostic tool)
echo - build.bat (Build script)
echo - Cargo.toml (Project metadata)
echo - src/ (Source code)
echo - lson/ (LSON parser)
echo - std/ (Standard library)
echo - properties.kson (Project configuration)
echo - main.crs (Example file)
echo - *.crs (Copper source files)
echo.

echo [SUCCESS] Cleanup complete!
echo.
echo To create a distributable package:
echo 1. Archive this directory
echo 2. Users can extract and run install.bat
echo.
pause