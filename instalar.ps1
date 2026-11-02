# Instalador do Voz para Windows
$ErrorActionPreference = "Stop"

$BinDir = "$HOME\.local\bin"
$RepoPadrao = "https://github.com/judhagsan/voz.git"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

if (Test-Path "$ScriptDir\.git") {
    $Destino = $ScriptDir
} elseif ($env:VOZ_DIR) {
    $Destino = $env:VOZ_DIR
} else {
    $Destino = "$HOME\voz"
}

Write-Host "==> Instalando Voz em $Destino..."

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "==> Rust (cargo) não encontrado. Instale via https://rustup.rs"
    exit 1
}

if (-not (Test-Path "$Destino\.git")) {
    Write-Host "==> Clonando repositório em $Destino..."
    git clone $RepoPadrao $Destino
}

if (-not (Test-Path $BinDir)) {
    New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
}

Write-Host "==> Compilando o binário voz..."
Push-Location $Destino
cargo build --release --quiet
Pop-Location

Copy-Item "$Destino\target\release\voz.exe" "$BinDir\voz.exe" -Force
Copy-Item "$Destino\voz.cmd" "$BinDir\voz.cmd" -Force

Write-Host "==> Registrando hooks nos agentes..."
python "$Destino\scripts\instalar_hooks.py" --destino "$Destino"

if (Test-Path "$BinDir\agy.exe") {
    if (-not (Test-Path "$BinDir\agy-bin.exe")) {
        Move-Item "$BinDir\agy.exe" "$BinDir\agy-bin.exe" -Force
    }
    $AgyCmd = @"
@echo off
setlocal enabledelayedexpansion
set "ARGS="
set "USE_API=0"
set "USE_OAUTH=0"
for %%A in (%*) do (
    if "%%~A"=="--api" (
        set "USE_API=1"
    ) else if "%%~A"=="--oauth" (
        set "USE_OAUTH=1"
    ) else (
        set "ARGS=!ARGS! %%A"
    )
)
if "!USE_API!"=="1" (
    if "%GEMINI_API_KEY%"=="" (
        echo Erro: Variavel de ambiente GEMINI_API_KEY nao configurada. >&2
        exit /b 1
    )
    "%~dp0agy-bin.exe" --settings "{\"modelProvider\":\"gemini\"}" !ARGS!
    exit /b %ERRORLEVEL%
)
if "!USE_OAUTH!"=="1" (
    "%~dp0agy-bin.exe" --settings "{\"modelProvider\":\"\"}" !ARGS!
    exit /b %ERRORLEVEL%
)
"%~dp0agy-bin.exe" !ARGS!
exit /b %ERRORLEVEL%
"@
    Set-Content -Path "$BinDir\agy.cmd" -Value $AgyCmd -Encoding ASCII
    Write-Host "==> Wrapper do agy instalado com sucesso em $BinDir\agy.cmd"
}

Write-Host ""
Write-Host "==> Configuração de Chaves de API no Windows:"
if ($env:GEMINI_API_KEY) {
    Write-Host "    [✓] Gemini: variável de ambiente GEMINI_API_KEY configurada."
} else {
    Write-Host "    [ ] Gemini (gratuito): https://aistudio.google.com/apikey"
    Write-Host "        Para configurar de forma permanente no PowerShell, execute:"
    Write-Host "        [System.Environment]::SetEnvironmentVariable('GEMINI_API_KEY', 'SUA_CHAVE', 'User')"
    Write-Host "        Dica: adicione GEMINI_API_KEY_2 para multiplicar a cota diária!"
}

if ($env:ELEVENLABS_API_KEY) {
    Write-Host "    [✓] ElevenLabs: chave configurada."
} else {
    Write-Host "    [-] ElevenLabs (opcional): https://elevenlabs.io"
    Write-Host "        [System.Environment]::SetEnvironmentVariable('ELEVENLABS_API_KEY', 'SUA_CHAVE', 'User')"
}

if ($env:OPENAI_API_KEY) {
    Write-Host "    [✓] OpenAI: chave configurada."
} else {
    Write-Host "    [-] OpenAI (opcional): https://platform.openai.com/api-keys"
    Write-Host "        [System.Environment]::SetEnvironmentVariable('OPENAI_API_KEY', 'SUA_CHAVE', 'User')"
}

Write-Host "    [✓] Síntese offline (System.Speech): pronta para uso offline sem chaves."

if ((Test-Path "$BinDir\agy-bin.exe") -and (Test-Path "$BinDir\agy.cmd")) {
    Write-Host "    [✓] Antigravity CLI (agy): wrapper ativo com suporte a --api e --oauth."
}

Write-Host ""
Write-Host "==> Instalação concluída no Windows! Teste no terminal:"
Write-Host "    voz"
Write-Host "    voz sintetizar `"Olá, tudo certo.`""
