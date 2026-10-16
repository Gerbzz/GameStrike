$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.IO.Compression.FileSystem

$probeRoot = Join-Path $env:RUNNER_TEMP 'gamestrike-interaction-probe'
$library = Join-Path $probeRoot 'library'
$fixtureRoot = Join-Path $probeRoot 'source'
$gameRoot = Join-Path $fixtureRoot 'Wrapped Probe'
$assets = Join-Path $gameRoot 'assets'
$zipPath = Join-Path $probeRoot 'Wrapped Probe.zip'
New-Item -ItemType Directory -Force -Path $library, $assets | Out-Null
[IO.File]::WriteAllText((Join-Path $gameRoot 'index.html'), '<!doctype html><title>Wrapped Probe</title><p id="status">Waiting for relative script</p><script defer src="assets/save.js"></script>')
[IO.File]::WriteAllText((Join-Path $assets 'save.js'), 'const count = Number(localStorage.getItem("probe-runs") || "0") + 1; localStorage.setItem("probe-runs", String(count)); document.getElementById("status").textContent = "Relative asset loaded; saved run " + count;')
[IO.Compression.ZipFile]::CreateFromDirectory($fixtureRoot, $zipPath)

function Find-Window([int]$processId) {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    foreach ($element in $root.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)) {
        try {
            if ($element.Current.ProcessId -eq $processId) { return $element }
        } catch {}
    }
    return $null
}

function Find-Named($window, [string]$text, [string]$kind = '') {
    if ($null -eq $window) { return $null }
    foreach ($element in $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
        try {
            if ($element.Current.Name.Contains($text) -and
                ($kind -eq '' -or $element.Current.ControlType.ProgrammaticName -eq "ControlType.$kind")) {
                return $element
            }
        } catch {}
    }
    return $null
}

function Wait-Named($window, [string]$text, [int]$seconds = 25, [string]$kind = '') {
    for ($attempt = 0; $attempt -lt $seconds * 2; $attempt++) {
        $element = Find-Named $window $text $kind
        if ($null -ne $element) { return $element }
        Start-Sleep -Milliseconds 500
    }
    throw "UI Automation did not find '$text'."
}

function Wait-Window([int]$processId, [int]$seconds = 25) {
    for ($attempt = 0; $attempt -lt $seconds * 2; $attempt++) {
        $window = Find-Window $processId
        if ($null -ne $window) { return $window }
        Start-Sleep -Milliseconds 500
    }
    throw "UI Automation did not find a window for process $processId."
}

function Invoke-Element($element) {
    $element.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
}

function Show-Tree($window) {
    if ($null -eq $window) { return }
    $count = 0
    foreach ($element in $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
        try {
            $name = $element.Current.Name
            if ($name) {
                Write-Host ("UIA: " + $element.Current.ControlType.ProgrammaticName + " | " + $name)
                $count++
                if ($count -ge 80) { break }
            }
        } catch {}
    }
}

$launcher = Join-Path $PWD 'target/debug/gamestrike-launcher.exe'
$launcherOut = Join-Path $probeRoot 'launcher.out.txt'
$launcherErr = Join-Path $probeRoot 'launcher.err.txt'
$launcherProcess = Start-Process -FilePath $launcher -ArgumentList @('--library', $library) -PassThru -WindowStyle Normal -RedirectStandardOutput $launcherOut -RedirectStandardError $launcherErr
try {
    $launcherWindow = Wait-Window $launcherProcess.Id
    Write-Host 'Launcher UI Automation tree:'
    Show-Tree $launcherWindow
    Invoke-Element (Wait-Named $launcherWindow 'Add a game' 25 'Button')

    $dialog = $null
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        $root = [System.Windows.Automation.AutomationElement]::RootElement
        foreach ($element in $root.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)) {
            try {
                if ($element.Current.ProcessId -eq $launcherProcess.Id -and $element.Current.ClassName -eq '#32770') {
                    $dialog = $element
                    break
                }
            } catch {}
        }
        if ($null -ne $dialog) { break }
        Start-Sleep -Milliseconds 500
    }
    if ($null -eq $dialog) {
        Write-Host 'Launcher UI Automation tree after Add game:'
        Show-Tree $launcherWindow
        Write-Host 'Other top-level windows owned by launcher:'
        $root = [System.Windows.Automation.AutomationElement]::RootElement
        foreach ($element in $root.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)) {
            try {
                if ($element.Current.ProcessId -eq $launcherProcess.Id) {
                    Write-Host ("Window: " + $element.Current.Name + " | " + $element.Current.ClassName)
                }
            } catch {}
        }
        Write-Host 'Launcher stdout:'
        Get-Content $launcherOut -ErrorAction SilentlyContinue
        Write-Host 'Launcher stderr:'
        Get-Content $launcherErr -ErrorAction SilentlyContinue
        throw 'Native file picker did not appear.'
    }
    Write-Host 'Picker UI Automation tree:'
    Show-Tree $dialog
    $fileName = Find-Named $dialog 'File name' 'Edit'
    if ($null -eq $fileName) { throw 'Picker file-name control was not exposed to UI Automation.' }
    $fileName.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($zipPath)
    Invoke-Element (Wait-Named $dialog 'Open' 25 'Button')
    Write-Host 'Native picker submitted the wrapped ZIP.'

    $card = Wait-Named $launcherWindow 'Wrapped Probe' 30 'Button'
    for ($run = 1; $run -le 2; $run++) {
        Invoke-Element $card
        Invoke-Element (Wait-Named $launcherWindow 'Allow network & launch' 25 'Button')
        $playerProcess = $null
        for ($attempt = 0; $attempt -lt 50; $attempt++) {
            $playerProcess = Get-Process -Name 'gamestrike-player' -ErrorAction SilentlyContinue | Select-Object -First 1
            if ($null -ne $playerProcess) { break }
            Start-Sleep -Milliseconds 500
        }
        if ($null -eq $playerProcess) { throw "Player process did not start on run $run." }
        $playerWindow = Wait-Window $playerProcess.Id
        Write-Host "Player UI Automation tree on run $run :"
        Show-Tree $playerWindow
        Wait-Named $playerWindow "Relative asset loaded; saved run $run" 25 | Out-Null
        Write-Host "Player run $run loaded the relative asset and displayed saved run $run."
        if (-not $playerProcess.CloseMainWindow()) { throw "Player window did not accept close on run $run." }
        if (-not $playerProcess.WaitForExit(10000)) { throw "Player did not exit on run $run." }
        Wait-Named $launcherWindow 'Player closed. Your library is ready.' 20 | Out-Null
        $card = Wait-Named $launcherWindow 'Wrapped Probe' 20 'Button'
    }
    Write-Host 'GUI picker, player return, and persisted save round trip passed.'
}
finally {
    Get-Process -Name 'gamestrike-player' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    if (-not $launcherProcess.HasExited) { Stop-Process -Id $launcherProcess.Id -Force }
}
