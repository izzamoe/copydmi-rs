# install.ps1 — one-liner installer for copydmi (Windows)
#
# USAGE (PowerShell):
#   irm https://raw.githubusercontent.com/izzamoe/copydmi-rs/master/install.ps1 | iex
#
# Downloads the latest release binary from GitHub Releases, verifies its
# SHA-256 checksum against SHA256SUMS.txt, and installs it to
# %LOCALAPPDATA%\Programs\copydmi (added to the user PATH automatically).

$ErrorActionPreference = "Stop"

$Repo    = "izzamoe/copydmi-rs"
$BinName = "copydmi.exe"
$Asset   = "copydmi-windows-x86_64.exe"

Write-Host "== Installing copydmi ==" -ForegroundColor Cyan

if (-not [Environment]::Is64BitOperatingSystem) {
    Write-Error "Only 64-bit Windows is supported by this installer."
    exit 1
}

Write-Host "-- resolving latest release --"
$release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
$tag = $release.tag_name
if (-not $tag) {
    Write-Error "Could not determine latest release tag. Check https://github.com/$Repo/releases"
    exit 1
}
Write-Host "  latest release: $tag"

$baseUrl = "https://github.com/$Repo/releases/download/$tag"
$tmpDir = Join-Path $env:TEMP "copydmi-install-$(Get-Random)"
New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null

try {
    $assetPath = Join-Path $tmpDir $Asset
    Write-Host "-- downloading $Asset --"
    Invoke-WebRequest -Uri "$baseUrl/$Asset" -OutFile $assetPath

    $sumsPath = Join-Path $tmpDir "SHA256SUMS.txt"
    $haveSums = $true
    try {
        Invoke-WebRequest -Uri "$baseUrl/SHA256SUMS.txt" -OutFile $sumsPath
    } catch {
        Write-Warning "Could not fetch SHA256SUMS.txt, skipping checksum verification"
        $haveSums = $false
    }

    if ($haveSums) {
        Write-Host "-- verifying checksum --"
        $sumsContent = Get-Content $sumsPath
        $expectedLine = $sumsContent | Where-Object { $_ -match [regex]::Escape($Asset) }
        if (-not $expectedLine) {
            Write-Warning "No checksum entry found for $Asset, skipping verification"
        } else {
            $expected = ($expectedLine -split '\s+')[0]
            $actual = (Get-FileHash -Path $assetPath -Algorithm SHA256).Hash.ToLower()
            if ($expected.ToLower() -ne $actual) {
                Write-Error "Checksum mismatch! expected $expected, got $actual. Refusing to install."
                exit 1
            }
            Write-Host "  checksum OK ($actual)"
        }
    }

    $installDir = Join-Path $env:LOCALAPPDATA "Programs\copydmi"
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    $destPath = Join-Path $installDir $BinName
    Copy-Item -Path $assetPath -Destination $destPath -Force

    Write-Host "-- installed --"
    Write-Host "  $destPath"

    # Add to user PATH if not already present.
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($userPath -notlike "*$installDir*") {
        [Environment]::SetEnvironmentVariable("Path", "$userPath;$installDir", "User")
        Write-Host "  Added '$installDir' to your user PATH."
        Write-Host "  Restart your terminal (or log off/on) for PATH changes to take effect."
    } else {
        Write-Host "  '$installDir' is already on your PATH."
    }

    Write-Host ""
    Write-Host "Run 'copydmi --help' (in a new terminal) to get started." -ForegroundColor Green
}
finally {
    Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
}
