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
function Get-CuePoolRegistrations {
    Get-ItemProperty "$uninstallRoot\*" | Where-Object {
        $_.PSObject.Properties['DisplayName'] -and $_.DisplayName -eq 'CuePool'
    }
}
if ((Test-Path $installDir) -or (Test-Path $profileDir) -or (Test-Path $shortcut) -or
    @(Get-CuePoolRegistrations).Count) {
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

function Read-MsiColumn([string]$path, [string]$query) {
    $database = $installer.OpenDatabase($path, 0)
    $view = $database.OpenView($query)
    try {
        [void]$view.Execute()
        while ($record = $view.Fetch()) {
            $record.StringData(1)
            [Runtime.InteropServices.Marshal]::FinalReleaseComObject($record) | Out-Null
        }
    } finally {
        [void]$view.Close()
        [Runtime.InteropServices.Marshal]::FinalReleaseComObject($view) | Out-Null
        [Runtime.InteropServices.Marshal]::FinalReleaseComObject($database) | Out-Null
    }
}
function Read-MsiProperty([string]$path, [string]$name) {
    $value = Read-MsiColumn $path "SELECT ``Value`` FROM ``Property`` WHERE ``Property`` = '$name'"
    if (-not $value) { throw "Missing MSI property: $name" }
    $value
}
function New-FailingMsi([string]$source, [string]$destination) {
    # Only this disposable copy is edited. The releasable MSI has no failure
    # switch or custom action. Type 19 fails after the nested old uninstalls.
    Copy-Item $source $destination
    $remove = [int](Read-MsiColumn $destination "SELECT ``Sequence`` FROM ``InstallExecuteSequence`` WHERE ``Action`` = 'RemoveExistingProducts'")
    $initialize = [int](Read-MsiColumn $destination "SELECT ``Sequence`` FROM ``InstallExecuteSequence`` WHERE ``Action`` = 'InstallInitialize'")
    $files = [int](Read-MsiColumn $destination "SELECT ``Sequence`` FROM ``InstallExecuteSequence`` WHERE ``Action`` = 'InstallFiles'")
    if ($remove -le $initialize -or $remove + 1 -ge $files) { throw 'Old products must be removed inside the transaction, before installing files' }
    $tables = @(Read-MsiColumn $destination 'SELECT `Name` FROM `_Tables`')
    $database = $installer.OpenDatabase($destination, 1)
    try {
        $queries = @()
        if ($tables -notcontains 'CustomAction') {
            $queries += 'CREATE TABLE `CustomAction` (`Action` CHAR(72) NOT NULL, `Type` SHORT NOT NULL, `Source` CHAR(72), `Target` CHAR(0) LOCALIZABLE PRIMARY KEY `Action`)'
        }
        $queries += "INSERT INTO ``CustomAction`` (``Action``, ``Type``, ``Target``) VALUES ('CuePoolRehearsalFailure', 19, 'CuePool rehearsal failure after removing [WIX_UPGRADE_DETECTED]')"
        $queries += "INSERT INTO ``InstallExecuteSequence`` (``Action``, ``Condition``, ``Sequence``) VALUES ('CuePoolRehearsalFailure', 'NOT Installed', $($remove + 1))"
        foreach ($query in $queries) {
            $view = $database.OpenView($query)
            try { [void]$view.Execute() } finally {
                [void]$view.Close()
                [Runtime.InteropServices.Marshal]::FinalReleaseComObject($view) | Out-Null
            }
        }
        # Distinguish the modified test package in Windows Installer's cache.
        $summary = $database.SummaryInformation(1)
        $summary.GetType().InvokeMember('Property', [Reflection.BindingFlags]::SetProperty, $null, $summary,
            @(9, ('{' + [guid]::NewGuid().ToString().ToUpperInvariant() + '}'))) | Out-Null
        [void]$summary.Persist()
        [Runtime.InteropServices.Marshal]::FinalReleaseComObject($summary) | Out-Null
        [void]$database.Commit()
    } finally {
        [Runtime.InteropServices.Marshal]::FinalReleaseComObject($database) | Out-Null
    }
}
function Assert-Registrations([string[]]$expected) {
    $actual = @(Get-CuePoolRegistrations | Select-Object -ExpandProperty PSChildName)
    if ($actual.Count -ne $expected.Count -or @($actual | Where-Object { $_ -notin $expected }).Count) {
        throw "Unexpected CuePool registrations: $($actual -join ', '); expected $($expected -join ', ')"
    }
}
function Assert-DirectoryHashes([string]$directory, [hashtable]$expected) {
    foreach ($name in $expected.Keys) {
        if ((Get-FileHash (Join-Path $directory $name)).Hash -ne $expected[$name]) {
            throw "Rollback changed the pre-upgrade payload: $name"
        }
    }
}
function Assert-RollbackLog([string]$log, [string[]]$codes) {
    # Exit 1603 alone could be a launch-condition failure before any removal.
    # Require completed removal followed by the deliberate failure, and proof
    # that every predecessor was discovered by this installation.
    if ($log -notmatch '(?s)Action ended [^\r\n]*RemoveExistingProducts\. Return value 1\..*CuePool rehearsal failure after removing') {
        throw 'The rollback probe did not reach the intended post-removal failure'
    }
    $message = [regex]::Match($log, 'CuePool rehearsal failure after removing ([^\r\n]+)').Groups[1].Value
    foreach ($code in $codes) {
        if (-not $message.Contains($code)) { throw "Rollback probe did not discover $code" }
    }
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
$expectedIdentity = [ordered]@{ ProductVersion = $Version; ProductName = 'CuePool'; ALLUSERS = '1'; Manufacturer = 'BlueJayLouche' }
$actualIdentity = [ordered]@{}
foreach ($property in $expectedIdentity.Keys) { $actualIdentity[$property] = Read-MsiProperty $Msi $property }
$actualIdentity | ConvertTo-Json | Set-Content (Join-Path $LogDir 'msi-identity.json')
foreach ($property in $expectedIdentity.Keys) {
    if ($actualIdentity[$property] -ne $expectedIdentity[$property]) {
        throw "MSI $property is $($actualIdentity[$property] | ConvertTo-Json -Compress); expected '$($expectedIdentity[$property])'"
    }
}
# Six same-version products reproduce accumulated legacy MSI registrations.
# All share component GUIDs; each package gets a distinct ProductCode. This is
# an installer-mechanics fixture, not an older application's runtime binary.
$previousDir = Join-Path $work 'previous'
New-Item -ItemType Directory $previousDir | Out-Null
Copy-Item "$SourceDir\*" $previousDir
Set-Content (Join-Path $previousDir 'obsolete-release-file.txt') 'removed by upgrade'
$previousMsis = @()
$previousCodes = @()
$components = @()
foreach ($number in 1..6) {
    $previousMsi = Join-Path $work "previous-$number.msi"
    & "$PSScriptRoot\make-msi.ps1" -Name CuePool -Version 0.1.0 -SourceDir $previousDir -Exe cuepool.exe -FileExt qproj -Out $previousMsi
    $code = Read-MsiProperty $previousMsi 'ProductCode'
    $currentComponents = @(Read-MsiColumn $previousMsi 'SELECT `ComponentId` FROM `Component`' | Sort-Object)
    if ((Read-MsiProperty $previousMsi 'UpgradeCode') -ne $upgradeCode -or
        $code -eq $productCode -or $code -in $previousCodes -or
        ($number -gt 1 -and @(Compare-Object $components $currentComponents).Count)) {
        throw 'Legacy fixtures must have distinct ProductCodes and identical UpgradeCode/component identity'
    }
    $components = $currentComponents
    $previousMsis += $previousMsi
    $previousCodes += $code
}
$results.previous_product_codes = $previousCodes
$failingMsi = Join-Path $work 'candidate-with-test-failure.msi'
New-FailingMsi $Msi $failingMsi
try {
    $portable = Join-Path $work 'portable'
    Expand-Archive $Zip $portable
    Assert-Payload $portable
    foreach ($dll in 'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll', 'avcodec-62.dll', 'avformat-62.dll', 'avutil-60.dll') {
        if (-not (Test-Path (Join-Path $portable $dll))) { throw "Missing app-local runtime $dll" }
    }
    Test-Runtime $portable 'zip-runtime'
    foreach ($index in 0..5) { Invoke-Msi /i $previousMsis[$index] "install-previous-$($index + 1)" }
    Assert-Registrations $previousCodes
    # Mimic a file-copy promotion performed after the legacy MSI installs.
    Set-Content (Join-Path $installDir 'cuepool.exe') 'manually promoted executable fixture'
    $legacyHashes = @{}
    foreach ($file in Get-ChildItem $installDir -File) { $legacyHashes[$file.Name] = (Get-FileHash $file.FullName).Hash }
    $backupDir = Join-Path $work 'production-file-copy-backup'
    Copy-Item $installDir $backupDir -Recurse
    Invoke-Msi /i $failingMsi 'forced-upgrade-failure' 1603
    Assert-RollbackLog (Get-Content (Join-Path $LogDir 'forced-upgrade-failure.log') -Raw) $previousCodes
    Assert-Registrations $previousCodes
    Assert-DirectoryHashes $installDir $legacyHashes
    Assert-DirectoryHashes $backupDir $legacyHashes
    Assert-DataPreserved
    $results.checks += 'transaction-restored-six-products-payload-and-data'
    Invoke-Msi /i $Msi 'upgrade-six-products-to-candidate'
    Assert-Registrations @($productCode)
    if (Test-Path (Join-Path $installDir 'obsolete-release-file.txt')) {
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
    Invoke-Msi /i $previousMsis[0] 'reject-downgrade' 1603
    Assert-Payload $installDir
    Invoke-Msi /x $Msi 'uninstall-candidate'
    if ((Test-Path "$installDir\cuepool.exe") -or (Test-Path "$uninstallRoot\$productCode") -or (Test-Path $shortcut)) {
        throw 'Uninstall left the executable, registration or shortcut installed'
    }
    Assert-DataPreserved
    Assert-Registrations @()
    Invoke-Msi /i $previousMsis[0] 'manual-rollback-reinstall-previous'
    Assert-Registrations @($previousCodes[0])
    if (-not (Test-Path "$installDir\obsolete-release-file.txt")) { throw 'Previous package was not restored' }
    Assert-DataPreserved
    Invoke-Msi /x $previousMsis[0] 'remove-previous'
    Invoke-Msi /i $Msi 'clean-install-candidate'
    Assert-Payload $installDir
    Test-Runtime $installDir 'clean-msi-runtime'
    Invoke-Msi /x $Msi 'clean-uninstall-candidate'
    Assert-DataPreserved
    Assert-Registrations @()
    Assert-DirectoryHashes $backupDir $legacyHashes
    if ((Get-FileHash $Msi).Hash -ne $results.msi_sha256) { throw 'Rehearsal modified the releasable MSI' }
    $results.result = 'passed'
} finally {
    foreach ($code in @($productCode) + $previousCodes) {
        if (Test-Path "$uninstallRoot\$code") { Invoke-Msi /x $code 'cleanup' }
    }
    $results | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $LogDir 'result.json')
    Remove-Item $profileDir, $work -Recurse -Force
}
Write-Output "Windows package rehearsal passed; evidence: $LogDir"
