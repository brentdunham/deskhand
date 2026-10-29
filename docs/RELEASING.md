# Releasing DeskHand

## Cut a release

1. Set the version everywhere it appears, and commit:

   ```powershell
   ./scripts/release/versions.ps1 -Set 0.2.0
   git commit -am "release: 0.2.0"
   git push
   ```

   This updates `Cargo.toml`, `app/src-tauri/tauri.conf.json`, `app/package.json` and its lockfile, the Chocolatey nuspec, and `Cargo.lock`. Running it with no arguments only checks that they agree.

2. Tag and push the tag:

   ```powershell
   git tag v0.2.0
   git push origin v0.2.0
   ```

   A hyphenated version (`v0.2.0-rc.1`) is marked as a prerelease.

3. The **Release** workflow then:
   - fails early unless the tag matches every version file;
   - runs the full CI suite (fmt, clippy, tests, x64 and ARM64 installers);
   - renames the installers to `DeskHand-<version>-<arch>-setup.exe`;
   - writes `SHA256SUMS.txt`;
   - builds `deskhand.<version>.nupkg` with the x64 installer's release URL and checksum filled in;
   - creates a **draft** GitHub release with all of the above and generated notes.

4. Review the draft on the Releases page and publish it. The Chocolatey package's download URL only works once the release is published.

To redo a failed release, delete the draft and the tag (`git push --delete origin v0.2.0`, `git tag -d v0.2.0`), fix, and tag again.

## Code signing (not set up yet)

Release installers are currently **unsigned**: the workflow warns, and the release notes tell testers how to get past SmartScreen. Signing is stubbed at the "Sign installers" step in `.github/workflows/release.yml`.

To enable it:

1. Get a signing service. For an open-source project, [SignPath Foundation](https://signpath.org) offers free certificates; it requires an application by the project owner. [Azure Trusted Signing](https://learn.microsoft.com/azure/trusted-signing/) is the paid alternative and requires identity validation.
2. Add the credentials as repository secrets (`SIGNPATH_API_TOKEN` for SignPath).
3. Replace the stub step with the provider's signing action. Sign the installers before the checksum step, so `SHA256SUMS.txt` and the Chocolatey checksum cover the signed files.
4. Ideally also sign `deskhand.exe` inside the installer, using Tauri's `bundle.windows.signCommand` during the build. Signing only the installer stops the SmartScreen warning at install time but leaves the installed binary unsigned.

The stub step fails the release if `SIGNPATH_API_TOKEN` exists but signing is not implemented, so a release can never be unsigned while appearing signed.

## Chocolatey

Testers can install from the release's `.nupkg` without the community repository:

```powershell
# In the folder containing deskhand.<version>.nupkg
choco install deskhand --source . --version <version>
```

`choco uninstall deskhand` runs the NSIS uninstaller silently.

Publishing to the community repository (`choco push deskhand.<version>.nupkg --source https://push.chocolatey.org/ --api-key <key>`) goes through human moderation. Do it only after signing is in place: unsigned installers draw scrutiny and trigger antivirus flags in the automated checks. The package targets x64; ARM64 machines run the x64 build under emulation until an ARM64 package is added.

## Local checks

```powershell
./scripts/release/versions.ps1                 # versions agree
./scripts/release/pack-choco.ps1 -Version 0.1.0 -Installer <path-to-setup.exe> -Url https://example.invalid/x.exe -OutDir $env:TEMP\choco-test
```
