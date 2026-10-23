# Synthetic metadata only: no executable, Rust toolchain, GUI, provider or audio device is invoked.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'native-capture-contract.ps1')

$source = '1234567890abcdef1234567890abcdef12345678'
$case = @{ Name = 'owned-fixture'; Time = '0.5'; Width = '1250'; Height = '800'; Extra = @() }
$script:checks = 0
function New-FixtureMetadata {
    [pscustomobject]@{
        source_sha = $source; kind = 'native Rust eframe/wgpu window framebuffer'
        library = 'owned synthetic fixture; no real catalog or devices'
        renderer = 'Native Rust / egui-wgpu 0.36.2 / Dx12 / owned fixture'
        game_count = 12; width = 1250; height = 800; pixels_per_point = 1.0
        requested_logical_size = @(1250, 800); boot_time = 0.5; boot_pose_seconds = 0.5
        skip_boot = $false; reduced_motion = $false
    }
}
function Assert-Rejected([scriptblock]$Mutation, [string]$Name) {
    $metadata = New-FixtureMetadata
    & $Mutation $metadata
    $rejected = $false
    try { Assert-NativeCaptureCase -Metadata $metadata -Case $case -ExpectedSourceSha $source }
    catch { $rejected = $true }
    if (-not $rejected) { throw "Negative control accepted: $Name" }
    $script:checks++
}

Assert-NativeCaptureCase -Metadata (New-FixtureMetadata) -Case $case -ExpectedSourceSha $source
$script:checks++
$scaled = New-FixtureMetadata
$scaled.width = 1875; $scaled.height = 1200; $scaled.pixels_per_point = 1.5
Assert-NativeCaptureCase -Metadata $scaled -Case $case -ExpectedSourceSha $source
$script:checks++
$skipCase = $case.Clone(); $skipCase.Extra = @('--skip-boot')
$skipped = New-FixtureMetadata; $skipped.skip_boot = $true; $skipped.boot_pose_seconds = 5.5
Assert-NativeCaptureCase -Metadata $skipped -Case $skipCase -ExpectedSourceSha $source
$script:checks++
$reducedCase = $case.Clone(); $reducedCase.Extra = @('--reduced-motion')
$reduced = New-FixtureMetadata; $reduced.reduced_motion = $true; $reduced.boot_pose_seconds = 5.5
Assert-NativeCaptureCase -Metadata $reduced -Case $reducedCase -ExpectedSourceSha $source
$script:checks++

Assert-Rejected { param($m) $m.width = 900 } 'clamped width'
Assert-Rejected { param($m) $m.pixels_per_point = 1.5 } 'mismatched DPI'
Assert-Rejected { param($m) $m.requested_logical_size = @(880, 620) } 'wrong requested size'
Assert-Rejected { param($m) $m.boot_time = 3.5 } 'wrong requested time'
Assert-Rejected { param($m) $m.boot_pose_seconds = 5.5 } 'wrong resolved pose'
Assert-Rejected { param($m) $m.skip_boot = $true } 'wrong skip flag'
Assert-Rejected { param($m) $m.reduced_motion = $true } 'wrong reduced-motion flag'
Assert-Rejected { param($m) $m.source_sha = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' } 'wrong source SHA'
Assert-Rejected { param($m) $m.library = 'real catalog' } 'non-synthetic provenance'
Assert-Rejected { param($m) $m.boot_pose_seconds = [double]::NaN } 'NaN resolved pose'
Assert-Rejected { param($m) $m.skip_boot = 'false' } 'non-boolean skip flag'
Write-Output "Native capture metadata contracts: $script:checks PASS (synthetic metadata only; no native execution)"
