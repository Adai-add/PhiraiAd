@echo off
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\build_all_new.ps1" %*
set "build_result=%errorlevel%"
echo.
pause
exit /b %build_result%
