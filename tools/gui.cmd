@echo off
rem Thin wrapper for tools\gui.ps1 so you can skip -ExecutionPolicy Bypass -File.
rem Usage: tools\gui.cmd dev^|run^|build^|check^|installer^|verify
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0gui.ps1" %*
exit /b %ERRORLEVEL%
