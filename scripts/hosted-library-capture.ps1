# Captures the real launcher's WebView2 surface for each view, using clearly labeled synthetic
# games imported through the CLI. Output: $env:RUNNER_TEMP/gamestrike-library-visual/*.png
$ErrorActionPreference = 'Stop'

# The original Windows workflow only calls this script after building, so it runs the extra checks
# here. The cross-platform workflow runs them as separate steps and sets GAMESTRIKE_CHECKS_DONE.
if ($env:GITHUB_ACTIONS -eq 'true' -and $env:GAMESTRIKE_CHECKS_DONE -ne '1') {
    rustup component add clippy
    if ($LASTEXITCODE -ne 0) { throw 'Could not install clippy.' }
    cargo clippy --locked --features launcher-windows --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Clippy failed.' }
    cargo test --locked --features launcher-windows --test player_isolation -- --include-ignored --nocapture
    if ($LASTEXITCODE -ne 0) { throw 'Real player isolation test failed.' }
}

$probeRoot = Join-Path $env:RUNNER_TEMP 'gamestrike-library-visual'
$library = Join-Path $probeRoot 'library'
$fixtures = Join-Path $probeRoot 'fixtures'
New-Item -ItemType Directory -Force -Path $library, $fixtures | Out-Null
$binDir = if ($env:GAMESTRIKE_BIN_DIR) { $env:GAMESTRIKE_BIN_DIR } else { Join-Path $PWD 'target/debug' }
$cli = Join-Path $binDir 'gamestrike.exe'
$launcher = Join-Path $binDir 'gamestrike-launcher.exe'
$player = Join-Path $binDir 'gamestrike-player.exe'

# Player smoke test: the exact bundled save fixture, launched twice, keeps its counter.
$smokeLibrary = Join-Path $probeRoot 'smoke-library'
New-Item -ItemType Directory -Force -Path $smokeLibrary | Out-Null
$smokeFile = Join-Path $probeRoot 'Save Counter (Synthetic Test).html'
Copy-Item -LiteralPath (Join-Path $PWD 'fixtures/save-counter.html') -Destination $smokeFile
& $cli --library $smokeLibrary import $smokeFile
if ($LASTEXITCODE -ne 0) { throw 'Smoke fixture import failed.' }
$smokeId = ((& $cli --library $smokeLibrary list) -split "`t")[0]
foreach ($launch in 1, 2) {
    $output = & $player --library $smokeLibrary --id $smokeId --smoke
    if ($LASTEXITCODE -ne 0) { throw "Player smoke launch $launch failed." }
    $output | Set-Content -Encoding utf8 (Join-Path $probeRoot "player-smoke-$launch.txt")
    if ($output -notcontains "GameStrike fixture launch $launch") { throw "Player smoke launch $launch did not report its saved counter." }
    Write-Host "Player smoke launch $launch : $($output -join ' | ')"
}

$titles = @(
    'Orbit Garden (Synthetic Test)',
    'Pixel Harbor (Synthetic Test)',
    'Lantern Maze (Synthetic Test)',
    'Comet Courier (Synthetic Test)',
    'Tiny Tactics (Synthetic Test)',
    'A Deliberately Long Title For Layout Checks In The Library (Synthetic Test)'
)
foreach ($title in $titles) {
    $file = Join-Path $fixtures "$title.html"
    [IO.File]::WriteAllText($file, "<!doctype html><title>$title</title><h1>$title</h1><p>GameStrike synthetic test fixture.</p>")
    & $cli --library $library import $file
    if ($LASTEXITCODE -ne 0) { throw "Synthetic import failed: $title" }
}

function Capture-Launcher([string]$Library, [string]$Name, [string[]]$Extra) {
    $png = Join-Path $probeRoot "$Name.png"
    $out = Join-Path $probeRoot "$Name.out.txt"
    $err = Join-Path $probeRoot "$Name.err.txt"
    $arguments = @('--library', $Library, '--capture-preview', $png) + $Extra
    $process = Start-Process -FilePath $launcher -ArgumentList $arguments -PassThru -WindowStyle Hidden -RedirectStandardOutput $out -RedirectStandardError $err
    try {
        $complete = $false
        for ($attempt = 0; $attempt -lt 60; $attempt++) {
            Start-Sleep -Seconds 1
            $process.Refresh()
            if ($process.HasExited) {
                $complete = (Test-Path -LiteralPath $png) -and $process.ExitCode -eq 0
                break
            }
        }
        if (-not $complete) {
            Get-Content -LiteralPath $out, $err -ErrorAction SilentlyContinue
            throw "Launcher did not capture $Name."
        }
    } finally {
        $process.Refresh()
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    }
    Add-Type -AssemblyName System.Drawing
    $bitmap = [System.Drawing.Bitmap]::new($png)
    try {
        if ($bitmap.Width -lt 880 -or $bitmap.Height -lt 620) { throw "$Name capture is too small." }
        Write-Host "$Name : $($bitmap.Width)x$($bitmap.Height) sha256=$((Get-FileHash -LiteralPath $png -Algorithm SHA256).Hash)"
    } finally {
        $bitmap.Dispose()
    }
}

Capture-Launcher $library 'library' @()
Capture-Launcher $library 'long-title' @('--capture-view', 'long')
Capture-Launcher $library 'network-dialog' @('--capture-view', 'launch')
Capture-Launcher $library 'manage' @('--capture-view', 'manage')
Capture-Launcher $library 'about' @('--capture-view', 'about')
# Baseline set for the Blades, Original, and PWDS Orange styles. Later branches compare their own
# captures against these exact file names to show the styles did not change.
foreach ($pair in @(@('legacy-blades', 'library'), @('legacy-system', 'system'), @('legacy-manage', 'manage'), @('legacy-original', 'original'), @('legacy-original-games', 'original-games'), @('legacy-pwds', 'pwds'), @('legacy-boot-blades', 'boot-blades'), @('legacy-boot-original', 'boot-original'), @('legacy-boot-pwds', 'boot-pwds'))) {
    Capture-Launcher $library $pair[0] @('--capture-view', $pair[1])
}
Capture-Launcher $library 'minimum' @('--window-size', '880', '620')
$empty = Join-Path $probeRoot 'empty-library'
New-Item -ItemType Directory -Force -Path $empty | Out-Null
Capture-Launcher $empty 'empty' @()
