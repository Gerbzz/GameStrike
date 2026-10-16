$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.Windows.Forms
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class NativePicker {
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr dialog, int id);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
}
'@

$probeRoot = Join-Path $env:RUNNER_TEMP 'gamestrike-interaction-probe'
$library = Join-Path $probeRoot 'library'
$zipPath = Join-Path $probeRoot 'Wrapped Probe.zip'
New-Item -ItemType Directory -Force -Path $library | Out-Null
$fixtureZip = [IO.Compression.ZipFile]::Open($zipPath, [IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($item in @(
        @{ Name = 'Wrapped Probe/index.html'; Body = '<!doctype html><title>Wrapped Probe</title><p id="status">Waiting for relative script</p><script defer src="assets/save.js"></script>' },
        @{ Name = 'Wrapped Probe/assets/save.js'; Body = 'const count = Number(localStorage.getItem("probe-runs") || "0") + 1; localStorage.setItem("probe-runs", String(count)); document.getElementById("status").textContent = "Relative asset loaded; saved run " + count;' }
    )) {
        $entry = $fixtureZip.CreateEntry($item.Name)
        $writer = [IO.StreamWriter]::new($entry.Open(), [Text.UTF8Encoding]::new($false))
        try { $writer.Write($item.Body) } finally { $writer.Dispose() }
    }
}
finally {
    $fixtureZip.Dispose()
}
$fixtureZip = [IO.Compression.ZipFile]::OpenRead($zipPath)
try {
    foreach ($entry in $fixtureZip.Entries) {
        $codes = ($entry.FullName.ToCharArray() | ForEach-Object { [int][char]$_ }) -join ','
        Write-Host ("Fixture ZIP member: [" + $entry.FullName + "] codes=" + $codes)
    }
}
finally {
    $fixtureZip.Dispose()
}

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
    $pattern = $null
    if ($element.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
        $pattern.Invoke()
    }
    else {
        $element.SetFocus()
        [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
    }
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

    $dialog = Wait-Named $launcherWindow 'Add a local HTML game' 25 'Window'
    Write-Host 'Picker UI Automation tree:'
    Show-Tree $dialog
    $fileName = Find-Named $dialog 'File name' 'Edit'
    if ($null -eq $fileName) { throw 'Picker file-name control was not exposed to UI Automation.' }
    $fileName.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($zipPath)
    Write-Host ("Picker file name: " + $fileName.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value)
    $fileName.SetFocus()
    [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
    Start-Sleep -Seconds 1
    if ($null -ne (Find-Named $launcherWindow 'Add a local HTML game' 'Window')) {
        $openButton = [NativePicker]::GetDlgItem([IntPtr]$dialog.Current.NativeWindowHandle, 1)
        if ($openButton -eq [IntPtr]::Zero) { throw 'Native picker Open control was unavailable.' }
        [void][NativePicker]::SendMessage($openButton, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
        Write-Host 'Clicked native picker Open control.'
    }
    Write-Host 'Native picker path submitted.'

    try {
        $card = Wait-Named $launcherWindow 'Wrapped Probe' 30 'Button'
    }
    catch {
        Write-Host 'Launcher UI Automation tree after picker:'
        Show-Tree $launcherWindow
        Write-Host 'Is picker still open:'
        Write-Host ($null -ne (Find-Named $launcherWindow 'Add a local HTML game' 'Window'))
        Write-Host 'Isolated library games:'
        Get-ChildItem (Join-Path $library 'games') -ErrorAction SilentlyContinue | Select-Object Name, Length
        Write-Host 'Launcher stdout:'
        Get-Content $launcherOut -ErrorAction SilentlyContinue
        Write-Host 'Launcher stderr:'
        Get-Content $launcherErr -ErrorAction SilentlyContinue
        throw
    }
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
