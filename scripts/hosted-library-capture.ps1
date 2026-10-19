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

# Real library data for the synthetic games: sources, favorites, a custom cover, and a ZIP whose own
# icon becomes its automatic cover. Every image is generated here; none is third-party art.
function Id-Of([string]$Title) {
    $line = (& $cli --library $library list) | Where-Object { $_ -like "*$Title*" } | Select-Object -First 1
    ($line -split "`t")[0]
}
function Invoke-Cli([string[]]$Arguments) {
    & $cli --library $library @Arguments
    if ($LASTEXITCODE -ne 0) { throw "gamestrike $($Arguments -join ' ') failed." }
}
Invoke-Cli @('set-source', (Id-Of 'Orbit Garden'), 'itch.io')
Invoke-Cli @('set-source', (Id-Of 'Pixel Harbor'), 'itch.io')
Invoke-Cli @('set-source', (Id-Of 'Lantern Maze'), 'Newgrounds')
Invoke-Cli @('favorite', (Id-Of 'Orbit Garden'), 'on')
Invoke-Cli @('favorite', (Id-Of 'Comet Courier'), 'on')
$coverFile = Join-Path $probeRoot 'cover.png'
node (Join-Path $PWD 'scripts/make-test-cover.cjs') $coverFile 125
Invoke-Cli @('set-cover', (Id-Of 'Pixel Harbor'), $coverFile)
$iconGame = Join-Path $probeRoot 'icon-game'
New-Item -ItemType Directory -Force -Path $iconGame | Out-Null
[IO.File]::WriteAllText((Join-Path $iconGame 'index.html'), '<!doctype html><title>Signal Tower (Synthetic Test)</title><h1>Signal Tower (Synthetic Test)</h1>')
node (Join-Path $PWD 'scripts/make-test-cover.cjs') (Join-Path $iconGame 'icon.png') 280
$iconZip = Join-Path $probeRoot 'Signal Tower (Synthetic Test).zip'
node (Join-Path $PWD 'scripts/make-test-zip.cjs') $iconZip (Join-Path $iconGame 'index.html') (Join-Path $iconGame 'icon.png')
Invoke-Cli @('import', $iconZip)
$listing = & $cli --library $library list
$listing | Set-Content -Encoding utf8 (Join-Path $probeRoot 'library-list.txt')
if (-not ($listing | Where-Object { $_ -like '*Signal Tower*' -and $_ -like '*cover: auto*' })) { throw "The ZIP's icon did not become its automatic cover." }
if (-not ($listing | Where-Object { $_ -like '*Pixel Harbor*' -and $_ -like '*cover: custom*' })) { throw 'The custom cover was not stored.' }

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
        Get-Content -LiteralPath $out -ErrorAction SilentlyContinue | Where-Object { $_ -like '*scene renderer*' } | ForEach-Object { Write-Host "  $_" }
    } finally {
        $bitmap.Dispose()
    }
}

Capture-Launcher $library 'library' @()
Capture-Launcher $library 'focus' @('--capture-view', 'focus')
Capture-Launcher $library 'long-title' @('--capture-view', 'long')
Capture-Launcher $library 'network-dialog' @('--capture-view', 'launch')
Capture-Launcher $library 'manage' @('--capture-view', 'manage')
Capture-Launcher $library 'about' @('--capture-view', 'about')
Capture-Launcher $library 'favorites' @('--capture-view', 'favorites')
Capture-Launcher $library 'collapsed' @('--capture-view', 'collapsed')
Capture-Launcher $library 'search' @('--capture-view', 'search')
Capture-Launcher $library 'boot' @('--capture-view', 'boot')
Capture-Launcher $library 'minimum' @('--window-size', '880', '620')
$empty = Join-Path $probeRoot 'empty-library'
New-Item -ItemType Directory -Force -Path $empty | Out-Null
Capture-Launcher $empty 'empty' @()
