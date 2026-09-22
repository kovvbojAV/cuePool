# Builds a per-app .msi from a dist folder using the WiX dotnet tool.
# The MSI installs the folder to Program Files\<Name> and adds a Start-menu shortcut.
param(
    [Parameter(Mandatory)] [string]$Name,       # display name, e.g. "CuePool"
    [Parameter(Mandatory)] [ValidatePattern('^\d+\.\d+\.\d+$')] [string]$Version,    # numeric x.y.z
    [Parameter(Mandatory)] [string]$SourceDir,  # folder whose files get installed
    [Parameter(Mandatory)] [string]$Exe,        # exe filename inside SourceDir
    [string]$Icon,                              # optional .ico for the shortcut
    [string]$FileExt,                           # optional extension to open with this app (e.g. "qproj")
    [Parameter(Mandatory)] [string]$Out         # output .msi path
)
$ErrorActionPreference = 'Stop'
$parts = $Version.Split('.')
if ([int]$parts[0] -gt 255 -or [int]$parts[1] -gt 255 -or [int]$parts[2] -gt 65535) {
    throw 'MSI versions require major/minor <= 255 and patch <= 65535'
}
if (-not (Test-Path (Join-Path $SourceDir $Exe) -PathType Leaf)) { throw "Missing executable: $Exe" }
if (@(Get-ChildItem $SourceDir -Directory).Count) { throw 'MSI payload must be a flat directory' }
function Xml([string]$value) { [System.Security.SecurityElement]::Escape($value) }


if (-not (Get-Command wix -ErrorAction SilentlyContinue)) {
    # ponytail: pinned to v5 — WiX v6+ requires accepting the OSMF EULA (WIX7015)
    dotnet tool install --global wix --version 5.0.2 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "WiX installation failed" }
}

# Preserve this namespace: existing rustjay-era MSIs must remain upgradeable.
$md5 = [System.Security.Cryptography.MD5]::Create()
$upgrade = [Guid]::new($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes("rustjay-msi:$Name")))

$iconXml = ''
$shortcutIcon = ''
$iconFileComponent = ''
$progIdIcon = ''
if ($Icon -and (Test-Path $Icon)) {
    $iconXml = "<Icon Id=`"AppIcon`" SourceFile=`"$(Xml (Resolve-Path $Icon).Path)`" />"
    $shortcutIcon = ' Icon="AppIcon"'
    # For a non-advertised ProgId, Icon must reference an installed *file* holding
    # the icon (not the shortcut's Icon-table entry) — ship the .ico next to the exe.
    $iconFileComponent = "        <Component Id=`"AppIconComponent`"><File Id=`"AppIconFile`" Source=`"$(Xml (Resolve-Path $Icon).Path)`" /></Component>"
    $progIdIcon = ' Icon="AppIconFile"'
}

# Explorer double-click association: ProgId + open verb nested in the exe's
# component. "&quot;%1&quot;" is the WiX quoting idiom for the clicked file's path.
$fileAssoc = ''
if ($FileExt) {
    $fileAssoc = @"
<ProgId Id="$Name.$FileExt" Description="$Name Project File"$progIdIcon>
          <Extension Id="$FileExt">
            <Verb Id="open" Command="Open" TargetFile="AppExe" Argument="&quot;%1&quot;" />
          </Extension>
        </ProgId>
"@
}

$components = (Get-ChildItem $SourceDir -File | Sort-Object Name | ForEach-Object {
    if ($_.Name -eq $Exe) {
        "        <Component><File Id=`"AppExe`" Source=`"$(Xml $_.FullName)`" />$fileAssoc</Component>"
    } else {
        "        <Component><File Source=`"$(Xml $_.FullName)`" /></Component>"
    }
}) -join "`n"

$wxs = @"
<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">
  <Package Name="$(Xml $Name)" Manufacturer="BlueJayLouche" Version="$Version"
           UpgradeCode="$upgrade" Scope="perMachine">
    <MajorUpgrade Schedule="afterInstallInitialize" DowngradeErrorMessage="A newer version of $(Xml $Name) is already installed." />
    <Property Id="ARPURLINFOABOUT" Value="https://github.com/kovvbojAV/cuePool" />
    <Property Id="ARPNOMODIFY" Value="1" />
    <MediaTemplate EmbedCab="yes" />
    $iconXml
    <StandardDirectory Id="ProgramFiles64Folder">
      <Directory Id="INSTALLFOLDER" Name="$Name">
$components
$iconFileComponent
      </Directory>
    </StandardDirectory>
    <StandardDirectory Id="ProgramMenuFolder">
      <Component Id="StartMenuShortcut">
        <Shortcut Id="AppShortcut" Name="$Name" Target="[INSTALLFOLDER]$Exe" WorkingDirectory="INSTALLFOLDER"$shortcutIcon />
        <RegistryValue Root="HKLM" Key="Software\BlueJayLouche\$Name" Name="installed"
                       Type="integer" Value="1" KeyPath="yes" />
      </Component>
    </StandardDirectory>
  </Package>
</Wix>
"@

$wxsPath = Join-Path ([IO.Path]::GetTempPath()) ("cuepool-" + [guid]::NewGuid() + '.wxs')
try {
    Set-Content $wxsPath $wxs -Encoding utf8
    wix build $wxsPath -arch x64 -o $Out
    if ($LASTEXITCODE -ne 0) { throw "wix build failed" }
} finally {
    Remove-Item $wxsPath -Force -ErrorAction SilentlyContinue
}
"MSI: $Out ($([math]::Round((Get-Item $Out).Length/1MB,1)) MB)"
