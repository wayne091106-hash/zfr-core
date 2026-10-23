#requires -version 5.1
<#
.SYNOPSIS
  Install the latest ZFR Core release for Windows.
.DESCRIPTION
  Downloads zfr-x86_64-pc-windows-msvc.zip from GitHub Releases, installs it to
  %LOCALAPPDATA%\Programs\zfr, adds it to the user PATH, and (optionally, with
  -Service and an elevated shell) registers + starts the zfr-core Windows service.

  Usage:
    irm https://raw.githubusercontent.com/wayne091106-hash/zfr-core/main/install.ps1 | iex
    & ([scriptblock]::Create((irm https://raw.githubusercontent.com/wayne091106-hash/zfr-core/main/install.ps1))) -Service
#>
param(
    [switch]$Service,
    [string]$Tag = "latest"
)
$ErrorActionPreference = "Stop"
$repo = "wayne091106-hash/zfr-core"
$headers = @{ "User-Agent" = "zfr-install" }

if ($Tag -eq "latest") {
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/latest" -Headers $headers
} else {
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/tags/$Tag" -Headers $headers
}
$asset = $release.assets | Where-Object { $_.name -eq "zfr-x86_64-pc-windows-msvc.zip" } | Select-Object -First 1
if (-not $asset) { throw "zfr-x86_64-pc-windows-msvc.zip not found in release $($release.tag_name)" }

$dir = Join-Path $env:LOCALAPPDATA "Programs\zfr"
New-Item -ItemType Directory -Force -Path $dir | Out-Null

$tmp = Join-Path $env:TEMP "zfr-install-$([guid]::NewGuid().ToString('N')).zip"
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $tmp -Headers $headers
Expand-Archive -Path $tmp -DestinationPath $dir -Force
Remove-Item $tmp -ErrorAction SilentlyContinue

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (($userPath -split ";") -notcontains $dir) {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$dir", "User")
    $env:Path += ";$dir"
}

Write-Host "installed zfr $($release.tag_name) -> $dir"

$exe = Join-Path $dir "zfr.exe"
if ($Service) {
    & $exe service install
    & $exe service start
    Write-Host ""
    Write-Host "service 'zfr-core' installed and started (auto-start at boot, restart on failure)"
    Write-Host "manage it with:  zfr service status | start | stop | uninstall"
} else {
    Write-Host ""
    Write-Host "get started (new terminal so PATH applies, or use the full path):"
    Write-Host "  zfr serve --daemon        # background gateway on http://127.0.0.1:18788/v1"
    Write-Host "  zfr stop                  # stop it"
    Write-Host "  zfr --help                # everything else"
}
