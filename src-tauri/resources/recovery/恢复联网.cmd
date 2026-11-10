@echo off
rem Yunkuo offline network recovery. Runs the PowerShell script with the same base name; keep this file ASCII-only.
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dpn0.ps1" %*
