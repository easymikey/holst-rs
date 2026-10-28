# Builds holst and holst-mcp and installs them into Cargo's bin directory (Windows).
# No signing: Windows Credential Manager does not ask per binary.
#Requires -Version 5.1
[Diagnostics.CodeAnalysis.SuppressMessageAttribute(
    'PSAvoidUsingWriteHost', '', Justification = 'an installer talks to the console in colour, not to the pipeline')]
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Write-Step([string] $Text) {
    Write-Host '==> ' -ForegroundColor Green -NoNewline
    Write-Host $Text
}

function Write-Note([string] $Text) {
    Write-Host 'note: ' -ForegroundColor Yellow -NoNewline
    Write-Host $Text
}

function Exit-Install([string] $Text) {
    Write-Host 'error: ' -ForegroundColor Red -NoNewline
    Write-Host $Text
    exit 1
}

# Run cargo with the given arguments; stop on a non-zero exit code.
function Invoke-Cargo([string[]] $Arguments) {
    & cargo @Arguments
    if ($LASTEXITCODE -ne 0) {
        Exit-Install "cargo $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Exit-Install 'cargo not found: install Rust from https://rustup.rs, then open a new terminal'
}

Push-Location (Join-Path $PSScriptRoot "..")
try {
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME '.cargo' }
    $root = if ($env:CARGO_INSTALL_ROOT) { $env:CARGO_INSTALL_ROOT } else { $cargoHome }
    $bin = Join-Path $root 'bin'

    Write-Step 'building and installing holst'
    Invoke-Cargo @('install', '--path', 'crates/holst-cli', '--locked')
    Write-Step 'building and installing holst-mcp'
    Invoke-Cargo @('install', '--path', 'crates/holst-mcp', '--locked')

    $holst = Join-Path $bin 'holst.exe'
    $mcp = Join-Path $bin 'holst-mcp.exe'
    Write-Host ''
    Write-Step "installed: $holst, $mcp"
    $onPath = ($env:PATH -split [IO.Path]::PathSeparator) |
        Where-Object { $_ -and ($_.TrimEnd('\', '/') -eq $bin.TrimEnd('\', '/')) }
    if (-not $onPath) {
        Write-Note "$bin is not on PATH: add it in System Properties > Environment Variables, then open a new terminal"
    }
    Write-Host 'next:'
    Write-Host '  holst auth login'
    Write-Host "  claude mcp add holst-rs -s user -- `"$mcp`""
}
finally {
    Pop-Location
}
