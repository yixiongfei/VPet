@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\start-vpet.ps1"
if errorlevel 1 pause
