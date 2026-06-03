@echo off
rem Thin wrapper: all install logic lives in install.py. Locate a Python 3
rem interpreter and forward every argument to it.
rem   scripts\install.bat            auto scope
rem   scripts\install.bat --local    per-user install
rem   scripts\install.bat --global   all-users install (run as Administrator)
setlocal
where python  >nul 2>nul && (python  "%~dp0install.py" %* & exit /b)
where py      >nul 2>nul && (py       "%~dp0install.py" %* & exit /b)
where python3 >nul 2>nul && (python3  "%~dp0install.py" %* & exit /b)
echo [ERROR] Python 3 not found on PATH (tried python, py, python3). Install it and retry.
exit /b 1
