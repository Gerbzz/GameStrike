# Saves a PNG of one running process's main window, cropped to that window (title bar included).
# Used for README screenshots of the real running build: .\scripts\capture-window.ps1 -ProcessId 1234 -Out shot.png
param([Parameter(Mandatory)] [int]$ProcessId, [Parameter(Mandatory)] [string]$Out)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class GsWin {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
}
'@
[GsWin]::SetProcessDPIAware() | Out-Null
$process = Get-Process -Id $ProcessId
$handle = $process.MainWindowHandle
if ($handle -eq [IntPtr]::Zero) { throw "Process $ProcessId has no main window." }
[GsWin]::SetForegroundWindow($handle) | Out-Null
Start-Sleep -Milliseconds 600
# DWMWA_EXTENDED_FRAME_BOUNDS (9) excludes the invisible resize border.
$rect = New-Object GsWin+RECT
[void][GsWin]::DwmGetWindowAttribute($handle, 9, [ref]$rect, 16)
$width = $rect.Right - $rect.Left; $height = $rect.Bottom - $rect.Top
$bitmap = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
$bitmap.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose(); $bitmap.Dispose()
"$Out ${width}x${height}"
