@echo off
:: Activate the repo's pre-commit / pre-push hooks for this clone. Idempotent.

cd /d "%~dp0\.."

where git >nul 2>&1
if %errorLevel% neq 0 (
    echo [ERROR] git not found in PATH.
    exit /b 1
)

git config core.hooksPath .githooks
if %errorLevel% neq 0 (
    echo [ERROR] Failed to set core.hooksPath.
    exit /b 1
)

echo [SUCCESS] Hooks activated from .githooks/
echo   pre-commit  : cargo fmt --check + cargo clippy -D warnings
echo   pre-push    : cargo test
echo.
echo Bypass with `--no-verify` only if you really must:
echo   git commit --no-verify
echo   git push   --no-verify
