# Hosted acceptance only. This script is source-prepared, not permission to execute a Room build.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [string]$ExpectedSourceSha = $env:GITHUB_SHA
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($env:GITHUB_ACTIONS -cne 'true' -or -not $env:GITHUB_WORKSPACE) {
    throw 'Native acceptance is restricted to an explicitly authorized GitHub-hosted job.'
}
if ($ExpectedSourceSha -notmatch '^[0-9a-fA-F]{40}$') {
    throw 'An exact expected source SHA is required.'
}
. (Join-Path $PSScriptRoot 'native-capture-contract.ps1')

$workspacePath = [IO.Path]::GetFullPath($env:GITHUB_WORKSPACE).TrimEnd([IO.Path]::DirectorySeparatorChar)
$executablePath = (Resolve-Path -LiteralPath $Executable).Path
$buildDirectory = [IO.Path]::GetFullPath((Join-Path $workspacePath 'target/release')) + [IO.Path]::DirectorySeparatorChar
if (-not $executablePath.StartsWith($buildDirectory, [StringComparison]::OrdinalIgnoreCase) -or
    [IO.Path]::GetFileName($executablePath) -cne 'gamestrike-desktop.exe') {
    throw 'Capture requires the newly compiled native desktop artifact under this checkout target/release.'
}
$evidencePath = [IO.Path]::GetFullPath($OutputDirectory)
if (-not $evidencePath.StartsWith($workspacePath + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Evidence must remain inside the hosted workspace.'
}
if (Test-Path -LiteralPath $evidencePath) { throw 'Evidence directory exists; previous results will not be overwritten.' }
New-Item -ItemType Directory -Path $evidencePath | Out-Null

$cases = @(
    @{ Name = 'library'; Time = '5.5'; Width = '1250'; Height = '800'; Extra = @() },
    @{ Name = 'boot-0.5'; Time = '0.5'; Width = '1250'; Height = '800'; Extra = @() },
    @{ Name = 'boot-2'; Time = '2'; Width = '1250'; Height = '800'; Extra = @() },
    @{ Name = 'boot-3.5'; Time = '3.5'; Width = '1250'; Height = '800'; Extra = @() },
    @{ Name = 'boot-4.5'; Time = '4.5'; Width = '1250'; Height = '800'; Extra = @() },
    @{ Name = 'boot-5.5'; Time = '5.5'; Width = '1250'; Height = '800'; Extra = @() },
    @{ Name = 'skip'; Time = '0.5'; Width = '1250'; Height = '800'; Extra = @('--skip-boot') },
    @{ Name = 'reduced-motion'; Time = '0.5'; Width = '1250'; Height = '800'; Extra = @('--reduced-motion') },
    @{ Name = 'library-target'; Time = '5.5'; Width = '1672'; Height = '941'; Extra = @() }
)

foreach ($case in $cases) {
    $pngPath = Join-Path $evidencePath ($case.Name + '.png')
    $arguments = @('--synthetic-library', '--capture-native', $pngPath, '--boot-time', $case.Time,
        '--width', $case.Width, '--height', $case.Height) + $case.Extra
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $executablePath
    $startInfo.WorkingDirectory = $workspacePath
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true # No console helper; the app creates its real native GUI window.
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in $arguments) { $startInfo.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) { throw "Native process did not start for $($case.Name)." }
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(30000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw "Native process timed out for $($case.Name)."
        }
        [IO.File]::WriteAllText((Join-Path $evidencePath ($case.Name + '.stdout.log')), $stdout.GetAwaiter().GetResult())
        [IO.File]::WriteAllText((Join-Path $evidencePath ($case.Name + '.stderr.log')), $stderr.GetAwaiter().GetResult())
        if ($process.ExitCode -ne 0) { throw "Native process failed for $($case.Name), exit $($process.ExitCode)." }
    } finally { $process.Dispose() }

    $jsonPath = [IO.Path]::ChangeExtension($pngPath, 'json')
    if (-not (Test-Path -LiteralPath $pngPath -PathType Leaf) -or
        -not (Test-Path -LiteralPath $jsonPath -PathType Leaf)) {
        throw "Native process exited without both framebuffer and metadata for $($case.Name)."
    }
    $metadata = Get-Content -LiteralPath $jsonPath -Raw | ConvertFrom-Json
    Assert-NativeCaptureCase -Metadata $metadata -Case $case -ExpectedSourceSha $ExpectedSourceSha
    $bytes = [IO.File]::ReadAllBytes($pngPath)
    if ($bytes.Length -lt 33 -or [Convert]::ToHexString($bytes[0..7]) -cne '89504E470D0A1A0A') {
        throw "Invalid PNG for $($case.Name)."
    }
    $width = ([uint32]$bytes[16] -shl 24) -bor ([uint32]$bytes[17] -shl 16) -bor
        ([uint32]$bytes[18] -shl 8) -bor [uint32]$bytes[19]
    $height = ([uint32]$bytes[20] -shl 24) -bor ([uint32]$bytes[21] -shl 16) -bor
        ([uint32]$bytes[22] -shl 8) -bor [uint32]$bytes[23]
    if ($width -ne $metadata.width -or $height -ne $metadata.height) {
        throw "PNG dimensions disagree with native metadata for $($case.Name)."
    }
}

[ordered]@{
    source_sha = $ExpectedSourceSha
    executable_sha256 = (Get-FileHash -LiteralPath $executablePath -Algorithm SHA256).Hash.ToLowerInvariant()
    evidence = 'compiled native Rust GUI; hosted runner, owned synthetic library'
    room_acceptance = 'not established'
    captures = @($cases | ForEach-Object { $_.Name })
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $evidencePath 'native-build.json') -Encoding utf8
