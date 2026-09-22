# Generate manifests only from an already-public canonical release.
param(
    [Parameter(Mandatory)] [ValidatePattern('^\d+\.\d+\.\d+$')] [string]$Version,
    [string]$Out = 'winget-manifests'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$id = 'BlueJayLouche.CuePool'
$repository = 'https://github.com/kovvbojAV/cuePool'
$asset = 'cuepool-windows-x86_64.msi'
$url = "$repository/releases/download/v$Version/$asset"
$release = Invoke-RestMethod "https://api.github.com/repos/kovvbojAV/cuePool/releases/tags/v$Version"
if ($release.draft -or $release.prerelease -or $release.tag_name -ne "v$Version") {
    throw 'WinGet submission requires a public stable release.'
}
$work = Join-Path ([IO.Path]::GetTempPath()) ("cuepool-winget-" + [guid]::NewGuid())
New-Item -ItemType Directory $work | Out-Null
try {
    $msi = Join-Path $work $asset
    Invoke-WebRequest $url -OutFile $msi
    $hash = (Get-FileHash $msi -Algorithm SHA256).Hash
    $checksumFile = Join-Path $work 'SHA256SUMS'
    Invoke-WebRequest "$repository/releases/download/v$Version/SHA256SUMS" -OutFile $checksumFile
    $checksumLines = [regex]::Matches((Get-Content $checksumFile -Raw), '(?m)^([a-fA-F0-9]{64})  cuepool-windows-x86_64\.msi\r?$')
    if ($checksumLines.Count -ne 1 -or $checksumLines[0].Groups[1].Value -ne $hash) {
        throw 'Public MSI does not match the release checksum manifest.'
    }
    function Read-Property([string]$name) {
        $view = $null
        $record = $null
        try {
            $view = $database.OpenView("SELECT ``Value`` FROM ``Property`` WHERE ``Property`` = '$name'")
            [void]$view.Execute()
            $record = $view.Fetch()
            if (-not $record) { throw "Missing MSI property: $name" }
            $record.StringData(1)
        } finally {
            if ($null -ne $record) { [Runtime.InteropServices.Marshal]::FinalReleaseComObject($record) | Out-Null }
            if ($null -ne $view) {
                try { [void]$view.Close() } finally { [Runtime.InteropServices.Marshal]::FinalReleaseComObject($view) | Out-Null }
            }
        }
    }
    $installer = $null
    $database = $null
    try {
        $installer = New-Object -ComObject WindowsInstaller.Installer
        $database = $installer.OpenDatabase($msi, 0)
        $productCode = Read-Property 'ProductCode'
        $upgradeCode = Read-Property 'UpgradeCode'
        if ((Read-Property 'ProductVersion') -ne $Version -or
            (Read-Property 'ProductName') -ne 'CuePool' -or
            (Read-Property 'Manufacturer') -ne 'BlueJayLouche' -or
            (Read-Property 'ALLUSERS') -ne '1' -or
            $upgradeCode -ne '{F5075673-C9EF-5895-C78C-E5839C0E93D8}' -or
            $productCode -notmatch '^\{[0-9A-Fa-f-]{36}\}$') {
            throw 'Published MSI identity differs from the WinGet package contract.'
        }
    } finally {
        if ($null -ne $database) { [Runtime.InteropServices.Marshal]::FinalReleaseComObject($database) | Out-Null }
        if ($null -ne $installer) { [Runtime.InteropServices.Marshal]::FinalReleaseComObject($installer) | Out-Null }
    }
    $directory = Join-Path $Out "manifests/b/BlueJayLouche/CuePool/$Version"
    if (Test-Path $directory) { throw "Refusing to overwrite existing manifests: $directory" }
    New-Item -ItemType Directory -Force $directory | Out-Null
    @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.version.1.12.0.schema.json
PackageIdentifier: $id
PackageVersion: $Version
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.12.0
"@ | Set-Content (Join-Path $directory "$id.yaml") -Encoding utf8
    @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.defaultLocale.1.12.0.schema.json
PackageIdentifier: $id
PackageVersion: $Version
PackageLocale: en-US
Publisher: BlueJayLouche
PublisherUrl: https://github.com/kovvbojAV
PublisherSupportUrl: $repository/issues
PackageName: CuePool
PackageUrl: $repository
License: GPL-3.0
LicenseUrl: https://www.gnu.org/licenses/gpl-3.0.html
ShortDescription: Audio, video and lighting cue playback for live shows and installations.
ReleaseNotesUrl: $repository/releases/tag/v$Version
ManifestType: defaultLocale
ManifestVersion: 1.12.0
"@ | Set-Content (Join-Path $directory "$id.locale.en-US.yaml") -Encoding utf8
    @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.installer.1.12.0.schema.json
PackageIdentifier: $id
PackageVersion: $Version
InstallerType: wix
Scope: machine
UpgradeBehavior: install
FileExtensions:
- qproj
Installers:
- Architecture: x64
  InstallerUrl: $url
  InstallerSha256: $hash
  ProductCode: '$productCode'
  AppsAndFeaturesEntries:
  - DisplayName: CuePool
    Publisher: BlueJayLouche
    ProductCode: '$productCode'
    UpgradeCode: '$upgradeCode'
ManifestType: installer
ManifestVersion: 1.12.0
"@ | Set-Content (Join-Path $directory "$id.installer.yaml") -Encoding utf8
    Write-Output "Prepared manifests from public MSI $hash at $directory"
} finally {
    Remove-Item $work -Recurse -Force
}
