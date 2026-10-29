<#
.SYNOPSIS
Builds the Chocolatey package for a release.

.DESCRIPTION
Copies packaging/choco to a temporary folder, fills in the version, the x64
installer URL, and its SHA256, and runs `choco pack`. The source templates
are never modified.

Works in Windows PowerShell 5.1 and PowerShell 7. Requires choco.

.EXAMPLE
./scripts/release/pack-choco.ps1 -Version 0.1.0 `
    -Installer dist/DeskHand-0.1.0-x64-setup.exe `
    -Url https://github.com/brentdunham/deskhand/releases/download/v0.1.0/DeskHand-0.1.0-x64-setup.exe
#>
param(
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$Installer,
    [Parameter(Mandatory)][string]$Url,
    [string]$OutDir = 'dist'
)

$ErrorActionPreference = 'Stop'
$utf8 = New-Object System.Text.UTF8Encoding $false

if (-not (Test-Path $Installer)) { throw "installer not found: $Installer" }
if ($Url -notmatch '^https://') { throw "the package URL must be https: $Url" }

$source = (Resolve-Path (Join-Path $PSScriptRoot '../../packaging/choco')).Path
$work = Join-Path ([IO.Path]::GetTempPath()) ("deskhand-choco-" + [guid]::NewGuid())
Copy-Item $source $work -Recurse

try {
    $checksum = (Get-FileHash $Installer -Algorithm SHA256).Hash.ToLower()

    $install = Join-Path $work 'tools/chocolateyinstall.ps1'
    $text = [IO.File]::ReadAllText($install).Replace('__URL64__', $Url).Replace('__CHECKSUM64__', $checksum)
    if ($text -match '__[A-Z0-9]+__') { throw "unreplaced token '$($Matches[0])' in chocolateyinstall.ps1" }
    [IO.File]::WriteAllText($install, $text, $utf8)

    $nuspec = Join-Path $work 'deskhand.nuspec'
    $xml = [IO.File]::ReadAllText($nuspec)
    $xml = ([regex]'(<version>)[^<]+(</version>)').Replace($xml, "`${1}$Version`${2}", 1)
    [IO.File]::WriteAllText($nuspec, $xml, $utf8)

    New-Item -ItemType Directory -Force $OutDir | Out-Null
    $out = (Resolve-Path $OutDir).Path
    choco pack $nuspec --outputdirectory $out --limit-output
    if ($LASTEXITCODE) { throw "choco pack failed with exit code $LASTEXITCODE" }

    $package = Join-Path $out "deskhand.$Version.nupkg"
    if (-not (Test-Path $package)) { throw "expected $package was not produced" }
    "packed $package (installer sha256 $checksum)"
} finally {
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
