# Packaging CuePool

The [release workflow](../.github/workflows/release.yml) builds macOS and
Windows packages with:

```sh
cargo build --release --locked -p cuepool
# Windows adds ASIO support:
cargo build --release --locked -p cuepool --features asio
```

It produces a macOS Apple Silicon `.dmg` containing `CuePool.app`, plus a
Windows x86-64 portable `.zip` and `.msi` installer. Linux is built and tested
in CI but is not currently packaged by the release workflow.

The workflow verifies the candidate source and validates both platform packages
before publishing. A manual run without a release tag produces artifacts only.
See [releases](../docs/releases.md) for product versions, GitHub setup,
publication gates, and retries, and
[Contributing](../CONTRIBUTING.md#building-from-source) for build dependencies.

## macOS app bundle

Cargo produces a bare executable. Finder and the Dock take the app icon from
an `.app` bundle; launching the bare binary shows the generic `exec` icon.
winit window icons have no effect on macOS.

The release workflow assembles `dist/CuePool.app` using
[Info.plist.tmpl](../.github/packaging/Info.plist.tmpl) and `AppIcon.icns`.
It uses `dylibbundler` to copy the FFmpeg libraries into the bundle and rewrite
their load paths. Without that step, a local bundle still depends on the
build machine's Homebrew libraries. Install the bundler with:

```sh
brew install dylibbundler
```

After building the release binary, create a local `dist/CuePool.app` with:

```sh
./package-macos.sh
```

The script uses the same bundle template and icon as the release workflow.
Run it again after rebuilding the binary.

The app is ad-hoc signed, not notarized. If macOS blocks first launch, approve
it under **System Settings → Privacy & Security**.

## Windows packages

The workflow and local packaging use the same `package-windows.ps1` payload:
`cuepool.exe`, the DLLs from the pinned FFmpeg 8.0 SDK, and the x64 Visual C++
runtime from Visual Studio's redistributable directory. Packaging fails if
required runtimes are missing. It preserves CuePool's license files and the
matching FFmpeg `LICENSE` and `README.txt`, including the source commit and build
configuration, plus the ASIO SDK license and source URL from the build's
`CPAL_ASIO_DIR`. The Gyan FFmpeg distribution is GPL v3. The ASIO SDK offers GPL v3 or
the proprietary Steinberg agreement described in its notice. These dependency
terms are separate from CuePool's source licenses; packaging does not change
the licenses of CuePool's source files. Device-specific ASIO drivers are not
included.

After building the release binary with `--features asio`, run:

```powershell
.\package-windows.ps1
```

This writes `dist/cuepool` and `dist/cuepool-windows.zip`. It packages an existing
binary; it does not build CuePool. `-ZipPath` changes the ZIP destination. If
Visual Studio discovery is unavailable, pass `-VCRuntimeDir` pointing to the
x64 `Microsoft.VC*.CRT` directory under the build tools' `VC/Redist/MSVC` tree.
The script deliberately does not copy runtime files from `System32`.
[Microsoft's redistribution guidance](https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files)
describes that directory and its terms. App-local runtime updates ship with
CuePool updates.

### MSI installation and upgrades

The MSI installs for all users into `C:\Program Files\CuePool`, adds a common
Start-menu shortcut, and associates `.qproj` files with CuePool. It keeps the
existing `BlueJayLouche` publisher and UpgradeCode
`{F5075673-C9EF-5895-C78C-E5839C0E93D8}` so existing CuePool MSIs remain in the
same upgrade family. Repository relocation does not change installer identity.

From an elevated Windows terminal:

```powershell
msiexec /i cuepool-windows-x86_64.msi /qn /norestart /l*v install.log
msiexec /x cuepool-windows-x86_64.msi /qn /norestart /l*v uninstall.log
```

A higher product version replaces the previous MSI. Removal happens inside the
[Windows Installer transaction](https://docs.firegiant.com/wix/schema/wxs/majorupgrade/)
so a failed upgrade can restore the previous installation. Installing an older
MSI over a newer version is blocked. To roll back deliberately, stop CuePool,
uninstall the new package, and reinstall the retained previous MSI. Restore the
pre-upgrade project/settings backup if the newer application changed its format.
Do not rebuild or replace an already published version's installer.

Settings remain in `%APPDATA%\CuePool`; projects and media remain wherever the
operator stores them. The installer neither owns nor removes these files. Keep
show data outside the application directory, back it up before first launch,
and stop CuePool before upgrading. The MSI does not install Nodel or configure
its launcher. Selecting the packaged executable and its startup mode is a
separate deployment step.

Packages are unsigned until a release-signing credential is configured. MSI
packaging and WinGet metadata do not provide a publisher signature.

### Package verification

Run `pwsh .github/scripts/test-windows-packaging-tools.ps1` for the portable
packaging checks. The Windows release job also runs
`test-windows-package.ps1` on a disposable GitHub-hosted runner. It verifies:

- ZIP and installed executable startup with the SDK removed from `PATH`,
  expected version and ASIO support, and required app-local runtime files.
- MSI product/publisher/version, machine install path, shortcut, and association.
- Upgrade from a synthetic `0.0.0` package, removal of obsolete files, rejection
  of a downgrade, clean installation/uninstallation, and deliberate rollback by
  uninstalling the candidate and reinstalling the previous package.
- Byte-for-byte payload agreement and preservation of settings/project sentinels.

Logs and `result.json` are uploaded separately as `windows-package-validation`.
The synthetic prior package proves installer mechanics, not compatibility with
an older application's project format. `--version` exercises the real Windows
loader without requiring a GPU or audio device; it does not prove ASIO routing,
video presentation, OSC or show playback. Those need an attended rig check.

See [WinGet submission](winget.md) for package identity and the publication steps.

## Icons

Icon slots picked up by the release workflow:

- `AppIcon.icns` — macOS bundle icon
- `icon.ico` — Windows Start-menu shortcut icon

Also here:

- `window-icon.png` — 64px, embedded in the binary and set as the winit window
  icon. Covers the Windows taskbar and Linux title bar, which the packaged
  `.ico` does not reach (the workflow builds a shortcut icon, not an embedded
  resource). macOS ignores window icons and reads `AppIcon.icns` instead.
- `cuepool-02-cue.svg`, `cuepool-02-cue-small.svg` — the mark itself, and the
  reduced form used at 24px and below once the counter stops resolving. These
  are the source of truth; the raster files above are generated from them.
