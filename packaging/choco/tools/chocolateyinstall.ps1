$ErrorActionPreference = 'Stop'

# The release workflow replaces __URL64__ and __CHECKSUM64__ with the signed
# x64 NSIS installer's GitHub release URL and its SHA256 before `choco pack`.
# ARM64 machines run the x64 build under emulation until Chocolatey ARM64
# support is added.
$packageArgs = @{
  packageName    = $env:ChocolateyPackageName
  fileType       = 'exe'
  url64bit       = '__URL64__'
  checksum64     = '__CHECKSUM64__'
  checksumType64 = 'sha256'
  silentArgs     = '/S'
  validExitCodes = @(0)
}

Install-ChocolateyPackage @packageArgs
