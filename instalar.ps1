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

Write-Host ""
Write-Host "==> Instalação concluída no Windows! Teste no terminal:"
Write-Host "    voz"
Write-Host "    voz sintetizar `"Olá, tudo certo.`""
