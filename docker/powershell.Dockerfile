# PowerShell check image: pwsh with PSScriptAnalyzer for install.ps1.
# mcr.microsoft.com/powershell has no arm64 build and its amd64 build crashes
# under Rosetta on Install-Module, so pwsh comes from the release tarball for
# the native architecture; the Rust base also lets install.ps1 run real cargo.
FROM rust:1.96.0-bookworm
ARG PWSH_VERSION=7.5.0
ARG TARGETARCH
RUN apt-get update \
 && apt-get install -y --no-install-recommends libicu72 \
 && rm -rf /var/lib/apt/lists/* \
 && arch=$(case "$TARGETARCH" in arm64) echo arm64 ;; *) echo x64 ;; esac) \
 && mkdir -p /opt/pwsh \
 && curl -fsSL "https://github.com/PowerShell/PowerShell/releases/download/v$PWSH_VERSION/powershell-$PWSH_VERSION-linux-$arch.tar.gz" \
    | tar -xz -C /opt/pwsh \
 && chmod +x /opt/pwsh/pwsh \
 && ln -s /opt/pwsh/pwsh /usr/local/bin/pwsh
RUN pwsh -NoLogo -NoProfile -Command \
    "Set-PSRepository PSGallery -InstallationPolicy Trusted; Install-Module PSScriptAnalyzer -Scope AllUsers -Force"
WORKDIR /work
