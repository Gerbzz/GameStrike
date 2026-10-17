# Captures the real launcher's WebView2 surface for each view, using clearly labeled synthetic
# games imported through the CLI. Output: $env:RUNNER_TEMP/gamestrike-library-visual/*.png
$ErrorActionPreference = 'Stop'

# The current workflow file only calls this script after building. Until the proposed workflow in
# docs/ci/windows-source.proposed.yml is installed (it needs a token with the `workflow` scope),
# run the extra checks here so every PR still exercises them.
if ($env:GITHUB_ACTIONS -eq 'true') {
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
$cli = Join-Path $PWD 'target/debug/gamestrike.exe'
$launcher = Join-Path $PWD 'target/debug/gamestrike-launcher.exe'

$titles = @(
    'Synthetic Test - Orbit Garden',
    'Synthetic Test - Pixel Harbor',
    'Synthetic Test - Lantern Maze',
    'Synthetic Test - Comet Courier',
    'Synthetic Test - Tiny Tactics',
    'Synthetic Test - A Deliberately Long Title For Layout Checks In The Library'
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
Capture-Launcher $library 'minimum' @('--window-size', '880', '620')
$empty = Join-Path $probeRoot 'empty-library'
New-Item -ItemType Directory -Force -Path $empty | Out-Null
Capture-Launcher $empty 'empty' @()
