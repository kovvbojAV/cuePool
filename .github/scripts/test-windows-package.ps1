# Destructive installer rehearsal for a disposable GitHub-hosted Windows runner.
param(
    [Parameter(Mandatory)] [string]$Msi,
    [Parameter(Mandatory)] [string]$Zip,
    [Parameter(Mandatory)] [string]$Version,
    [Parameter(Mandatory)] [string]$SourceDir,
    [string]$LogDir = 'package-validation'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'Run this installer rehearsal only on a disposable GitHub-hosted Windows runner.'
}
$Msi = (Resolve-Path $Msi).Path
$Zip = (Resolve-Path $Zip).Path
$SourceDir = (Resolve-Path $SourceDir).Path
$LogDir = [IO.Path]::GetFullPath($LogDir)
$installDir = Join-Path $env:ProgramFiles 'CuePool'
$profileDir = Join-Path $env:APPDATA 'CuePool'
$shortcut = Join-Path ([Environment]::GetFolderPath('CommonPrograms')) 'CuePool.lnk'
$uninstallRoot = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
if ((Test-Path $installDir) -or (Test-Path $profileDir) -or (Test-Path $shortcut) -or
    @(Get-ItemProperty "$uninstallRoot\*" | Where-Object DisplayName -eq 'CuePool').Count) {
    throw 'Refusing to overwrite an existing CuePool installation or user profile.'
}
New-Item -ItemType Directory -Force $LogDir, $profileDir | Out-Null
$work = Join-Path $env:RUNNER_TEMP ("cuepool-installer-" + [guid]::NewGuid())
New-Item -ItemType Directory $work | Out-Null
$settings = Join-Path $profileDir 'settings.json'
$project = Join-Path $work 'operator-show.qproj'
Set-Content $settings '{"recent_files":["operator-show.qproj"],"last_seen_release_notes":"0.0"}'
Set-Content $project '{"preservation_probe":"installer must not change projects"}'
$settingsHash = (Get-FileHash $settings).Hash
$projectHash = (Get-FileHash $project).Hash
$results = [ordered]@{ version = $Version; msi_sha256 = (Get-FileHash $Msi).Hash; zip_sha256 = (Get-FileHash $Zip).Hash; checks = @() }
$installer = New-Object -ComObject WindowsInstaller.Installer

function Read-MsiProperty([string]$path, [string]$name) {
    $database = $installer.OpenDatabase($path, 0)
    $view = $database.OpenView("SELECT ``Value`` FROM ``Property`` WHERE ``Property`` = '$name'")
    $view.Execute()
    $record = $view.Fetch()
    if (-not $record) { throw "Missing MSI property: $name" }
    $value = $record.StringData(1)
    $view.Close()
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($record) | Out-Null
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($view) | Out-Null
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($database) | Out-Null
    $value
}
function Invoke-Msi([string]$operation, [string]$package, [string]$label, [int]$expected = 0) {
    $log = Join-Path $LogDir "$label.log"
    $process = Start-Process "$env:SystemRoot\System32\msiexec.exe" -PassThru -Wait -ArgumentList @(
        $operation, "`"$package`"", '/qn', '/norestart', '/l*v', "`"$log`"")
    if ($process.ExitCode -ne $expected) { throw "$label returned $($process.ExitCode), expected $expected; see $log" }
    $results.checks += $label
}
function Assert-DataPreserved {
    if ((Get-FileHash $settings).Hash -ne $settingsHash -or (Get-FileHash $project).Hash -ne $projectHash) {
        throw 'Installer changed operator settings or project data'
    }
}
function Assert-Payload([string]$directory) {
    foreach ($source in Get-ChildItem $SourceDir -File) {
        $target = Join-Path $directory $source.Name
        if (-not (Test-Path $target) -or (Get-FileHash $target).Hash -ne (Get-FileHash $source.FullName).Hash) {
            throw "Missing or changed packaged file: $target"
        }
    }
}
function Test-Runtime([string]$directory, [string]$label) {
    # This verifies the executable's actual Windows loader and linked DLLs,
    # independent of the build SDK's PATH. Rig playback is a separate check.
    $start = [Diagnostics.ProcessStartInfo]::new((Join-Path $directory 'cuepool.exe'))
    $start.ArgumentList.Add('--version')
    $start.WorkingDirectory = $work
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.Environment['PATH'] = "$env:SystemRoot\System32;$env:SystemRoot"
    foreach ($name in 'FFMPEG_DIR', 'LIB', 'LIBPATH', 'INCLUDE') { $start.Environment.Remove($name) | Out-Null }
    $process = [Diagnostics.Process]::Start($start)
    if (-not $process.WaitForExit(30000)) { $process.Kill($true); throw "$label did not exit within 30 seconds" }
    $output = $process.StandardOutput.ReadToEnd() + $process.StandardError.ReadToEnd()
    Set-Content (Join-Path $LogDir "$label.txt") $output
    if ($process.ExitCode -ne 0 -or $output -notmatch ('(?m)^CuePool ' + [regex]::Escape($Version) + '(\s|$)') -or
        $output -notmatch '(?m)^ASIO: enabled\s*$') {
        throw "$label failed: exit=$($process.ExitCode), output=$output"
    }
    $results.checks += $label
}

$productCode = Read-MsiProperty $Msi 'ProductCode'
$upgradeCode = Read-MsiProperty $Msi 'UpgradeCode'
$results.product_code = $productCode
$results.upgrade_code = $upgradeCode
$results.publisher = Read-MsiProperty $Msi 'Manufacturer'
if ((Read-MsiProperty $Msi 'ProductVersion') -ne $Version -or
    (Read-MsiProperty $Msi 'ProductName') -ne 'CuePool' -or
    (Read-MsiProperty $Msi 'ALLUSERS') -ne '1' -or $results.publisher -ne 'BlueJayLouche') {
    throw 'MSI identity, scope or version does not match the release'
}
# A synthetic previous MSI exercises installer mechanics before the first
# public release exists. This is not evidence of project-format compatibility.
$previousDir = Join-Path $work 'previous'
New-Item -ItemType Directory $previousDir | Out-Null
Copy-Item "$SourceDir\*" $previousDir
Set-Content (Join-Path $previousDir 'obsolete-release-file.txt') 'removed by upgrade'
$previousMsi = Join-Path $work 'previous.msi'
& "$PSScriptRoot\make-msi.ps1" -Name CuePool -Version 0.0.0 -SourceDir $previousDir -Exe cuepool.exe -FileExt qproj -Out $previousMsi
$previousCode = Read-MsiProperty $previousMsi 'ProductCode'
if ((Read-MsiProperty $previousMsi 'UpgradeCode') -ne $upgradeCode -or $previousCode -eq $productCode) {
    throw 'UpgradeCode must be stable and ProductCode must change across releases'
}
try {
    $portable = Join-Path $work 'portable'
    Expand-Archive $Zip $portable
    Assert-Payload $portable
    foreach ($dll in 'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll', 'avcodec-62.dll', 'avformat-62.dll', 'avutil-60.dll') {
        if (-not (Test-Path (Join-Path $portable $dll))) { throw "Missing app-local runtime $dll" }
    }
    Test-Runtime $portable 'zip-runtime'
    Invoke-Msi /i $previousMsi 'install-previous'
    Invoke-Msi /i $Msi 'upgrade-to-candidate'
    if ((Test-Path "$uninstallRoot\$previousCode") -or (Test-Path (Join-Path $installDir 'obsolete-release-file.txt'))) {
        throw 'Upgrade left the previous product or obsolete payload installed'
    }
    $registered = Get-ItemProperty "$uninstallRoot\$productCode"
    if ($registered.DisplayVersion -ne $Version -or $registered.Publisher -ne 'BlueJayLouche') {
        throw 'Apps and Features identity does not match the release'
    }
    if (-not (Test-Path $shortcut)) { throw 'Missing machine-wide Start-menu shortcut' }
    $shell = New-Object -ComObject WScript.Shell
    if ($shell.CreateShortcut($shortcut).TargetPath -ne (Join-Path $installDir 'cuepool.exe')) { throw 'Shortcut points outside the installed package' }
    $association = (Get-Item 'Registry::HKEY_LOCAL_MACHINE\Software\Classes\CuePool.qproj\shell\open\command').GetValue('')
    if ($association -notmatch [regex]::Escape((Join-Path $installDir 'cuepool.exe')) -or $association -notmatch '"%1"') {
        throw "Invalid .qproj open command: $association"
    }
    Assert-Payload $installDir
    Assert-DataPreserved
    Test-Runtime $installDir 'msi-runtime'
    Invoke-Msi /i $previousMsi 'reject-downgrade' 1603
    Assert-Payload $installDir
    Invoke-Msi /x $Msi 'uninstall-candidate'
    if ((Test-Path "$installDir\cuepool.exe") -or (Test-Path "$uninstallRoot\$productCode") -or (Test-Path $shortcut)) {
        throw 'Uninstall left the executable, registration or shortcut installed'
    }
    Assert-DataPreserved
    Invoke-Msi /i $previousMsi 'rollback-reinstall-previous'
    if (-not (Test-Path "$installDir\obsolete-release-file.txt")) { throw 'Previous package was not restored' }
    Assert-DataPreserved
    Invoke-Msi /x $previousMsi 'remove-previous'
    Invoke-Msi /i $Msi 'clean-install-candidate'
    Assert-Payload $installDir
    Test-Runtime $installDir 'clean-msi-runtime'
    Invoke-Msi /x $Msi 'clean-uninstall-candidate'
    Assert-DataPreserved
    $results.result = 'passed'
} finally {
    foreach ($code in $productCode, $previousCode) {
        if (Test-Path "$uninstallRoot\$code") { Invoke-Msi /x $code 'cleanup' }
    }
    $results | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $LogDir 'result.json')
    Remove-Item $profileDir, $work -Recurse -Force
}
Write-Output "Windows package rehearsal passed; evidence: $LogDir"
