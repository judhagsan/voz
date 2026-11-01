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

Write-Host "==> Instalação concluída no Windows!"
