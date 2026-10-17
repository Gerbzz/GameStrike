$ErrorActionPreference = 'Stop'
$probeRoot = Join-Path $env:RUNNER_TEMP 'gamestrike-library-visual'
$library = Join-Path $probeRoot 'library'
$capture = Join-Path $probeRoot 'launcher.png'
$fixture = Join-Path $probeRoot 'Studio Sample.html'
$stdout = Join-Path $probeRoot 'launcher.out.txt'
$stderr = Join-Path $probeRoot 'launcher.err.txt'
New-Item -ItemType Directory -Force -Path $library | Out-Null
[IO.File]::WriteAllText($fixture, '<!doctype html><html><head><title>Studio Sample</title></head><body><h1>Studio Sample</h1></body></html>')
$cli = Join-Path $PWD 'target/debug/gamestrike.exe'
& $cli --library $library import $fixture
if ($LASTEXITCODE -ne 0) { throw 'Synthetic library import failed.' }
$launcher = Join-Path $PWD 'target/debug/gamestrike-launcher.exe'
$process = Start-Process -FilePath $launcher -ArgumentList @('--library', $library, '--capture-preview', $capture) -PassThru -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr
try {
    $complete = $false
    for ($attempt = 0; $attempt -lt 45; $attempt++) {
        Start-Sleep -Seconds 1
        if (Test-Path -LiteralPath $capture) {
            $probe = [IO.File]::ReadAllBytes($capture)
            if ($probe.Length -gt 1000) {
                $trailer = [byte[]](73, 69, 78, 68, 174, 66, 96, 130)
                $complete = $true
                for ($i = 0; $i -lt 8; $i++) {
                    if ($probe[$probe.Length - 8 + $i] -ne $trailer[$i]) { $complete = $false; break }
                }
                if ($complete) { break }
            }
        }
        $process.Refresh()
        if ($process.HasExited) { break }
    }
    if (-not $complete) {
        Write-Host 'Launcher stdout:'; Get-Content -LiteralPath $stdout -ErrorAction SilentlyContinue
        Write-Host 'Launcher stderr:'; Get-Content -LiteralPath $stderr -ErrorAction SilentlyContinue
        throw 'Hosted launcher did not produce a complete PNG within 45 seconds.'
    }
} finally {
    $process.Refresh()
    if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
}
Add-Type -AssemblyName System.Drawing
$bitmap = [System.Drawing.Bitmap]::new($capture)
try {
    if ($bitmap.Width -lt 880 -or $bitmap.Height -lt 620) { throw 'Launcher capture dimensions are too small.' }
    $colors = [System.Collections.Generic.HashSet[int]]::new()
    $dark = 0
    $orange = 0
    for ($y = 0; $y -lt $bitmap.Height; $y += 8) {
        for ($x = 0; $x -lt $bitmap.Width; $x += 8) {
            $pixel = $bitmap.GetPixel($x, $y)
            [void]$colors.Add($pixel.ToArgb())
            if ($pixel.R -lt 70 -and $pixel.G -lt 70 -and $pixel.B -lt 70) { $dark++ }
            if ($pixel.R -gt 180 -and $pixel.G -gt 70 -and $pixel.G -lt 190 -and $pixel.B -lt 130) { $orange++ }
        }
    }
    if ($colors.Count -lt 24 -or $dark -lt 200 -or $orange -lt 8) {
        throw "Launcher capture lacks expected app content: colors=$($colors.Count), dark=$dark, orange=$orange"
    }
    Write-Host "CAPTURE_DIMENSIONS:$($bitmap.Width)x$($bitmap.Height)"
    Write-Host "CAPTURE_PIXELS:colors=$($colors.Count),dark=$dark,orange=$orange"
} finally {
    $bitmap.Dispose()
}
$bytes = [IO.File]::ReadAllBytes($capture)
Write-Host "CAPTURE_SHA256:$((Get-FileHash -LiteralPath $capture -Algorithm SHA256).Hash)"
Write-Host "CAPTURE_BYTES:$($bytes.Length)"
$encoded = [Convert]::ToBase64String($bytes)
for ($offset = 0; $offset -lt $encoded.Length; $offset += 4000) {
    Write-Host ("CAPTURE_CHUNK:" + $encoded.Substring($offset, [Math]::Min(4000, $encoded.Length - $offset)))
}
