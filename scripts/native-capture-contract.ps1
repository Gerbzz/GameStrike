# Pure metadata checks. Loading this file never starts a process or opens a device/network connection.
function Assert-NativeCaptureCase {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]$Metadata,
        [Parameter(Mandatory = $true)][hashtable]$Case,
        [Parameter(Mandatory = $true)][string]$ExpectedSourceSha
    )
    $requestedTime = [double]::Parse($Case.Time, [Globalization.CultureInfo]::InvariantCulture)
    $skip = $Case.Extra -contains '--skip-boot'
    $reduced = $Case.Extra -contains '--reduced-motion'
    $expectedPose = if ($skip -or $reduced) { 5.5 } else { $requestedTime }
    $scale = [double]$Metadata.pixels_per_point
    foreach ($number in @($scale, [double]$Metadata.width, [double]$Metadata.height,
        [double]$Metadata.boot_time, [double]$Metadata.boot_pose_seconds)) {
        if ([double]::IsNaN($number) -or [double]::IsInfinity($number)) {
            throw 'Native evidence contains a nonfinite dimension, DPI or time.'
        }
    }
    if ($Metadata.skip_boot -isnot [bool] -or $Metadata.reduced_motion -isnot [bool]) {
        throw 'Native evidence flags must be JSON booleans.'
    }
    if ([double]::IsNaN($scale) -or [double]::IsInfinity($scale) -or $scale -le 0 -or
        [Math]::Abs([double]$Metadata.width - [int]$Case.Width * $scale) -gt 2 -or
        [Math]::Abs([double]$Metadata.height - [int]$Case.Height * $scale) -gt 2 -or
        $Metadata.requested_logical_size[0] -ne [int]$Case.Width -or
        $Metadata.requested_logical_size[1] -ne [int]$Case.Height -or
        [Math]::Abs([double]$Metadata.boot_time - $requestedTime) -gt 0.0001 -or
        [Math]::Abs([double]$Metadata.boot_pose_seconds - $expectedPose) -gt 0.0001 -or
        $Metadata.skip_boot -ne $skip -or $Metadata.reduced_motion -ne $reduced) {
        throw "Native capture size/pose/flags do not match the requested case $($Case.Name)."
    }
    if ($Metadata.source_sha -cne $ExpectedSourceSha -or
        $Metadata.kind -cne 'native Rust eframe/wgpu window framebuffer' -or
        $Metadata.library -cne 'owned synthetic fixture; no real catalog or devices' -or
        $Metadata.renderer -notmatch '(?i)wgpu' -or $Metadata.game_count -lt 8 -or
        $Metadata.width -lt 880 -or $Metadata.height -lt 620) {
        throw "Native evidence identity/content contract failed for $($Case.Name)."
    }
}
