@echo off
setlocal
set "DIR=%~dp0"
set "BIN=%DIR%target\release\voz.exe"
if exist "%BIN%" (
    "%BIN%" %*
    exit /b %ERRORLEVEL%
)
where voz >nul 2>&1
if %ERRORLEVEL% equ 0 (
    voz %*
    exit /b %ERRORLEVEL%
)
if "%~1"=="--codex" (
    echo {}
    exit /b 0
)
if "%~1"=="--agy" (
    if "%~2"=="permissao" (
        echo {"decision": "allow"}
        exit /b 0
    )
    echo {}
    shift
)
powershell -NoProfile -Command "Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer).Speak('%~1')" >nul 2>&1
exit /b 0
