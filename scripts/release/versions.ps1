<#
.SYNOPSIS
Checks or sets the DeskHand version in every file that carries it.

.DESCRIPTION
The version lives in four places: the Cargo workspace, tauri.conf.json,
app/package.json (and its lockfile), and the Chocolatey nuspec. They must
agree, and a release tag must match them.

Works in Windows PowerShell 5.1 and PowerShell 7.

.EXAMPLE
./scripts/release/versions.ps1
Prints each version; fails if they differ.

.EXAMPLE
./scripts/release/versions.ps1 -Expect 0.2.0
Also fails unless every version is 0.2.0. The release workflow passes the tag.

.EXAMPLE
./scripts/release/versions.ps1 -Set 0.2.0
Updates every file, then refreshes Cargo.lock and package-lock.json.
#>
param(
    [string]$Expect,
    [string]$Set
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$semver = '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$'
$utf8 = New-Object System.Text.UTF8Encoding $false

# Each pattern captures (prefix)(version)(suffix) for the first occurrence.
$sources = [ordered]@{
    'Cargo.toml'                    = '(?ms)(^\[workspace\.package\].*?^version\s*=\s*")([^"]+)(")'
    'app/src-tauri/tauri.conf.json' = '("version"\s*:\s*")([^"]+)(")'
    'app/package.json'              = '("version"\s*:\s*")([^"]+)(")'
    'packaging/choco/deskhand.nuspec' = '(<version>)([^<]+)(</version>)'
}

function Read-Versions {
    $found = [ordered]@{}
    foreach ($file in $sources.Keys) {
        $text = [IO.File]::ReadAllText((Join-Path $root $file))
        $m = [regex]::Match($text, $sources[$file])
        if (-not $m.Success) { throw "no version found in $file" }
        $found[$file] = $m.Groups[2].Value
    }
    return $found
}

# Runs a native command and returns its exit code. Windows PowerShell 5.1
# turns any stderr output from a native command into a terminating error
# when $ErrorActionPreference is Stop, and cargo and npm write ordinary
# progress to stderr, so judge native commands by exit code only.
function Invoke-Native([scriptblock]$Command) {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $Command 2>&1 | Out-Null
        return $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
    }
}

if ($Set) {
    if ($Set -notmatch $semver) { throw "'$Set' is not a semantic version (e.g. 0.2.0 or 0.2.0-beta.1)" }
    foreach ($file in $sources.Keys) {
        if ($file -eq 'app/package.json') { continue } # npm updates it with its lockfile below
        $path = Join-Path $root $file
        $text = [IO.File]::ReadAllText($path)
        $updated = ([regex]$sources[$file]).Replace($text, "`${1}$Set`${3}", 1)
        [IO.File]::WriteAllText($path, $updated, $utf8)
    }
    Push-Location (Join-Path $root 'app')
    try {
        if (Invoke-Native { npm version $Set --no-git-tag-version --allow-same-version }) {
            throw 'npm version failed'
        }
    } finally { Pop-Location }
    Push-Location $root
    try {
        # Rewrites the workspace crates' versions in Cargo.lock.
        if ((Invoke-Native { cargo update --workspace --offline }) -and
            (Invoke-Native { cargo update --workspace })) {
            throw 'cargo update failed'
        }
    } finally { Pop-Location }
}

$versions = Read-Versions
$versions.GetEnumerator() | ForEach-Object { '{0,-34} {1}' -f $_.Key, $_.Value }

$distinct = @($versions.Values | Sort-Object -Unique)
if ($distinct.Count -ne 1) { throw "versions differ: $($distinct -join ', ')" }
if ($distinct[0] -notmatch $semver) { throw "'$($distinct[0])' is not a semantic version" }
if ($Expect -and $distinct[0] -ne $Expect) {
    throw "files say $($distinct[0]) but the release is $Expect. Run: ./scripts/release/versions.ps1 -Set $Expect"
}
