# Downloads holst.exe and holst-mcp.exe from a GitHub release and installs them (Windows, x86_64).
#   $env:HOLST_VERSION      release tag, e.g. v0.1.0 (default: the latest release)
#   $env:HOLST_INSTALL_DIR  target directory (default: ~\.local\bin)
#Requires -Version 5.1
[Diagnostics.CodeAnalysis.SuppressMessageAttribute(
    'PSAvoidUsingWriteHost', '', Justification = 'an installer talks to the console in colour, not to the pipeline')]
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repo = 'easymikey/holst-rs'
$fallback = "cargo install --git https://github.com/$repo --locked holst-cli holst-mcp"

function Write-Step([string] $Text) {
    Write-Host '==> ' -ForegroundColor Green -NoNewline
    Write-Host $Text
}

function Exit-Install([string] $Text) {
    Write-Host 'error: ' -ForegroundColor Red -NoNewline
    Write-Host $Text
    Write-Host "to build from source instead (needs Rust, https://rustup.rs): $fallback"
    exit 1
}

if ([Environment]::Is64BitOperatingSystem -eq $false -or $env:PROCESSOR_ARCHITECTURE -notin @('AMD64')) {
    if ($env:PROCESSOR_ARCHITEW6432 -ne 'AMD64') {
        Exit-Install "no holst build for $env:PROCESSOR_ARCHITECTURE"
    }
}

$base = if ($env:HOLST_VERSION) {
    "https://github.com/$repo/releases/download/$($env:HOLST_VERSION)"
}
else {
    "https://github.com/$repo/releases/latest/download"
}
$dir = if ($env:HOLST_INSTALL_DIR) { $env:HOLST_INSTALL_DIR } else { Join-Path $HOME '.local\bin' }
$archive = 'holst-x86_64-pc-windows-msvc.zip'
$tmp = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $tmp | Out-Null

try {
    Write-Step "downloading $archive"
    try {
        Invoke-WebRequest -Uri "$base/$archive" -OutFile (Join-Path $tmp $archive)
        Invoke-WebRequest -Uri "$base/checksums.txt" -OutFile (Join-Path $tmp 'checksums.txt')
    }
    catch {
        Exit-Install "cannot download from ${base}: $($_.Exception.Message)"
    }

    $line = Get-Content (Join-Path $tmp 'checksums.txt') |
        Where-Object { ($_ -split '\s+')[1] -eq $archive } |
        Select-Object -First 1
    if (-not $line) { Exit-Install "no checksum for $archive in checksums.txt" }
    $expected = ($line -split '\s+')[0]
    $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $tmp $archive)).Hash
    if ($expected -ne $actual) { Exit-Install "checksum mismatch for $archive" }

    Expand-Archive -Path (Join-Path $tmp $archive) -DestinationPath $tmp -Force
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    Copy-Item (Join-Path $tmp 'holst.exe') (Join-Path $dir 'holst.exe') -Force
    Copy-Item (Join-Path $tmp 'holst-mcp.exe') (Join-Path $dir 'holst-mcp.exe') -Force

    $mcp = Join-Path $dir 'holst-mcp.exe'
    Write-Step "installed: $(Join-Path $dir 'holst.exe'), $mcp"
    $onPath = ($env:PATH -split [IO.Path]::PathSeparator) |
        Where-Object { $_ -and ($_.TrimEnd('\', '/') -eq $dir.TrimEnd('\', '/')) }
    if (-not $onPath) {
        Write-Host 'note: ' -ForegroundColor Yellow -NoNewline
        Write-Host "$dir is not on PATH: add it in System Properties > Environment Variables, then open a new terminal"
    }
    Write-Host 'next:'
    Write-Host '  holst auth login'
    Write-Host "  claude mcp add holst-rs -s user -- `"$mcp`""
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
