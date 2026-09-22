# Exercise the real MSI lifecycle with a tiny synthetic payload before compiling
# CuePool. This checks installer mechanics only; release runtime tests still apply.
param([string]$LogDir = 'installer-mechanics-validation')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'Run this installer rehearsal only on a disposable GitHub-hosted Windows runner.'
}
$LogDir = [IO.Path]::GetFullPath($LogDir)
New-Item -ItemType Directory -Force $LogDir | Out-Null
$notice = 'SYNTHETIC PAYLOAD: validates MSI mechanics only, not CuePool, ASIO or FFmpeg runtime behavior.'
Set-Content (Join-Path $LogDir 'SYNTHETIC-FIXTURE.txt') $notice
Write-Output $notice

# .NET Framework is installed on the hosted Windows image. Use its compiler
# directly so this fixture needs no Cargo, SDK download or package restore.
$compiler = Join-Path $env:SystemRoot 'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
if (-not (Test-Path $compiler -PathType Leaf)) { throw "Missing .NET Framework C# compiler: $compiler" }
$work = Join-Path $env:RUNNER_TEMP ('cuepool-msi-mechanics-' + [guid]::NewGuid())
$payload = Join-Path $work 'payload'
New-Item -ItemType Directory -Force $payload | Out-Null
try {
    $source = Join-Path $work 'Fixture.cs'
    @'
using System;
class Fixture {
    static int Main(string[] args) {
        if (args.Length != 1 || args[0] != "--version") return 2;
        Console.WriteLine("CuePool 0.12.3 (synthetic installer fixture)");
        Console.WriteLine("ASIO: enabled");
        Console.WriteLine("Synthetic fixture: no CuePool, ASIO or FFmpeg code is executed.");
        return 0;
    }
}
'@ | Set-Content $source
    # Match the desktop subsystem required by the shared packaged-runtime checks.
    & $compiler /nologo /target:winexe /platform:x64 "/out:$(Join-Path $payload 'cuepool.exe')" $source
    if ($LASTEXITCODE -ne 0) { throw 'Could not compile the synthetic installer fixture' }
    # Deliberately inert files fill the shared rehearsal's runtime payload slots.
    # The synthetic executable never loads these files.
    foreach ($name in 'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll', 'avcodec-62.dll', 'avformat-62.dll', 'avutil-60.dll') {
        Set-Content (Join-Path $payload $name) $notice
    }
    Set-Content (Join-Path $payload 'README.txt') $notice
    $zip = Join-Path $work 'synthetic-cuepool.zip'
    $msi = Join-Path $work 'synthetic-cuepool.msi'
    Compress-Archive -Path "$payload\*" -DestinationPath $zip
    & "$PSScriptRoot/make-msi.ps1" -Name CuePool -Version 0.12.3 -SourceDir $payload -Exe cuepool.exe -FileExt qproj -Out $msi
    & "$PSScriptRoot/test-windows-package.ps1" -Msi $msi -Zip $zip -Version 0.12.3 -SourceDir $payload -LogDir $LogDir
} finally {
    Remove-Item $work -Recurse -Force
}
