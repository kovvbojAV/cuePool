# Packages cuepool.exe + the FFmpeg DLLs from $env:FFMPEG_DIR\bin into a
# self-contained, shareable folder + zip, matching .github/workflows/release.yml.
# DLLs sit next to the exe so colleagues need no PATH setup. Re-run after a rebuild.
param(
    [string]$ZipPath = (Join-Path $PSScriptRoot 'dist/cuepool-windows.zip'),
    [string]$VCRuntimeDir
)
$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot   # repo root
$exe  = Join-Path $root 'target\release\cuepool.exe'
$ffmpeg = $env:FFMPEG_DIR
$dist = Join-Path $root 'dist'
$out = Join-Path $dist 'cuepool'
$zip = [IO.Path]::GetFullPath($ZipPath)
$suffix = [guid]::NewGuid().ToString('N')
$staging = Join-Path $dist "cuepool.$suffix.partial"
$zipStaging = "$zip.$suffix.partial.zip"
$outBackup = Join-Path $dist "cuepool.$suffix.previous"
$zipBackup = "$zip.$suffix.previous.zip"

if (-not (Test-Path $exe)) { throw "Build first: cargo build --release --locked -p cuepool --features asio. Missing: $exe" }
if (-not $env:CPAL_ASIO_DIR) { throw 'Set CPAL_ASIO_DIR to the ASIO SDK used for the ASIO-enabled Windows build.' }
$asioLicense = Join-Path $env:CPAL_ASIO_DIR 'common/LICENSE.txt'
if (-not (Test-Path $asioLicense -PathType Leaf)) { throw "Missing ASIO SDK license: $asioLicense" }
if (-not $ffmpeg) { throw "Set FFMPEG_DIR to the FFmpeg 8.0 shared SDK used for the build (see .github/workflows/release.yml)." }
$ffmpegBin = Join-Path $ffmpeg 'bin'
$dlls = @(Get-ChildItem $ffmpegBin -Filter '*.dll' -File -ErrorAction Stop)
if ($dlls.Count -eq 0) { throw "No FFmpeg DLLs found in: $ffmpegBin" }

# Use the redistributable directory shipped with the build tools, never an
# arbitrary System32 runtime that can differ from the compiler used to build.
if (-not $VCRuntimeDir) {
    $redist = $env:VCToolsRedistDir
    if (-not $redist) {
        $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
        $vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($LASTEXITCODE -ne 0 -or -not $vs) { throw 'Cannot locate Visual Studio C++ redistributable files; pass -VCRuntimeDir' }
        $redist = Join-Path $vs 'VC/Redist/MSVC/*'
    }
    $VCRuntimeDir = Get-Item (Join-Path $redist 'x64/Microsoft.VC*.CRT') |
        Sort-Object { [version](Get-Item (Join-Path $_.FullName 'vcruntime140.dll')).VersionInfo.FileVersion.Split(' ')[0] } -Descending |
        Select-Object -First 1 -ExpandProperty FullName
}
foreach ($name in 'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll') {
    if (-not $VCRuntimeDir -or -not (Test-Path (Join-Path $VCRuntimeDir $name) -PathType Leaf)) {
        throw "Missing redistributable runtime: $name (pass -VCRuntimeDir pointing to the x64 Microsoft.VC*.CRT directory)"
    }
}
$ffmpegLicense = Join-Path $ffmpeg 'LICENSE'
if (-not (Test-Path $ffmpegLicense -PathType Leaf)) { throw "Missing FFmpeg distribution license: $ffmpegLicense" }
New-Item -ItemType Directory -Force $staging, (Split-Path $zip -Parent) | Out-Null

try {
    Copy-Item $exe $staging
    Copy-Item $dlls.FullName $staging

    Copy-Item (Join-Path $VCRuntimeDir '*.dll') $staging
    Copy-Item (Join-Path $root 'LICENSE-MIT'), (Join-Path $root 'LICENSE-APACHE') $staging
    Copy-Item $ffmpegLicense (Join-Path $staging 'FFmpeg-LICENSE.txt')
    Copy-Item $asioLicense (Join-Path $staging 'Steinberg-ASIO-LICENSE.txt')
    Copy-Item (Join-Path $ffmpeg 'README.txt') (Join-Path $staging 'FFmpeg-README.txt')

    @"
cuepool (Windows)

Just run cuepool.exe. All required DLLs are in this folder.
Source and releases: https://github.com/kovvbojAV/cuePool
Settings: %APPDATA%\CuePool (shared with the installed version).
Keep projects and media outside this application directory.

FFmpeg 8.0 shared runtime: https://github.com/GyanD/codexffmpeg/releases/tag/8.0
See FFmpeg-README.txt for the matching source commit and build configuration.
See FFmpeg-LICENSE.txt for its separate terms.
Microsoft Visual C++ runtime is deployed app-local from the build tools' Redist directory.
Its updates are delivered with CuePool package updates.
ASIO SDK 2.3.4 source: https://download.steinberg.net/sdk_downloads/ASIO-SDK_2.3.4_2025-10-15.zip
ASIO SDK SHA256: d5ebf0c20dd2c5f43771fd0c1418f4b361bf52434ee670097cfa6b3a335e2eca
See Steinberg-ASIO-LICENSE.txt for its separate terms.
ASIO device drivers must be installed separately.
"@ | Out-File (Join-Path $staging 'README.txt') -Encoding utf8

    Compress-Archive -Path "$staging\*" -DestinationPath $zipStaging
} catch {
    Remove-Item $staging -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $zipStaging -Force -ErrorAction SilentlyContinue
    throw
}

$outBackedUp = $false
$zipBackedUp = $false
$newOutPublished = $false
$newZipPublished = $false
try {
    if (Test-Path $out) {
        Move-Item $out $outBackup
        $outBackedUp = $true
    }
    if (Test-Path $zip) {
        Move-Item $zip $zipBackup
        $zipBackedUp = $true
    }
    Move-Item $staging $out
    $newOutPublished = $true
    Move-Item $zipStaging $zip
    $newZipPublished = $true
} catch {
    if ($newOutPublished -and (Test-Path $out)) {
        Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue
    }
    if ($newZipPublished -and (Test-Path $zip)) {
        Remove-Item $zip -Force -ErrorAction SilentlyContinue
    }
    if ($outBackedUp -and (Test-Path $outBackup)) {
        Move-Item $outBackup $out
    }
    if ($zipBackedUp -and (Test-Path $zipBackup)) {
        Move-Item $zipBackup $zip
    }
    Remove-Item $staging -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $zipStaging -Force -ErrorAction SilentlyContinue
    throw
}

Remove-Item $outBackup -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $zipBackup -Force -ErrorAction SilentlyContinue
"Packaged: $out"
"Zip:      $zip ($([math]::Round((Get-Item $zip).Length/1MB,1)) MB)"
