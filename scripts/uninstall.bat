@echo off
rem Thin wrapper: forwards to the cross-platform uninstall.py living beside it.
rem Copied into the install dir next to uninstall.py so a double-click works.
setlocal
where python  >nul 2>nul && (python  "%~dp0uninstall.py" %* & exit /b)
where py      >nul 2>nul && (py       "%~dp0uninstall.py" %* & exit /b)
where python3 >nul 2>nul && (python3  "%~dp0uninstall.py" %* & exit /b)
echo [ERROR] Python 3 not found on PATH (tried python, py, python3). Install it and retry.
exit /b 1
