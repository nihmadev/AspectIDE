@echo off
setlocal
cd /d "%~dp0.."
node apps\desktop\scripts\kill-dev-port.mjs 5173
if errorlevel 1 exit /b 1
pnpm dev