# Submit CuePool to WinGet

Use the canonical public release at `https://github.com/kovvbojAV/cuePool`.
Publish and read back the verified MSI before creating the final manifest. Do
not submit a workflow artifact URL, a mutable `latest` URL, or an installer
hash/ProductCode taken from a different build.

Preserve the included notices and verify access to the exact corresponding
source and build information for the shipped dependencies before publication.
The FFmpeg README identifies its commit and bundled libraries; that inventory
and a source link alone do not establish that the complete corresponding
source is available. See [FFmpeg redistribution guidance](https://ffmpeg.org/legal.html)
and the bundled ASIO license. WinGet validation does not perform this check.

The intended identity preserves the MSI's existing publisher:

| Field | Value/source |
| --- | --- |
| PackageIdentifier | `BlueJayLouche.CuePool` |
| Publisher | `BlueJayLouche` (MSI Manufacturer) |
| PackageName | `CuePool` (MSI ProductName) |
| PackageVersion | Final MSI ProductVersion |
| PackageLocale | `en-US` |
| PackageUrl | `https://github.com/kovvbojAV/cuePool` |
| License | `GPL-3.0` for the Windows distribution with GPL FFmpeg/ASIO; CuePool source remains `MIT OR Apache-2.0` |
| ShortDescription | `Audio, video and lighting cue playback for live shows and installations.` |
| Architecture | `x64` |
| InstallerType | `wix` |
| Scope | `machine` |
| UpgradeBehavior | `install` |
| FileExtensions | `qproj` |
| InstallerUrl | Exact versioned release URL ending in `cuepool-windows-x86_64.msi` |
| InstallerSha256 | SHA256 of the MSI downloaded from that URL |
| ProductCode | ProductCode read from that same MSI |
| AppsAndFeaturesEntries | DisplayName `CuePool`, Publisher `BlueJayLouche`, the same ProductCode, UpgradeCode `{F5075673-C9EF-5895-C78C-E5839C0E93D8}` |

The version, public URL, hash and ProductCode remain release outputs; no final
manifest is committed here with placeholders. `package-validation/result.json`
records the tested MSI's version, SHA256, ProductCode and UpgradeCode. Compare
it with the public download before submission.

For the first submission, run **Validate WinGet submission** in GitHub Actions
with the published version (for example `0.13.0`). This workflow downloads the
public MSI and checksum manifest, verifies its actual product identity, creates
the three schema-1.12 manifests, and tests validation, installation and removal
through WinGet on a disposable Windows runner. Use the manifest files from its
`cuepool-winget-submission` artifact only after the job succeeds; the workflow
does not submit a PR. It also retains the WinGet diagnostic logs.

The same generation step can run on Windows with
`.github/scripts/prepare-winget.ps1 -Version <published-version>`. It requires a
public stable release and refuses to overwrite an existing manifest directory.
The manual steps below remain available with Microsoft's manifest tool.

1. Install Microsoft's manifest tool with `winget install Microsoft.WingetCreate`.
2. Run `wingetcreate new <exact-public-msi-url>` and use the metadata above.
   WinGet supplies standard MSI silent switches; no custom installer script or
   Cargo dependency is needed. The packaged runtime is app-local.
3. Save the version, installer and English locale manifests under
   `manifests/b/BlueJayLouche/CuePool/<version>/` in a fork of
   `microsoft/winget-pkgs`. Validate with `winget validate --manifest <directory>`.
4. On a disposable Windows machine, enable local manifests with
   `winget settings --enable LocalManifestFiles`, then run
   `winget install --manifest <directory> --silent --scope machine` and
   `winget uninstall --id BlueJayLouche.CuePool --exact --silent`. Check the
   installed version and the same machine path used by the MSI rehearsal.
5. Submit the manifests with `wingetcreate submit <directory>` or a PR to
   `microsoft/winget-pkgs`. Resolve automated installer/security validation and
   maintainer feedback, then verify the published source with
   `winget show --id BlueJayLouche.CuePool --exact --source winget`.

On an accepted exhibit installation, prevent broad upgrade commands from
changing it with `winget pin add --id BlueJayLouche.CuePool --exact --blocking`.
Remove the pin only for a scheduled, validated upgrade. WinGet does not replace
show acceptance or project/settings backups.

References: [manifest authoring](https://learn.microsoft.com/en-us/windows/package-manager/package/manifest),
[installer schema](https://github.com/microsoft/winget-pkgs/blob/master/doc/manifest/schema/1.12.0/installer.md),
[manifest submission](https://learn.microsoft.com/en-us/windows/package-manager/package/repository),
[local manifest validation](https://learn.microsoft.com/en-us/windows/package-manager/winget/validate),
[version pinning](https://learn.microsoft.com/en-us/windows/package-manager/winget/pinning).
