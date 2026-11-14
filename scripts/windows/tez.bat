@echo off
setlocal
rem tez launcher for Windows.
rem Usage: tez.bat [--dev]
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0tez.ps1" %*
