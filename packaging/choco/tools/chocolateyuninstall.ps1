$ErrorActionPreference = 'Stop'

# The NSIS installer registers DeskHand under the standard Uninstall keys.
$keys = @(Get-UninstallRegistryKey -SoftwareName 'DeskHand')

if ($keys.Count -eq 0) {
    Write-Warning 'DeskHand is not registered as installed; nothing to uninstall.'
    return
}
if ($keys.Count -gt 1) {
    Write-Warning "Found $($keys.Count) DeskHand installs; uninstall them from Settings > Apps."
    $keys | ForEach-Object { Write-Warning "  $($_.DisplayName) $($_.DisplayVersion)" }
    return
}

# UninstallString is the quoted path to the NSIS uninstaller.
$uninstaller = $keys[0].UninstallString.Trim().Trim('"')

$packageArgs = @{
  packageName    = $env:ChocolateyPackageName
  fileType       = 'exe'
  file           = $uninstaller
  silentArgs     = '/S'
  validExitCodes = @(0)
}

Uninstall-ChocolateyPackage @packageArgs
