# Portable checks for packaging failures and generated WiX input; Windows MSI
# execution is covered separately by test-windows-package.ps1.
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/../..").Path
$work = Join-Path ([IO.Path]::GetTempPath()) ("cuepool-package-tools-" + [guid]::NewGuid())
$oldFfmpeg = $env:FFMPEG_DIR
$oldAsio = $env:CPAL_ASIO_DIR
function Assert([bool]$condition, [string]$message) { if (-not $condition) { throw $message } }
function global:wix {
    $global:cuepoolTestWxs = [xml](Get-Content $args[1] -Raw)
    Set-Content $args[-1] 'fake MSI; XML generation only'
    $global:LASTEXITCODE = 0
}
try {
    $sdk = Join-Path $work 'FFmpeg & SDK'
    $crt = Join-Path $work 'CRT'
    $asio = Join-Path $work 'ASIO'
    $binaryDir = Join-Path $work 'target/release'
    New-Item -ItemType Directory -Force $binaryDir, "$sdk/bin", $crt, "$asio/common", "$work/packaging" | Out-Null
    Copy-Item "$root/package-windows.ps1", "$root/LICENSE-MIT", "$root/LICENSE-APACHE" $work
    Set-Content "$binaryDir/cuepool.exe" 'executable fixture'
    Set-Content "$sdk/bin/avcodec-62.dll" 'DLL fixture'
    Set-Content "$sdk/LICENSE.txt" 'FFmpeg license fixture'
    Copy-Item "$root/packaging/windows-sources.md" "$work/packaging"
    $deps = Get-Content "$root/packaging/windows-dependencies.json" -Raw | ConvertFrom-Json
    $deps.ffmpeg.dll_sha256 = @{ 'avcodec-62.dll' = (Get-FileHash "$sdk/bin/avcodec-62.dll" -Algorithm SHA256).Hash.ToLower() }
    $deps | ConvertTo-Json -Depth 8 | Set-Content "$work/packaging/windows-dependencies.json"
    foreach ($name in 'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll') { Set-Content "$crt/$name" 'runtime fixture' }
    Set-Content "$asio/common/LICENSE.txt" 'ASIO license fixture'
    $env:CPAL_ASIO_DIR = $asio
    $env:FFMPEG_DIR = $sdk
    $zip = Join-Path $work 'custom/cuepool.zip'
    & "$work/package-windows.ps1" -VCRuntimeDir $crt -ZipPath $zip
    Assert (Test-Path $zip) 'Custom ZIP path was not created'
    $payload = Join-Path $work 'dist/cuepool'
    Assert (Test-Path "$payload/FFmpeg-LICENSE.txt") 'FFmpeg license missing'
    Assert (Test-Path "$payload/THIRD-PARTY-SOURCES.md") 'Source access index missing'
    Assert (Test-Path "$payload/windows-dependencies.json") 'Pinned dependency metadata missing'
    Assert (Test-Path "$payload/LICENSE-MIT") 'CuePool license missing'
    Set-Content "$payload/obsolete.dll" 'stale payload'
    & "$work/package-windows.ps1" -VCRuntimeDir $crt -ZipPath $zip
    Assert (-not (Test-Path "$payload/obsolete.dll")) 'Repackaging retained a stale DLL'
    $goodHash = (Get-FileHash $zip).Hash
    Set-Content "$sdk/bin/avcodec-62.dll" 'wrong SDK'
    $failed = $false
    try { & "$work/package-windows.ps1" -VCRuntimeDir $crt -ZipPath $zip } catch { $failed = $true }
    Assert $failed 'A different FFmpeg SDK was packaged under the pinned source notice'
    Assert ((Get-FileHash $zip).Hash -eq $goodHash) 'Wrong SDK replaced the last good ZIP'
    Set-Content "$sdk/bin/avcodec-62.dll" 'DLL fixture'
    Remove-Item "$crt/msvcp140.dll"
    $failed = $false
    try { & "$work/package-windows.ps1" -VCRuntimeDir $crt -ZipPath $zip } catch { $failed = $true }
    Assert $failed 'Missing VC++ runtime did not fail packaging'
    Assert ((Get-FileHash $zip).Hash -eq $goodHash) 'Failed packaging replaced the last good ZIP'
    # Exercise XML escaping with a literal ampersand in a payload path.
    $msiPayload = Join-Path $work 'MSI & payload'
    New-Item -ItemType Directory $msiPayload | Out-Null
    Copy-Item "$payload/cuepool.exe" $msiPayload
    & "$root/.github/scripts/make-msi.ps1" -Name CuePool -Version 0.12.1 -SourceDir $msiPayload -Exe cuepool.exe -FileExt qproj -Out "$work/check.msi"
    $package = $global:cuepoolTestWxs.Wix.Package
    Assert ($package.UpgradeCode -eq 'f5075673-c9ef-5895-c78c-e5839c0e93d8') 'Existing UpgradeCode changed'
    Assert ($package.MajorUpgrade.Schedule -eq 'afterInstallInitialize') 'Upgrade removal is outside the rollback transaction'
    $file = $global:cuepoolTestWxs.SelectSingleNode('//*[local-name()="File" and @Id="AppExe"]')
    Assert ($file.Source -eq (Join-Path $msiPayload 'cuepool.exe')) 'WiX file path was not escaped faithfully'
    Write-Output 'Packaging tool checks passed (Windows installer execution is a separate CI gate).'
} finally {
    $env:FFMPEG_DIR = $oldFfmpeg
    $env:CPAL_ASIO_DIR = $oldAsio
    Remove-Item Function:\wix
    Remove-Variable cuepoolTestWxs -Scope Global -ErrorAction SilentlyContinue
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
