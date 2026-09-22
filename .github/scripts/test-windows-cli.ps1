# Exercise the actual release binary, including its PE subsystem and redirected CLI.
param(
    [Parameter(Mandatory)][string]$Exe,
    # CI's unbundled build still needs its configured FFmpeg SDK. Packages do not.
    [switch]$UseBuildEnvironment
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$Exe = (Resolve-Path $Exe).Path
$bytes = [IO.File]::ReadAllBytes($Exe)
if ($bytes.Length -lt 64 -or [BitConverter]::ToUInt16($bytes, 0) -ne 0x5a4d) { throw 'Not a Windows executable' }
$pe = [BitConverter]::ToInt32($bytes, 0x3c)
if ($pe -lt 0 -or $pe + 94 -gt $bytes.Length -or [BitConverter]::ToUInt32($bytes, $pe) -ne 0x4550) { throw 'Invalid PE header' }
if ([BitConverter]::ToUInt16($bytes, $pe + 24 + 68) -ne 2) {
    throw 'CuePool release executable must use the Windows GUI subsystem, so Explorer does not create a terminal.'
}
$work = Join-Path ([IO.Path]::GetTempPath()) ('cuepool-cli-' + [guid]::NewGuid())
New-Item -ItemType Directory $work | Out-Null
$oldProfile = $env:CUEPOOL_AUTOMATION_PROFILE
$oldEncoding = [Console]::OutputEncoding
$probeProfile = 'cli-' + [guid]::NewGuid().ToString('N')
$profilePath = Join-Path $env:APPDATA "CuePool/automation/$probeProfile"
try {
    $env:CUEPOOL_AUTOMATION_PROFILE = $probeProfile
    [Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $Exe
    $start.Arguments = '--version'
    $start.WorkingDirectory = $work
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.StandardOutputEncoding = New-Object Text.UTF8Encoding($false)
    if (-not $UseBuildEnvironment) {
        $start.EnvironmentVariables['PATH'] = "$env:SystemRoot\System32;$env:SystemRoot"
        foreach ($name in 'FFMPEG_DIR', 'LIB', 'LIBPATH', 'INCLUDE') { $start.EnvironmentVariables.Remove($name) }
    }
    $process = [Diagnostics.Process]::Start($start)
    if (-not $process.WaitForExit(30000)) { $process.Kill(); throw 'Redirected --version timed out' }
    $output = $process.StandardOutput.ReadToEnd()
    $errorOutput = $process.StandardError.ReadToEnd()
    if ($process.ExitCode -ne 0 -or $output -notmatch '(?m)^CuePool \d+\.\d+\.\d+' -or
        $output -notmatch '(?m)^ASIO: enabled\s*$' -or $errorOutput) {
        throw "Redirected --version failed: exit=$($process.ExitCode), stdout=$output, stderr=$errorOutput"
    }
    # Out-String reconstructs native lines using Windows CRLF; Rust emits LF.
    $expected = $output.Replace("`r`n", "`n").Trim()
    $pipeline = (& $Exe --version | Out-String)
    if ($LASTEXITCODE -ne 0 -or $pipeline.Replace("`r`n", "`n").Trim() -ne $expected) { throw 'PowerShell pipeline lost version output' }
    $file = Join-Path $work 'version.txt'
    # A bare GUI command can return before file output is complete in Windows
    # PowerShell. Wait explicitly instead of reading a still-empty file.
    $fileProcess = Start-Process $Exe -ArgumentList '--version' -Wait -PassThru -RedirectStandardOutput $file
    $fileOutput = [string](Get-Content $file -Raw -Encoding UTF8)
    if ($fileProcess.ExitCode -ne 0 -or $fileOutput.Replace("`r`n", "`n").Trim() -ne $expected) { throw 'File redirection lost version output' }
    if (Test-Path $profilePath) { throw '--version unexpectedly initialized its application profile' }
    Write-Output 'Windows GUI subsystem, redirected --version, pipeline, file output and side-effect checks passed.'
} finally {
    $env:CUEPOOL_AUTOMATION_PROFILE = $oldProfile
    [Console]::OutputEncoding = $oldEncoding
    Remove-Item $work -Recurse -Force
}
